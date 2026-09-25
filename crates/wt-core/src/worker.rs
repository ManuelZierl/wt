use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use wt_runtime::{Program, QueryArena, RawDiagnostic, RuntimeLimits, SourceFile};

const MAX_PROTOCOL_BYTES: usize = 128 * 1024 * 1024;
const COMPILED_CACHE_BYTES: usize = 256 * 1024 * 1024;
pub(crate) const MEMORY_BUDGET_BYTES: usize = 1024 * 1024 * 1024;
pub(crate) const WORKER_MEMORY_RESERVATION_BYTES: usize = 256 * 1024 * 1024;
pub(crate) const PARENT_MEMORY_RESERVATION_BYTES: usize = 256 * 1024 * 1024;
const INVOCATION_TIMEOUT: Duration = Duration::from_secs(2);
const REPOSITORY_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_FILE_BYTES: usize = 64 * 1024 * 1024;

/// Deadlines the watchdog uses to kill a worker that stops responding.
///
/// Production code always uses [`WorkerTimeouts::default`]. Tests that need
/// to exercise the kill-and-recover path deterministically (without racing
/// the real production deadlines under load) can construct a [`Pool`] with
/// explicit, generous timeouts via [`Pool::with_memory_and_timeouts`].
#[derive(Clone, Copy, Debug)]
pub struct WorkerTimeouts {
    pub invocation: Duration,
    pub repository: Duration,
}

