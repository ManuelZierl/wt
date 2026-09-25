use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use wt_core::worker::{MemoryProfile, Pool, SourceFileWire, WorkerRequest, WorkerRule};

fn worker_executable() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_wt"))
}

fn request(source: &str) -> WorkerRequest {
    WorkerRequest {
        rules: vec![WorkerRule {
            id: "worker-test".to_owned(),
            manifest: serde_json::json!({
                "schema_version": 1,
                "id": "worker-test",
                "title": "worker test",
                "documentation": {"source": "# Worker test\n\nWorker test.\n"},
                "mode": "advisory",
                "severity": "warning",
                "execution": "file",
                "scope": {"include": ["**/*"]},
                "patterns": {"bad": "bad"},
                "diagnostics": {"hit": {"kind": "violation", "message": "bad", "help": "fix"}},
                "code": {"language": "wt-rule-1", "capabilities": ["text.v1"]}
            }),
            source: "for m in rx::find_all(file, \"bad\") { emit(m.span, \"hit\"); }".to_owned(),
        }],
        files: vec![SourceFileWire {
            path: "src/input.txt".to_owned(),
            text: source.to_owned(),
        }],
        optimized: true,
        limits: wt_runtime::RuntimeLimits {
            max_file_bytes: 4 * 1024 * 1024,
            ..wt_runtime::RuntimeLimits::default()
        },
        repository: false,
    }
}

#[test]
fn worker_protocol_round_trips_bounded_wire_types() {
    let request = request("fn main() {}");
    let encoded = serde_json::to_vec(&request).unwrap();
    let decoded: WorkerRequest = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(decoded.rules[0].id, "worker-test");
    assert_eq!(decoded.files[0].path, "src/input.txt");
    assert!(decoded.optimized);
}

#[test]
fn real_workers_preserve_order_and_serial_semantics() {
    let executable = worker_executable();
    let first = request("bad");
    let second = request("good");

    let mut serial = Pool::new(&executable, 1).unwrap();
    let serial_first = serial.execute(first.clone()).unwrap();
    let serial_second = serial.execute(second.clone()).unwrap();

    let mut parallel = Pool::new(&executable, 2).unwrap();
    let parallel_results = parallel.execute_many(vec![first, second]);
    assert_eq!(parallel_results.len(), 2);
    assert_eq!(
        parallel_results[0].as_ref().unwrap().results,
        serial_first.results
    );
    assert_eq!(
        parallel_results[1].as_ref().unwrap().results,
        serial_second.results
    );
    assert_eq!(
        parallel_results[0].as_ref().unwrap().stats["pool_worker_count"],
        2
    );
    assert!(parallel_results[0].as_ref().unwrap().stats["worker_pid"]
        .as_u64()
        .is_some_and(|pid| pid > 0));
    assert!(Pool::new(&executable, 4).is_err());
}

#[test]
fn invalid_worker_frame_exits_with_error() {
    let mut child = Command::new(worker_executable())
        .arg("__wt-worker")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"{not-json}\n")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
}

#[test]
fn compile_failure_is_returned_without_input() {
    let mut invalid = request("");
    invalid.files.clear();
    invalid.rules[0].source = "not valid WT".to_owned();
    let mut pool = Pool::new(&worker_executable(), 1).unwrap();
    let result = pool.execute(invalid).unwrap();
    assert!(result.results[0].error.is_some());
}

#[cfg(unix)]
#[test]
fn killed_worker_slot_recovers_on_next_request() {
    use std::os::unix::fs::PermissionsExt;
    use std::time::Duration;
    use tempfile::tempdir;
    use wt_core::worker::WorkerTimeouts;

    let directory = tempdir().unwrap();
    let script = directory.path().join("worker.sh");
    std::fs::write(
        &script,
        // Persist the invocation count in the state file with the fewest
        // possible forks (no `cat`) before ever blocking, so a heavily
        // loaded test machine still records "this is the first launch"
        // before the watchdog can kill this process. The first invocation
        // then sleeps far longer than the watchdog timeout configured below,
        // so the kill is a deterministic outcome of the timeout firing
        // rather than a race against how quickly the sleep happens to
        // finish under load.
        "#!/bin/sh\nstate=\"$0.state\"\ncount=0\nif test -f \"$state\"; then read -r count < \"$state\"; fi\ncount=$((count + 1))\nprintf '%s' \"$count\" > \"$state\"\nwhile IFS= read -r request; do\n  if test \"$count\" -eq 1; then sleep 30; else printf '%s\\n' '{\"results\":[],\"stats\":{}}'; fi\ndone\n",
    )
    .unwrap();
    let mut permissions = std::fs::metadata(&script).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&script, permissions).unwrap();

    // Use an explicit, generous watchdog timeout instead of the tight
    // production `INVOCATION_TIMEOUT` (2s). The production constant leaves
    // little margin against process-scheduling jitter on a heavily loaded
    // test machine: both the "kill fires well before the 30s sleep would
    // end" side and the "the freshly respawned worker replies in time" side
    // of this test need real wall-clock headroom to be non-flaky under
    // load, not just under quiet conditions. The watchdog kills the whole
    // process group (see `kill_process_tree` in wt-core), so this timeout
    // — not the mock worker's sleep duration — bounds how long the first
    // `execute` call takes.
    let timeouts = WorkerTimeouts {
        invocation: Duration::from_secs(5),
        repository: Duration::from_secs(5),
    };
    let mut pool =
        Pool::with_memory_and_timeouts(&script, 1, MemoryProfile::default(), timeouts).unwrap();
    assert!(pool.execute(request("bad")).is_err());
    assert!(pool.execute(request("bad")).is_ok());
}
