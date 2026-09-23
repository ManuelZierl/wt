use crate::digest::{digest_bytes, normalize_reference, semantic_digest};
use crate::fixtures::TestOutcome;
use crate::rule::RulePackage;
use anyhow::{Context, Result};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use tempfile::NamedTempFile;
use wt_runtime::RawDiagnostic;

const CACHE_SCHEMA_VERSION: u64 = 3;
const MAX_CACHE_ENTRY_BYTES: usize = 16 * 1024 * 1024;
const CACHE_SEMANTICS_VERSION: &str = "WT-CACHE-3";
const RUNTIME_RESOURCE_PROFILE: &str = "wt-runtime-resource-profile-1";
const COMPILED_SEMANTIC_INPUT: &str = concat!(
    "WT-COMPILED-SEMANTICS-1\n",
    env!("CARGO_PKG_VERSION"),
    "\n",
    include_str!("../../wt-runtime/src/lib.rs"),
    "\n-- Cargo.lock --\n",
    include_str!("../../../Cargo.lock"),
    "\n-- rust-toolchain.toml --\n",
    include_str!("../../../rust-toolchain.toml"),
);

#[derive(Default, Serialize)]
pub struct CacheStats {
    pub enabled: bool,
    pub raw_hits: usize,
    pub raw_misses: usize,
    pub raw_writes: usize,
    pub fixture_hits: usize,
    pub fixture_misses: usize,
    pub fixture_writes: usize,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEntry {
    schema_version: u64,
    key: String,
    diagnostics: Vec<RawDiagnostic>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureEntry {
    schema_version: u64,
    key: String,
    outcome: TestOutcome,
}

pub struct Store {
    root: Option<PathBuf>,
    pub stats: CacheStats,
    pub notices: Vec<String>,
}

impl Store {
    pub fn new(enabled: bool) -> Self {
        if !enabled {
            return Self::disabled();
        }
        match directory() {
            Ok(root) => Self::at_directory(root),
            Err(error) => Self {
                root: None,
                stats: CacheStats {
                    enabled: true,
                    ..CacheStats::default()
                },
                notices: vec![format!("cache unavailable: {error}")],
            },
        }
    }

    pub fn at_directory(path: impl AsRef<Path>) -> Self {
        let root = path.as_ref().to_path_buf();
        let stats = CacheStats {
            enabled: true,
            ..CacheStats::default()
        };
        if let Err(error) = fs::create_dir_all(root.join("raw")) {
            return Self {
                root: None,
                stats,
                notices: vec![format!("cache unavailable: {error}")],
            };
        }
        if let Err(error) = fs::create_dir_all(root.join("fixtures")) {
            return Self {
                root: None,
                stats,
                notices: vec![format!("fixture cache unavailable: {error}")],
            };
        }
        Self {
            root: Some(root),
            stats,
            notices: Vec::new(),
        }
    }

    fn disabled() -> Self {
        Self {
            root: None,
            stats: CacheStats {
                enabled: false,
                ..CacheStats::default()
            },
            notices: Vec::new(),
        }
    }

    pub fn read_raw(&mut self, key: &str) -> Option<Vec<RawDiagnostic>> {
        let entry = self.load_raw(key)?;
        self.stats.raw_hits += 1;
        Some(entry.diagnostics)
    }

    #[allow(dead_code)]
    pub fn read_raw_validated<I, S>(
        &mut self,
        key: &str,
        path: &str,
        text: &str,
        diagnostic_ids: I,
    ) -> Option<Vec<RawDiagnostic>>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let diagnostic_ids = diagnostic_ids
            .into_iter()
            .map(|id| id.as_ref().to_owned())
            .collect::<std::collections::HashSet<_>>();
        let entry = self.load_raw(key)?;
        if entry.diagnostics.iter().all(|diagnostic| {
            diagnostic.path == path
                && diagnostic_ids.contains(&diagnostic.code)
                && diagnostic.start_byte <= diagnostic.end_byte
                && diagnostic.end_byte <= text.len()
                && text.is_char_boundary(diagnostic.start_byte)
                && text.is_char_boundary(diagnostic.end_byte)
        }) {
            self.stats.raw_hits += 1;
            Some(entry.diagnostics)
        } else {
            self.stats.raw_misses += 1;
            self.discard_raw(key, "discarded invalid raw cache entry");
            None
        }
    }

    pub fn write_raw(&mut self, key: &str, diagnostics: &[RawDiagnostic]) {
        let Some(root) = &self.root else {
            return;
        };
        let entry = RawEntry {
            schema_version: CACHE_SCHEMA_VERSION,
            key: key.to_owned(),
            diagnostics: diagnostics.to_vec(),
        };
        let path = entry_path(root, "raw", key);
        match write_atomic(&path, &entry) {
            Ok(()) => self.stats.raw_writes += 1,
            Err(error) => self.notices.push(format!("cache write failed: {error}")),
        }
    }

    pub fn read_fixture(&mut self, key: &str) -> Option<TestOutcome> {
        let root = self.root.clone()?;
        let path = entry_path(&root, "fixtures", key);
        let Some(entry) = self.load_entry::<FixtureEntry>(&path, "fixture") else {
            self.stats.fixture_misses += 1;
            return None;
        };
        if entry.schema_version == CACHE_SCHEMA_VERSION
            && entry.key == key
            && valid_fixture_outcome(&entry.outcome)
        {
            self.stats.fixture_hits += 1;
            Some(entry.outcome)
        } else {
            self.stats.fixture_misses += 1;
            self.discard_path(&path, "discarded corrupt fixture cache entry");
            None
        }
    }

    pub fn write_fixture(&mut self, key: &str, outcome: &TestOutcome) {
        if !outcome.passed {
            return;
        }
        let Some(root) = &self.root else {
            return;
        };
        let entry = FixtureEntry {
            schema_version: CACHE_SCHEMA_VERSION,
            key: key.to_owned(),
            outcome: outcome.clone(),
        };
        let path = entry_path(root, "fixtures", key);
        match write_atomic(&path, &entry) {
            Ok(()) => self.stats.fixture_writes += 1,
            Err(error) => self
                .notices
                .push(format!("fixture cache write failed: {error}")),
        }
    }

    fn load_raw(&mut self, key: &str) -> Option<RawEntry> {
        let root = self.root.clone()?;
        let path = entry_path(&root, "raw", key);
        let Some(entry) = self.load_entry::<RawEntry>(&path, "raw") else {
            self.stats.raw_misses += 1;
            return None;
        };
        if entry.schema_version == CACHE_SCHEMA_VERSION && entry.key == key {
            Some(entry)
        } else {
            self.stats.raw_misses += 1;
            self.discard_path(&path, "discarded corrupt raw cache entry");
            None
        }
    }

    fn load_entry<T: DeserializeOwned>(&mut self, path: &Path, kind: &str) -> Option<T> {
        let bytes = match read_bounded(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
            Err(error) => {
                self.notices
                    .push(format!("{kind} cache read failed: {error}"));
                return None;
            }
        };
        match serde_json::from_slice(&bytes) {
            Ok(entry) => Some(entry),
            Err(error) => {
                self.notices
                    .push(format!("discarded corrupt {kind} cache entry: {error}"));
                self.discard_path(path, "removed corrupt cache entry");
                None
            }
        }
    }

    fn discard_raw(&mut self, key: &str, notice: &str) {
        if let Some(root) = &self.root {
            self.discard_path(&entry_path(root, "raw", key), notice);
        }
    }

    fn discard_path(&mut self, path: &Path, notice: &str) {
        match fs::remove_file(path) {
            Ok(()) => self.notices.push(notice.to_owned()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.notices.push(notice.to_owned())
            }
            Err(error) => self
                .notices
                .push(format!("{notice}; unable to remove cache entry: {error}")),
        }
    }

    pub fn stats_value(&self) -> Value {
        serde_json::to_value(&self.stats).expect("cache stats serialization")
    }
}

pub fn raw_key(package: &RulePackage, path: &str, file_digest: &str) -> String {
    let rule = execution_digest(package);
    let applicability = serde_json::to_vec(&package.manifest.scope).expect("rule scope identity");
    let resource_profile = serde_json::to_vec(&(
        RUNTIME_RESOURCE_PROFILE,
        &package.manifest.execution,
        &package.manifest.code.capabilities,
    ))
    .expect("runtime resource identity");
    semantic_digest(
        "WT-RAW-FILE-3",
        &[
            CACHE_SEMANTICS_VERSION.as_bytes(),
            compiled_semantic_identity().as_bytes(),
            rule.as_bytes(),
            &applicability,
            &resource_profile,
            path.as_bytes(),
            file_digest.as_bytes(),
        ],
    )
}

pub fn fixture_key(package: &RulePackage) -> Result<String> {
    let mut parts = vec![
        CACHE_SEMANTICS_VERSION.as_bytes().to_vec(),
        compiled_semantic_identity().as_bytes().to_vec(),
        package.directory.to_string_lossy().as_bytes().to_vec(),
        package.source.as_bytes().to_vec(),
    ];
    parts.push(serde_json::to_vec(&package.manifest.execution)?);
    parts.push(serde_json::to_vec(&package.manifest.scope)?);
    parts.push(serde_json::to_vec(&package.manifest.patterns)?);
    parts.push(serde_json::to_vec(&package.manifest.diagnostics)?);
    parts.push(serde_json::to_vec(&package.manifest.code.capabilities)?);
    if let Some(suite) = &package.tests {
        parts.push(serde_json::to_vec(suite)?);
        for case in &suite.cases {
            for file in &case.files {
                if let Some(path) = &file.fixture {
                    let normalized = normalize_reference(path)?;
                    let bytes = package
                        .fixture_contents
                        .get(&normalized)
                        .with_context(|| format!("fixture reference {path:?} was not loaded"))?;
                    parts.push(normalized.into_bytes());
                    parts.push(bytes.clone());
                }
            }
        }
    }
    let refs = parts.iter().map(Vec::as_slice).collect::<Vec<_>>();
    Ok(semantic_digest("WT-FIXTURES-3", &refs))
}

#[allow(dead_code)]
pub fn repo_raw_key(package: &RulePackage, files: &[(&str, &str)]) -> String {
    let mut files = files.to_vec();
    files.sort_unstable();
    let file_parts = files
        .into_iter()
        .flat_map(|(path, digest)| [path.as_bytes(), digest.as_bytes()])
        .collect::<Vec<_>>();
    let applicability = serde_json::to_vec(&package.manifest.scope).expect("rule scope identity");
    let resource_profile = serde_json::to_vec(&(
        RUNTIME_RESOURCE_PROFILE,
        &package.manifest.execution,
        &package.manifest.code.capabilities,
    ))
    .expect("runtime resource identity");
    let input_digest = semantic_digest("WT-REPOSITORY-INPUTS-1", &file_parts);
    semantic_digest(
        "WT-RAW-REPOSITORY-3",
        &[
            CACHE_SEMANTICS_VERSION.as_bytes(),
            compiled_semantic_identity().as_bytes(),
            execution_digest(package).as_bytes(),
            &applicability,
            &resource_profile,
            input_digest.as_bytes(),
        ],
    )
}

fn execution_digest(package: &RulePackage) -> String {
    let mut value = package.manifest_value.clone();
    if let Value::Object(object) = &mut value {
        object.remove("mode");
        object.remove("severity");
        object.remove("title");
        object.remove("description");
        object.remove("rationale");
        object.remove("metadata");
        object.remove("tests_file");
        if let Some(code) = object.get_mut("code").and_then(Value::as_object_mut) {
            code.remove("file");
            code.insert("source".to_owned(), Value::String(package.source.clone()));
        }
        if let Some(diagnostics) = object.get_mut("diagnostics").and_then(Value::as_object_mut) {
            for definition in diagnostics.values_mut() {
                *definition = json!(null);
            }
        }
    }
    semantic_digest(
        "WT-RULE-EXECUTION-2",
        &[serde_json::to_vec(&value)
            .expect("rule execution identity")
            .as_slice()],
    )
}

pub fn directory() -> Result<PathBuf> {
    if let Some(path) = absolute_env_path("XDG_CACHE_HOME")? {
        return Ok(path.join("wt"));
    }
    if let Some(path) = absolute_env_path("LOCALAPPDATA")? {
        return Ok(path.join("wt/cache"));
    }
    if let Some(path) = absolute_env_path("HOME")? {
        return Ok(path.join(".cache/wt"));
    }
    anyhow::bail!("unable to resolve cache directory")
}

pub fn clear() -> Result<()> {
    let path = directory()?;
    clear_at_directory(path)
}

#[allow(dead_code)]
pub fn clear_at_directory(path: impl AsRef<Path>) -> Result<()> {
    clear_directory(path.as_ref())
}

fn absolute_env_path(name: &str) -> Result<Option<PathBuf>> {
    let Some(path) = std::env::var_os(name) else {
        return Ok(None);
    };
    let path = PathBuf::from(path);
    if !path.is_absolute() {
        anyhow::bail!("{name} must be an absolute path")
    }
    Ok(Some(path))
}

fn clear_directory(path: &Path) -> Result<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() {
        anyhow::bail!(
            "refusing to clear symlink cache directory {}",
            path.display()
        )
    }
    if !metadata.is_dir() {
        anyhow::bail!("cache path is not a directory: {}", path.display())
    }
    fs::remove_dir_all(path).with_context(|| format!("unable to clear {}", path.display()))
}