impl Default for WorkerTimeouts {
    fn default() -> Self {
        Self {
            invocation: INVOCATION_TIMEOUT,
            repository: REPOSITORY_TIMEOUT,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct MemoryProfile {
    pub worker_bytes: usize,
    pub parent_bytes: usize,
    pub total_bytes: usize,
}

impl Default for MemoryProfile {
    fn default() -> Self {
        Self {
            worker_bytes: WORKER_MEMORY_RESERVATION_BYTES,
            parent_bytes: PARENT_MEMORY_RESERVATION_BYTES,
            total_bytes: MEMORY_BUDGET_BYTES,
        }
    }
}

impl MemoryProfile {
    pub fn max_workers(self) -> Result<usize> {
        if !(WORKER_MEMORY_RESERVATION_BYTES..=512 * 1024 * 1024).contains(&self.worker_bytes)
            || !(PARENT_MEMORY_RESERVATION_BYTES..=512 * 1024 * 1024).contains(&self.parent_bytes)
            || self.total_bytes > MEMORY_BUDGET_BYTES
            || self.total_bytes < self.parent_bytes.saturating_add(self.worker_bytes)
        {
            bail!("runtime memory reservations must admit one worker under the 1 GiB ceiling");
        }
        Ok((self.total_bytes - self.parent_bytes) / self.worker_bytes)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkerRule {
    pub id: String,
    pub manifest: Value,
    pub source: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceFileWire {
    pub path: String,
    pub text: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkerRequest {
    pub rules: Vec<WorkerRule>,
    pub files: Vec<SourceFileWire>,
    pub optimized: bool,
    pub limits: RuntimeLimits,
    pub repository: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct InvocationResult {
    pub id: String,
    pub diagnostics: Vec<RawDiagnostic>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkerResult {
    pub results: Vec<InvocationResult>,
    pub stats: Value,
}

struct CachedProgram {
    program: Arc<Program>,
    residual_bytes: usize,
    pattern_reservations: Vec<(String, usize)>,
}

struct WorkerRuntime {
    compiled: HashMap<String, CachedProgram>,
    pattern_reservations: HashMap<String, (usize, usize)>,
    compiled_bytes: usize,
    compiled_count: u64,
    memory: MemoryProfile,
}

impl WorkerRuntime {
    #[cfg(test)]
    fn new() -> Self {
        Self::with_memory(MemoryProfile::default())
    }

    fn with_memory(memory: MemoryProfile) -> Self {
        Self {
            compiled: HashMap::new(),
            pattern_reservations: HashMap::new(),
            compiled_bytes: 0,
            compiled_count: 0,
            memory,
        }
    }

    fn reservation_bytes(
        &self,
        residual_bytes: usize,
        pattern_reservations: &[(String, usize)],
    ) -> usize {
        residual_bytes.saturating_add(
            pattern_reservations
                .iter()
                .filter(|(key, _)| !self.pattern_reservations.contains_key(key))
                .map(|(_, bytes)| *bytes)
                .sum::<usize>(),
        )
    }

    fn remove_cached(&mut self, digest: &str) {
        let Some(cached) = self.compiled.remove(digest) else {
            return;
        };
        self.compiled_bytes = self.compiled_bytes.saturating_sub(cached.residual_bytes);
        for (key, bytes) in cached.pattern_reservations {
            let Some((users, reserved_bytes)) = self.pattern_reservations.get_mut(&key) else {
                continue;
            };
            let last_user = *users == 1;
            if last_user {
                debug_assert_eq!(*reserved_bytes, bytes);
            } else {
                *users -= 1;
                debug_assert_eq!(*reserved_bytes, bytes);
            }
            let released_bytes = if last_user {
                Some(*reserved_bytes)
            } else {
                None
            };
            if let Some(released_bytes) = released_bytes {
                self.pattern_reservations.remove(&key);
                self.compiled_bytes = self.compiled_bytes.saturating_sub(released_bytes);
            }
        }
    }

    fn reserve_program(
        &mut self,
        residual_bytes: usize,
        pattern_reservations: &[(String, usize)],
    ) -> Result<()> {
        let required = self.reservation_bytes(residual_bytes, pattern_reservations);
        if required > COMPILED_CACHE_BYTES {
            let evictable = self
                .compiled
                .iter()
                .filter(|(_, cached)| Arc::strong_count(&cached.program) == 1)
                .map(|(digest, _)| digest.clone())
                .collect::<Vec<_>>();
            for digest in evictable {
                self.remove_cached(&digest);
                if self.reservation_bytes(residual_bytes, pattern_reservations)
                    <= COMPILED_CACHE_BYTES
                {
                    break;
                }
            }
        }
        let required = self.reservation_bytes(residual_bytes, pattern_reservations);
        if required > COMPILED_CACHE_BYTES {
            bail!(
                "compiled program cache physical budget exceeded: requires {required} bytes, budget is {COMPILED_CACHE_BYTES}"
            );
        }
        Ok(())
    }

    fn compiled_program(&mut self, rule: &WorkerRule) -> Result<Arc<Program>> {
        let manifest_bytes = serde_json::to_vec(&rule.manifest)
            .context("unable to serialize worker manifest for digest")?;
        let digest = program_digest(&manifest_bytes, rule.source.as_bytes());
        if let Some(cached) = self.compiled.get(&digest) {
            return Ok(Arc::clone(&cached.program));
        }

        let program = {
            let _watchdog = HardWatchdog::new(INVOCATION_TIMEOUT);
            Arc::new(
                wt_runtime::compile(&rule.manifest, &rule.source)
                    .with_context(|| format!("unable to compile rule {}", rule.id))?,
            )
        };
        let (residual_bytes, pattern_reservations) = program.compiled_reservation();
        self.reserve_program(residual_bytes, &pattern_reservations)?;
        self.compiled_bytes = self
            .compiled_bytes
            .saturating_add(self.reservation_bytes(residual_bytes, &pattern_reservations));
        for (key, bytes) in &pattern_reservations {
            let entry = self
                .pattern_reservations
                .entry(key.clone())
                .or_insert((0, *bytes));
            entry.0 += 1;
            debug_assert_eq!(entry.1, *bytes);
        }
        self.compiled.insert(
            digest,
            CachedProgram {
                program: Arc::clone(&program),
                residual_bytes,
                pattern_reservations,
            },
        );
        self.compiled_count += 1;
        Ok(program)
    }

    fn execute(&mut self, request: WorkerRequest) -> Result<WorkerResult> {
        if request.rules.is_empty() {
            bail!("worker request requires at least one rule");
        }
        if request.limits.max_file_bytes == 0 || request.limits.max_file_bytes > MAX_FILE_BYTES {
            bail!("worker max_file_bytes must be between 1 and {MAX_FILE_BYTES}");
        }
        let files = request
            .files
            .into_iter()
            .map(|file| SourceFile {
                path: file.path,
                text: file.text.into(),
            })
            .collect::<Vec<_>>();
        let mut arena = QueryArena::new(request.optimized);
        let limits = request.limits;
        let compilation_started = Instant::now();
        let compiled_before = self.compiled_count;
        let mut states = request
            .rules
            .into_iter()
            .map(|rule| {
                let repository =
                    rule.manifest.get("execution").and_then(Value::as_str) == Some("repository");
                let compiled = self.compiled_program(&rule);
                let error = compiled_error(&compiled);
                RuleState {
                    id: rule.id,
                    repository,
                    compiled,
                    diagnostics: Vec::new(),
                    error,
                }
            })
            .collect::<Vec<_>>();
        let compilation_elapsed_ms = compilation_started.elapsed().as_millis();

        if files.is_empty() {
            for state in &mut states {
                if state.error.is_none() {
                    state.error = Some("no authorized source files supplied".to_owned());
                }
            }
        }

        if !files.is_empty() {
            for state in states.iter_mut().filter(|state| state.repository) {
                run_state(state, &files, &mut arena, limits);
            }
            if states.iter().any(|state| state.repository) {
                arena.clear_file_results();
            }

            for file in &files {
                for state in states.iter_mut().filter(|state| !state.repository) {
                    run_state(state, std::slice::from_ref(file), &mut arena, limits);
                }
                arena.clear_file_results();
            }
        }

        let stats = serde_json::json!({
            "arena": arena.stats(),
            "worker_pid": std::process::id(),
            "compiled_cache_entries": self.compiled.len(),
            "compiled_pattern_entries": self.pattern_reservations.len(),
            "compiled_cache_bytes": self.compiled_bytes,
            "compiled_cache_budget_bytes": COMPILED_CACHE_BYTES,
            "memory_budget_bytes": self.memory.total_bytes,
            "worker_memory_reservation_bytes": self.memory.worker_bytes,
            "parent_memory_reservation_bytes": self.memory.parent_bytes,
            "compilation": {"programs_compiled":self.compiled_count - compiled_before,
                "elapsed_ms":compilation_elapsed_ms},
        });
        Ok(WorkerResult {
            results: states
                .into_iter()
                .map(|state| InvocationResult {
                    id: state.id,
                    diagnostics: state.diagnostics,
                    error: state.error,
                })
                .collect(),
            stats,
        })
    }
}

struct RuleState {
    id: String,
    repository: bool,
    compiled: Result<Arc<Program>>,
    diagnostics: Vec<RawDiagnostic>,
    error: Option<String>,
}

fn run_state(
    state: &mut RuleState,
    files: &[SourceFile],
    arena: &mut QueryArena,
    limits: RuntimeLimits,
) {
    let Ok(program) = &state.compiled else {
        if state.error.is_none() {
            state.error = state.compiled.as_ref().err().map(ToString::to_string);
        }
        return;
    };
    let _watchdog = HardWatchdog::new(if state.repository {
        REPOSITORY_TIMEOUT
    } else {
        INVOCATION_TIMEOUT
    });
    match program.execute_with_limits(files, arena, limits) {
        Ok(mut diagnostics) => state.diagnostics.append(&mut diagnostics),
        Err(error) => {
            if state.error.is_none() {
                state.error = Some(error.to_string());
            }
        }
    }
}

fn compiled_error(compiled: &Result<Arc<Program>>) -> Option<String> {
    compiled.as_ref().err().map(ToString::to_string)
}

fn program_digest(manifest: &[u8], source: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(manifest.len().to_le_bytes());
    digest.update(manifest);
    digest.update(source.len().to_le_bytes());
    digest.update(source);
    digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

struct HardWatchdog {
    cancel: Option<Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl HardWatchdog {
    fn new(timeout: Duration) -> Self {
        let (cancel, receiver) = mpsc::channel();
        let thread = thread::spawn(move || {
            if matches!(
                receiver.recv_timeout(timeout),
                Err(RecvTimeoutError::Timeout)
            ) {
                std::process::exit(2);
            }
        });
        Self {
            cancel: Some(cancel),
            thread: Some(thread),
        }
    }
}

impl Drop for HardWatchdog {
    fn drop(&mut self) {
        self.cancel.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Run the persistent worker protocol on stdin/stdout.
///
/// The worker reads one complete JSON request per line and writes one complete
/// JSON response per line. It never opens source paths; all source bytes arrive
/// in the request.
pub fn serve() -> Result<()> {
    let memory = MemoryProfile {
        worker_bytes: worker_env("WT_WORKER_MEMORY_BYTES", WORKER_MEMORY_RESERVATION_BYTES)?,
        parent_bytes: worker_env("WT_PARENT_MEMORY_BYTES", PARENT_MEMORY_RESERVATION_BYTES)?,
        total_bytes: worker_env("WT_TOTAL_MEMORY_BYTES", MEMORY_BUDGET_BYTES)?,
    };
    memory.max_workers()?;
    apply_worker_memory_limit(memory.worker_bytes)?;
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut input = BufReader::new(stdin.lock());
    let mut output = BufWriter::new(stdout.lock());
    let mut runtime = WorkerRuntime::with_memory(memory);

    while let Some(line) = read_bounded_line(&mut input, MAX_PROTOCOL_BYTES)? {
        let request: WorkerRequest =
            serde_json::from_slice(&line).context("invalid worker request")?;
        let response = runtime.execute(request)?;
        let mut encoded =
            serde_json::to_vec(&response).context("unable to encode worker response")?;
        encoded.push(b'\n');
        if encoded.len() > MAX_PROTOCOL_BYTES {
            bail!("worker response exceeds {} bytes", MAX_PROTOCOL_BYTES);
        }
        output
            .write_all(&encoded)
            .context("unable to write worker response")?;
        output.flush().context("unable to flush worker response")?;
    }
    Ok(())
}

fn worker_env(key: &str, default: usize) -> Result<usize> {
    match std::env::var(key) {
        Ok(value) => value
            .parse::<usize>()
            .with_context(|| format!("invalid {key}")),
        Err(std::env::VarError::NotPresent) => Ok(default),
        Err(error) => Err(anyhow!("invalid {key}: {error}")),
    }
}

#[cfg(target_os = "linux")]
fn apply_worker_memory_limit(bytes: usize) -> Result<()> {
    let limit = bytes as libc::rlim_t;
    let resource_limit = libc::rlimit {
        rlim_cur: limit,
        rlim_max: limit,
    };
    let result = unsafe { libc::setrlimit(libc::RLIMIT_AS, &resource_limit) };
    if result != 0 {
        return Err(io::Error::last_os_error())
            .context("unable to apply worker address-space limit");
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn apply_worker_memory_limit(_bytes: usize) -> Result<()> {
    // Darwin address-space reservations are not comparable to Linux RLIMIT_AS,
    // and Windows requires a separate Job Object implementation. Scheduling and
    // logical limits still apply; no hard OS memory limit is claimed here.
    Ok(())
}

fn read_bounded_line<R: BufRead>(reader: &mut R, limit: usize) -> io::Result<Option<Vec<u8>>> {
    let mut line = Vec::new();
    loop {
        let chunk = reader.fill_buf()?;
        if chunk.is_empty() {
            if line.is_empty() {
                return Ok(None);
            }
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "unterminated JSON line",
            ));
        }
        let newline = chunk.iter().position(|byte| *byte == b'\n');
        let consumed = newline.map_or(chunk.len(), |position| position + 1);
        let content_len = newline.unwrap_or(consumed);
        if line.len().saturating_add(consumed) > limit {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "JSON line exceeds limit",
            ));
        }
        line.extend_from_slice(&chunk[..content_len]);
        reader.consume(consumed);
        if newline.is_some() {
            return Ok(Some(line));
        }
    }
}

struct WorkerProcess {
    child: Arc<Mutex<Child>>,
    stdin: ChildStdin,
    responses: Receiver<io::Result<Vec<u8>>>,
}

struct ProcessWatchdog {
    cancel: Option<Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl ProcessWatchdog {
    fn new(child: Arc<Mutex<Child>>, timeout: Duration) -> Self {
        let (cancel, receiver) = mpsc::channel();
        let thread = thread::spawn(move || {
            if !matches!(
                receiver.recv_timeout(timeout),
                Err(RecvTimeoutError::Timeout)
            ) {
                return;
            }
            {
                let mut child = match child.lock() {
                    Ok(child) => child,
                    Err(poisoned) => poisoned.into_inner(),
                };
                kill_process_tree(&mut child);
            }
        });
        Self {
            cancel: Some(cancel),
            thread: Some(thread),
        }
    }
}

/// Kill a worker and every process it may have spawned.
///
/// A worker's own child process (e.g. a shell helper, or in tests a mock
/// script that forks `sleep`) can keep the worker's stdout pipe open after
/// `Child::kill` terminates only the immediate process, since the
/// descendant inherits the same pipe file descriptor. That leaves the
/// watchdog's promised deadline hollow: the parent only observes EOF once
/// every process holding the write end exits, which can be far later than
/// the configured timeout. Spawning each worker in its own process group
/// (see [`WorkerProcess::spawn`]) lets us kill the whole tree at once.
#[cfg(unix)]
fn kill_process_tree(child: &mut Child) {
    let pid = child.id() as libc::pid_t;
    // Negative pid targets the whole process group; the worker is always
    // spawned as that group's leader (pgid == pid).
    unsafe {
        libc::kill(-pid, libc::SIGKILL);
    }
    let _ = child.kill();
}

#[cfg(not(unix))]
fn kill_process_tree(child: &mut Child) {
    let _ = child.kill();
}

impl Drop for ProcessWatchdog {
    fn drop(&mut self) {
        self.cancel.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl WorkerProcess {
    fn spawn(executable: &Path, memory: MemoryProfile) -> Result<Self> {
        let mut command = Command::new(executable);
        command
            .arg("__wt-worker")
            .env("WT_WORKER_MEMORY_BYTES", memory.worker_bytes.to_string())
            .env("WT_PARENT_MEMORY_BYTES", memory.parent_bytes.to_string())
            .env("WT_TOTAL_MEMORY_BYTES", memory.total_bytes.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        // Make the worker the leader of its own process group so the
        // watchdog can kill it and anything it forks in one shot; see
        // `kill_process_tree`.
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let mut child = command
            .spawn()
            .with_context(|| format!("unable to spawn worker {}", executable.display()))?;
        let stdin = match child.stdin.take() {
            Some(stdin) => stdin,
            None => {
                kill_process_tree(&mut child);
                let _ = child.wait();
                bail!("worker stdin was not piped");
            }
        };
        let stdout = match child.stdout.take() {
            Some(stdout) => stdout,
            None => {
                kill_process_tree(&mut child);
                let _ = child.wait();
                bail!("worker stdout was not piped");
            }
        };
        let child = Arc::new(Mutex::new(child));
        let (sender, responses) = mpsc::sync_channel(1);
        thread::spawn(move || {
            let mut stdout = BufReader::new(stdout);
            loop {
                let result = read_bounded_line(&mut stdout, MAX_PROTOCOL_BYTES).and_then(|line| {
                    line.ok_or_else(|| {
                        io::Error::new(
                            io::ErrorKind::UnexpectedEof,
                            "worker exited without a response",
                        )
                    })
                });
                let failed = result.is_err();
                if sender.send(result).is_err() || failed {
                    break;
                }
            }
        });
        Ok(Self {
            child,
            stdin,
            responses,
        })
    }

    fn request(
        &mut self,
        request: &WorkerRequest,
        timeouts: WorkerTimeouts,
    ) -> Result<WorkerResult> {
        let repository = request.repository
            || request.rules.iter().any(|rule| {
                rule.manifest.get("execution").and_then(Value::as_str) == Some("repository")
            });
        let timeout = if repository {
            timeouts.repository
        } else {
            timeouts.invocation
        };
        let _watchdog = ProcessWatchdog::new(Arc::clone(&self.child), timeout);
        self.request_inner(request)
    }

    fn request_inner(&mut self, request: &WorkerRequest) -> Result<WorkerResult> {
        let mut encoded = serde_json::to_vec(request).context("unable to encode worker request")?;
        encoded.push(b'\n');
        if encoded.len() > MAX_PROTOCOL_BYTES {
            bail!("worker request exceeds {} bytes", MAX_PROTOCOL_BYTES);
        }
        self.stdin
            .write_all(&encoded)
            .context("unable to write worker request")?;
        self.stdin
            .flush()
            .context("unable to flush worker request")?;

        let response = match self.responses.recv() {
            Ok(result) => result.context("worker response reader failed")?,
            Err(_) => bail!("worker response channel disconnected"),
        };
        let mut result: WorkerResult =
            serde_json::from_slice(&response).context("invalid worker response")?;
        result.stats["transport"] =
            serde_json::json!({"request_bytes":encoded.len(),"response_bytes":response.len()});
        Ok(result)
    }
}

impl Drop for WorkerProcess {
    fn drop(&mut self) {
        if let Ok(mut child) = self.child.lock() {
            kill_process_tree(&mut child);
            let _ = child.wait();
        }
    }
}

struct WorkerSlot {
    executable: PathBuf,
    memory: MemoryProfile,
    timeouts: WorkerTimeouts,
    process: Option<WorkerProcess>,
}

impl WorkerSlot {
    fn new(executable: PathBuf, memory: MemoryProfile, timeouts: WorkerTimeouts) -> Result<Self> {
        let process = WorkerProcess::spawn(&executable, memory)?;
        Ok(Self {
            executable,
            memory,
            timeouts,
            process: Some(process),
        })
    }

    fn execute(&mut self, request: &WorkerRequest) -> Result<WorkerResult> {
        if self.process.is_none() {
            self.process = Some(WorkerProcess::spawn(&self.executable, self.memory)?);
        }
        let result = self
            .process
            .as_mut()
            .expect("worker process initialized")
            .request(request, self.timeouts);
        if result.is_err() {
            self.process.take();
        }
        result
    }
}

/// Persistent process pool for isolated worker runtimes.
pub struct Pool {
    workers: Vec<Mutex<WorkerSlot>>,
    next_worker: usize,
}

impl Pool {
    /// Spawn at most three workers from the explicit 1 GiB budget: 256 MiB for
    /// the parent and 256 MiB per worker. This makes no platform-specific RSS
    /// claim.
    pub fn new(executable: &Path, jobs: usize) -> Result<Self> {
        Self::with_memory(executable, jobs, MemoryProfile::default())
    }

    pub fn with_memory(executable: &Path, jobs: usize, memory: MemoryProfile) -> Result<Self> {
        Self::with_memory_and_timeouts(executable, jobs, memory, WorkerTimeouts::default())
    }

    /// Like [`Pool::with_memory`], but with explicit watchdog deadlines
    /// instead of the production [`WorkerTimeouts::default`]. Intended for
    /// tests that need to exercise the kill-and-recover path without racing
    /// the tight production timeouts under heavy machine load.
    pub fn with_memory_and_timeouts(
        executable: &Path,
        jobs: usize,
        memory: MemoryProfile,
        timeouts: WorkerTimeouts,
    ) -> Result<Self> {
        if jobs == 0 {
            bail!("worker pool requires at least one job");
        }
        let admitted = memory.max_workers()?;
        if jobs > admitted {
            bail!(
                "worker pool supports at most {admitted} jobs under the configured memory budget"
            );
        }
        let count = jobs;
        let mut workers = Vec::with_capacity(count);
        for _ in 0..count {
            workers.push(Mutex::new(WorkerSlot::new(
                executable.to_owned(),
                memory,
                timeouts,
            )?));
        }
        Ok(Self {
            workers,
            next_worker: 0,
        })
    }

    pub fn execute(&mut self, request: WorkerRequest) -> Result<WorkerResult> {
        let worker = self.next_worker % self.workers.len();
        self.next_worker = self.next_worker.wrapping_add(1);
        let mut slot = self.workers[worker]
            .lock()
            .map_err(|_| anyhow!("worker slot lock poisoned"))?;
        let mut result = slot.execute(&request)?;
        result.stats["pool_worker_count"] = serde_json::json!(self.workers.len());
        Ok(result)
    }

    /// Dispatch requests concurrently, assigning them round-robin to
    /// independent persistent processes and preserving input order in the
    /// returned vector. Requests sharing a slot are serialized by that slot.
    pub fn execute_many(&mut self, requests: Vec<WorkerRequest>) -> Vec<Result<WorkerResult>> {
        if requests.is_empty() {
            return Vec::new();
        }
        let start = self.next_worker;
        self.next_worker = self.next_worker.wrapping_add(requests.len());
        let workers = &self.workers;
        let worker_count = workers.len();
        thread::scope(|scope| {
            let handles = requests
                .into_iter()
                .enumerate()
                .map(|(index, request)| {
                    let worker = &workers[start.wrapping_add(index) % workers.len()];
                    scope.spawn(move || {
                        let mut slot = worker
                            .lock()
                            .map_err(|_| anyhow!("worker slot lock poisoned"))?;
                        let mut result = slot.execute(&request)?;
                        result.stats["pool_worker_count"] = serde_json::json!(worker_count);
                        Ok(result)
                    })
                })
                .collect::<Vec<_>>();
            handles
                .into_iter()
                .map(|handle| {
                    handle
                        .join()
                        .unwrap_or_else(|_| Err(anyhow!("worker dispatch thread panicked")))
                })
                .collect()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn bounded_reader_requires_one_complete_line() {
        let mut reader = Cursor::new(b"{}\n");
        assert_eq!(
            read_bounded_line(&mut reader, 3).unwrap(),
            Some(b"{}".to_vec())
        );
        assert_eq!(read_bounded_line(&mut reader, 3).unwrap(), None);
    }

    #[test]
    fn bounded_reader_rejects_oversized_lines() {
        let mut reader = Cursor::new(b"1234\n");
        assert!(read_bounded_line(&mut reader, 4).is_err());
    }

    #[test]
    fn bounded_reader_rejects_unterminated_lines() {
        let mut reader = Cursor::new(b"{}");
        assert!(read_bounded_line(&mut reader, 3).is_err());
    }

    #[test]
    fn thousand_programs_with_one_pattern_fit_the_compiled_cache() {
        let mut runtime = WorkerRuntime::new();
        for index in 0..1_000 {
            let alias = format!("pattern_{index}");
            let rule = WorkerRule {
                id: format!("rule-{index}"),
                manifest: serde_json::json!({
                    "execution": "file",
                    "patterns": {alias.clone(): "same"},
                    "diagnostics": {"hit": {"kind": "violation"}},
                    "code": {"language": "wt-rule-1", "capabilities": ["text.v1"]}
                }),
                source: format!("rx::is_match(file, \"{alias}\");"),
            };
            runtime.compiled_program(&rule).unwrap();
        }

        assert_eq!(runtime.compiled.len(), 1_000);
        assert_eq!(runtime.pattern_reservations.len(), 1);
        assert!(runtime.compiled_bytes < COMPILED_CACHE_BYTES);
    }
}
