use crate::parse_json;
use crate::rule::valid_rule_id;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Clone, Debug, Default, Serialize)]
pub struct ConfigFile {
    pub schema_version: u64,
    pub scan: ScanConfig,
    pub runtime: RuntimeConfig,
    pub optimizer: OptimizerConfig,
    pub rules: RulesConfig,
    pub coverage: CoverageConfig,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoverageConfig {
    #[serde(default)]
    pub expectations: Vec<CoverageExpectation>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoverageExpectation {
    pub rule_id: String,
    pub minimum_files: u64,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct EffectiveExpectation {
    #[serde(flatten)]
    pub expectation: CoverageExpectation,
    pub origin: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct ScanConfig {
    pub respect_gitignore: bool,
    pub honor_git_local_excludes: bool,
    pub include_hidden: bool,
    pub max_file_bytes: u64,
    pub exclude: Vec<ExcludeConfig>,
    #[serde(skip_serializing)]
    supplied: ScanSupplied,
}

#[derive(Clone, Copy, Debug, Default)]
struct ScanSupplied {
    respect_gitignore: bool,
    honor_git_local_excludes: bool,
    include_hidden: bool,
    max_file_bytes: bool,
    exclude: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfigFile {
    schema_version: u64,
    #[serde(default)]
    scan: RawScanConfig,
    #[serde(default, deserialize_with = "present_object")]
    runtime: Option<RawRuntimeConfig>,
    #[serde(default, deserialize_with = "present_object")]
    optimizer: Option<RawOptimizerConfig>,
    #[serde(default)]
    rules: RulesConfig,
    #[serde(default, deserialize_with = "present_object")]
    coverage: Option<CoverageConfig>,
}

fn present_object<'de, D, T>(deserializer: D) -> std::result::Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawScanConfig {
    respect_gitignore: Option<bool>,
    honor_git_local_excludes: Option<bool>,
    include_hidden: Option<bool>,
    max_file_bytes: Option<u64>,
    exclude: Option<Vec<ExcludeConfig>>,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct RuntimeConfig {
    pub file_steps: u64,
    pub file_native_bytes: u64,
    pub repository_steps: u64,
    pub repository_native_bytes: u64,
    pub worker_memory_bytes: u64,
    pub parent_memory_bytes: u64,
    pub total_memory_bytes: u64,
    #[serde(skip_serializing)]
    supplied: RuntimeSupplied,
}

#[derive(Clone, Copy, Debug, Default)]
struct RuntimeSupplied {
    file_steps: bool,
    file_native_bytes: bool,
    repository_steps: bool,
    repository_native_bytes: bool,
    worker_memory_bytes: bool,
    parent_memory_bytes: bool,
    total_memory_bytes: bool,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRuntimeConfig {
    file_steps: Option<u64>,
    file_native_bytes: Option<u64>,
    repository_steps: Option<u64>,
    repository_native_bytes: Option<u64>,
    worker_memory_bytes: Option<u64>,
    parent_memory_bytes: Option<u64>,
    total_memory_bytes: Option<u64>,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        let ceilings = wt_runtime::RuntimeLimits::default();
        let memory = crate::worker::MemoryProfile::default();
        Self {
            file_steps: ceilings.file_steps,
            file_native_bytes: ceilings.file_native_bytes,
            repository_steps: ceilings.repository_steps,
            repository_native_bytes: ceilings.repository_native_bytes,
            worker_memory_bytes: memory.worker_bytes as u64,
            parent_memory_bytes: memory.parent_bytes as u64,
            total_memory_bytes: memory.total_bytes as u64,
            supplied: RuntimeSupplied::default(),
        }
    }
}

impl RuntimeConfig {
    pub fn memory(self) -> crate::worker::MemoryProfile {
        crate::worker::MemoryProfile {
            worker_bytes: self.worker_memory_bytes as usize,
            parent_bytes: self.parent_memory_bytes as usize,
            total_bytes: self.total_memory_bytes as usize,
        }
    }
    pub fn limits(self, max_file_bytes: u64) -> wt_runtime::RuntimeLimits {
        wt_runtime::RuntimeLimits {
            max_file_bytes: max_file_bytes as usize,
            file_steps: self.file_steps,
            file_native_bytes: self.file_native_bytes,
            repository_steps: self.repository_steps,
            repository_native_bytes: self.repository_native_bytes,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct OptimizerConfig {
    pub mode: String,
    #[serde(skip_serializing)]
    supplied: bool,
}

impl Default for OptimizerConfig {
    fn default() -> Self {
        Self {
            mode: "auto".to_owned(),
            supplied: false,
        }
    }
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawOptimizerConfig {
    mode: Option<String>,
}

impl<'de> Deserialize<'de> for ConfigFile {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = RawConfigFile::deserialize(deserializer)?;
        let mut scan = ScanConfig::default();
        scan.supplied = ScanSupplied {
            respect_gitignore: raw.scan.respect_gitignore.is_some(),
            honor_git_local_excludes: raw.scan.honor_git_local_excludes.is_some(),
            include_hidden: raw.scan.include_hidden.is_some(),
            max_file_bytes: raw.scan.max_file_bytes.is_some(),
            exclude: raw.scan.exclude.is_some(),
        };
        if let Some(value) = raw.scan.respect_gitignore {
            scan.respect_gitignore = value;
        }
        if let Some(value) = raw.scan.honor_git_local_excludes {
            scan.honor_git_local_excludes = value;
        }
        if let Some(value) = raw.scan.include_hidden {
            scan.include_hidden = value;
        }
        if let Some(value) = raw.scan.max_file_bytes {
            scan.max_file_bytes = value;
        }
        if let Some(value) = raw.scan.exclude {
            scan.exclude = value;
        }
        let mut runtime = RuntimeConfig::default();
        if let Some(raw) = raw.runtime {
            runtime.supplied = RuntimeSupplied {
                file_steps: raw.file_steps.is_some(),
                file_native_bytes: raw.file_native_bytes.is_some(),
                repository_steps: raw.repository_steps.is_some(),
                repository_native_bytes: raw.repository_native_bytes.is_some(),
                worker_memory_bytes: raw.worker_memory_bytes.is_some(),
                parent_memory_bytes: raw.parent_memory_bytes.is_some(),
                total_memory_bytes: raw.total_memory_bytes.is_some(),
            };
            if let Some(value) = raw.file_steps {
                runtime.file_steps = value;
            }
            if let Some(value) = raw.file_native_bytes {
                runtime.file_native_bytes = value;
            }
            if let Some(value) = raw.repository_steps {
                runtime.repository_steps = value;
            }
            if let Some(value) = raw.repository_native_bytes {
                runtime.repository_native_bytes = value;
            }
            if let Some(value) = raw.worker_memory_bytes {
                runtime.worker_memory_bytes = value;
            }
            if let Some(value) = raw.parent_memory_bytes {
                runtime.parent_memory_bytes = value;
            }
            if let Some(value) = raw.total_memory_bytes {
                runtime.total_memory_bytes = value;
            }
        }
        let mut optimizer = OptimizerConfig::default();
        if let Some(raw) = raw.optimizer {
            optimizer.supplied = raw.mode.is_some();
            if let Some(value) = raw.mode {
                optimizer.mode = value;
            }
        }
        Ok(Self {
            schema_version: raw.schema_version,
            scan,
            runtime,
            optimizer,
            rules: raw.rules,
            coverage: raw.coverage.unwrap_or_default(),
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExcludeConfig {
    pub glob: String,
    pub reason: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RulesConfig {
    #[serde(default)]
    pub mode_overrides: std::collections::BTreeMap<String, ModeOverride>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModeOverride {
    pub mode: String,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct EffectiveConfig {
    pub schema_version: u64,
    pub scan: ScanConfig,
    pub runtime: RuntimeConfig,
    pub optimizer: OptimizerConfig,
    pub mode_overrides: std::collections::BTreeMap<String, ModeOverride>,
    pub coverage: CoverageConfig,
    pub expectations: Vec<EffectiveExpectation>,
    pub origins: Vec<ValueOrigin>,
    pub inactive_overrides: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ValueOrigin {
    pub key: String,
    pub origin: String,
}

impl Default for ScanConfig {
    fn default() -> Self {
        Self {
            respect_gitignore: true,
            honor_git_local_excludes: true,
            include_hidden: true,
            max_file_bytes: default_max_file_bytes(),
            exclude: Vec::new(),
            supplied: ScanSupplied::default(),
        }
    }
}

pub fn default_max_file_bytes() -> u64 {
    4 * 1024 * 1024
}

pub fn load(path: &Path) -> Result<Option<ConfigFile>> {
    if !path.exists() {
        return Ok(None);
    }
    let mut file =
        fs::File::open(path).with_context(|| format!("unable to read {}", path.display()))?;
    let mut bytes = Vec::new();
    file.by_ref()
        .take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("unable to read {}", path.display()))?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        bail!(
            "configuration {} exceeds the {} byte limit",
            path.display(),
            MAX_FILE_BYTES
        )
    }
    let value = parse_json(std::str::from_utf8(&bytes)?)?;
    let config: ConfigFile = serde_json::from_value(value).context("invalid configuration")?;
    if config.schema_version != crate::CONTRACT_VERSION {
        bail!(
            "configuration schema_version must be {}",
            crate::CONTRACT_VERSION
        )
    }
    validate(&config)?;
    Ok(Some(config))
}

pub fn validate(config: &ConfigFile) -> Result<()> {
    let runtime = config.runtime;
    let ceilings = wt_runtime::RuntimeLimits::default();
    for (key, value, ceiling) in [
        (
            "runtime.file_steps",
            runtime.file_steps,
            ceilings.file_steps,
        ),
        (
            "runtime.file_native_bytes",
            runtime.file_native_bytes,
            ceilings.file_native_bytes,
        ),
        (
            "runtime.repository_steps",
            runtime.repository_steps,
            ceilings.repository_steps,
        ),
        (
            "runtime.repository_native_bytes",
            runtime.repository_native_bytes,
            ceilings.repository_native_bytes,
        ),
    ] {
        if value == 0 || value > ceiling {
            bail!("{key} must be between 1 and {ceiling}")
        }
    }
    for (key, value) in [
        ("runtime.worker_memory_bytes", runtime.worker_memory_bytes),
        ("runtime.parent_memory_bytes", runtime.parent_memory_bytes),
        ("runtime.total_memory_bytes", runtime.total_memory_bytes),
    ] {
        if value > usize::MAX as u64 {
            bail!("{key} exceeds the platform address-space range");
        }
    }
    runtime.memory().max_workers()?;
    if !matches!(config.optimizer.mode.as_str(), "auto" | "off") {
        bail!("optimizer.mode must be auto or off")
    }
    if config.scan.max_file_bytes == 0 || config.scan.max_file_bytes > MAX_FILE_BYTES {
        bail!("scan.max_file_bytes exceeds the binary safety ceiling")
    }
    for exclude in &config.scan.exclude {
        if exclude.glob.is_empty()
            || exclude.glob.contains('\\')
            || exclude.glob.starts_with('/')
            || exclude.reason.trim().is_empty()
        {
            bail!("configuration exclusions require safe globs and reasons")
        }
        globset::GlobBuilder::new(&exclude.glob)
            .literal_separator(true)
            .build()?;
    }
    for (id, override_value) in &config.rules.mode_overrides {
        if !(id.starts_with("global/") || id.starts_with("local/")) {
            bail!("mode override {id:?} must use a qualified rule id")
        }
        if !valid_rule_id(id.split_once('/').map_or("", |(_, value)| value))
            || !matches!(
                override_value.mode.as_str(),
                "advisory" | "enforced" | "disabled"
            )
            || override_value.reason.trim().is_empty()
        {
            bail!("invalid mode override {id:?}")
        }
    }
    for expectation in &config.coverage.expectations {
        if !valid_qualified_id(&expectation.rule_id)
            || expectation.minimum_files == 0
            || expectation.reason.trim().is_empty()
        {
            bail!("invalid coverage expectation for {:?}: use a qualified rule ID, positive minimum_files and nonempty reason", expectation.rule_id)
        }
    }
    Ok(())
}

fn valid_qualified_id(id: &str) -> bool {
    id.split_once('/')
        .is_some_and(|(scope, name)| matches!(scope, "global" | "local") && valid_rule_id(name))
}

pub fn merge(
    global: Option<&ConfigFile>,
    local: Option<&ConfigFile>,
    options: &Value,
) -> Result<EffectiveConfig> {
    let mut scan = ScanConfig::default();
    let mut runtime = RuntimeConfig::default();
    let mut optimizer = OptimizerConfig::default();
    let mut origins = vec![
        ValueOrigin {
            key: "scan.respect_gitignore".to_owned(),
            origin: "built-in".to_owned(),
        },
        ValueOrigin {
            key: "scan.honor_git_local_excludes".to_owned(),
            origin: "built-in".to_owned(),
        },
        ValueOrigin {
            key: "scan.include_hidden".to_owned(),
            origin: "built-in".to_owned(),
        },
        ValueOrigin {
            key: "scan.max_file_bytes".to_owned(),
            origin: "built-in".to_owned(),
        },
    ];
    let mut overrides = std::collections::BTreeMap::new();
    for key in [
        "runtime.file_steps",
        "runtime.file_native_bytes",
        "runtime.repository_steps",
        "runtime.repository_native_bytes",
        "runtime.worker_memory_bytes",
        "runtime.parent_memory_bytes",
        "runtime.total_memory_bytes",
        "optimizer.mode",
    ] {
        origins.push(ValueOrigin {
            key: key.to_owned(),
            origin: "built-in".to_owned(),
        });
    }
    let mut expectations = Vec::new();
    for (name, config, origin) in [("global", global, "global"), ("local", local, "local")] {
        if let Some(config) = config {
            apply_scan(&mut scan, &mut origins, &config.scan, origin);
            for (key, supplied, value) in [
                (
                    "file_steps",
                    config.runtime.supplied.file_steps,
                    config.runtime.file_steps,
                ),
                (
                    "file_native_bytes",
                    config.runtime.supplied.file_native_bytes,
                    config.runtime.file_native_bytes,
                ),
                (
                    "repository_steps",
                    config.runtime.supplied.repository_steps,
                    config.runtime.repository_steps,
                ),
                (
                    "repository_native_bytes",
                    config.runtime.supplied.repository_native_bytes,
                    config.runtime.repository_native_bytes,
                ),
                (
                    "worker_memory_bytes",
                    config.runtime.supplied.worker_memory_bytes,
                    config.runtime.worker_memory_bytes,
                ),
                (
                    "parent_memory_bytes",
                    config.runtime.supplied.parent_memory_bytes,
                    config.runtime.parent_memory_bytes,
                ),
                (
                    "total_memory_bytes",
                    config.runtime.supplied.total_memory_bytes,
                    config.runtime.total_memory_bytes,
                ),
            ] {
                if supplied {
                    match key {
                        "file_steps" => runtime.file_steps = value,
                        "file_native_bytes" => runtime.file_native_bytes = value,
                        "repository_steps" => runtime.repository_steps = value,
                        "repository_native_bytes" => runtime.repository_native_bytes = value,
                        "worker_memory_bytes" => runtime.worker_memory_bytes = value,
                        "parent_memory_bytes" => runtime.parent_memory_bytes = value,
                        "total_memory_bytes" => runtime.total_memory_bytes = value,
                        _ => unreachable!("known runtime setting"),
                    }
                    origins.push(ValueOrigin {
                        key: format!("runtime.{key}"),
                        origin: origin.to_owned(),
                    });
                }
            }
            if config.optimizer.supplied {
                optimizer.mode.clone_from(&config.optimizer.mode);
                origins.push(ValueOrigin {
                    key: "optimizer.mode".to_owned(),
                    origin: origin.to_owned(),
                });
            }
            overrides.extend(config.rules.mode_overrides.clone());
            expectations.extend(
                config
                    .coverage
                    .expectations
                    .iter()
                    .cloned()
                    .map(|expectation| EffectiveExpectation {
                        expectation,
                        origin: name.to_owned(),
                    }),
            );
            for key in config.rules.mode_overrides.keys() {
                origins.push(ValueOrigin {
                    key: format!("rules.mode_overrides.{key}"),
                    origin: name.to_owned(),
                });
            }
        }
    }
    if let Some(value) = options.get("max_file_bytes") {
        let max = value
            .as_u64()
            .ok_or_else(|| anyhow::anyhow!("max_file_bytes must be an unsigned integer"))?;
        scan.max_file_bytes = max;
        origins.push(ValueOrigin {
            key: "scan.max_file_bytes".to_owned(),
            origin: "cli".to_owned(),
        });
    }
    if let Some(value) = options.get("optimizer") {
        optimizer.mode = value
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("optimizer must be auto or off"))?
            .to_owned();
        origins.push(ValueOrigin {
            key: "optimizer.mode".to_owned(),
            origin: "cli".to_owned(),
        });
    }
    validate(&ConfigFile {
        schema_version: crate::CONTRACT_VERSION,
        scan: scan.clone(),
        runtime,
        optimizer: optimizer.clone(),
        rules: RulesConfig {
            mode_overrides: overrides.clone(),
        },
        coverage: CoverageConfig {
            expectations: expectations
                .iter()
                .map(|entry| entry.expectation.clone())
                .collect(),
        },
    })?;
    Ok(EffectiveConfig {
        schema_version: crate::CONTRACT_VERSION,
        scan,
        runtime,
        optimizer,
        mode_overrides: overrides,
        coverage: CoverageConfig {
            expectations: expectations
                .iter()
                .map(|entry| entry.expectation.clone())
                .collect(),
        },
        expectations,
        origins,
        inactive_overrides: Vec::new(),
    })
}

fn apply_scan(
    target: &mut ScanConfig,
    origins: &mut Vec<ValueOrigin>,
    source: &ScanConfig,
    origin: &str,
) {
    if source.supplied.respect_gitignore {
        target.respect_gitignore = source.respect_gitignore;
        origins.push(ValueOrigin {
            key: "scan.respect_gitignore".to_owned(),
            origin: origin.to_owned(),
        });
    }
    if source.supplied.honor_git_local_excludes {
        target.honor_git_local_excludes = source.honor_git_local_excludes;
        origins.push(ValueOrigin {
            key: "scan.honor_git_local_excludes".to_owned(),
            origin: origin.to_owned(),
        });
    }
    if source.supplied.include_hidden {
        target.include_hidden = source.include_hidden;
        origins.push(ValueOrigin {
            key: "scan.include_hidden".to_owned(),
            origin: origin.to_owned(),
        });
    }
    if source.supplied.max_file_bytes {
        target.max_file_bytes = source.max_file_bytes;
        origins.push(ValueOrigin {
            key: "scan.max_file_bytes".to_owned(),
            origin: origin.to_owned(),
        });
    }
    if source.supplied.exclude {
        target.exclude.extend(source.exclude.clone());
        origins.push(ValueOrigin {
            key: "scan.exclude".to_owned(),
            origin: origin.to_owned(),
        });
    }
}

pub fn effective_value(config: &EffectiveConfig) -> Value {
    serde_json::to_value(config).expect("effective configuration serialization")
}

pub fn global_dir(explicit: Option<&str>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        let path = PathBuf::from(path);
        return if path.is_absolute() {
            Ok(path)
        } else {
            Ok(std::env::current_dir()?.join(path))
        };
    }
    if let Some(path) = std::env::var_os("WT_CONFIG_HOME") {
        return absolute_path(PathBuf::from(path), "WT_CONFIG_HOME");
    }
    #[cfg(unix)]
    {
        if let Some(path) = std::env::var_os("XDG_CONFIG_HOME") {
            let path = PathBuf::from(path);
            if !path.is_absolute() {
                bail!("XDG_CONFIG_HOME must be an absolute path")
            }
            return Ok(path.join("wt"));
        }
    }
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        return Ok(PathBuf::from(home).join(".config/wt"));
    }
    bail!("unable to resolve global configuration directory")
}

fn absolute_path(path: PathBuf, name: &str) -> Result<PathBuf> {
    if !path.is_absolute() {
        bail!("{name} must be an absolute path")
    }
    Ok(path)
}