fn compiled_semantic_identity() -> &'static str {
    static IDENTITY: OnceLock<String> = OnceLock::new();
    IDENTITY.get_or_init(|| digest_bytes(COMPILED_SEMANTIC_INPUT.as_bytes()))
}

fn valid_fixture_outcome(outcome: &TestOutcome) -> bool {
    outcome.passed && outcome.failed_cases.is_empty()
}

fn entry_path(root: &Path, kind: &str, key: &str) -> PathBuf {
    let digest = digest_bytes(key.as_bytes());
    let filename = digest.strip_prefix("sha256:").unwrap_or(&digest);
    root.join(kind).join(format!("{filename}.json"))
}

fn read_bounded(path: &Path) -> std::io::Result<Vec<u8>> {
    let mut file = File::open(path)?;
    let mut bytes = Vec::new();
    file.by_ref()
        .take(MAX_CACHE_ENTRY_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_CACHE_ENTRY_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("cache entry exceeds {MAX_CACHE_ENTRY_BYTES} bytes"),
        ));
    }
    Ok(bytes)
}

fn write_atomic<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let parent = path.parent().context("cache path has no parent")?;
    let temp = NamedTempFile::new_in(parent)?;
    serde_json::to_writer(&temp, value)?;
    temp.as_file().sync_all()?;
    temp.persist(path)
        .map_err(|error| anyhow::anyhow!("unable to persist cache entry: {error}"))?;
    Ok(())
}
