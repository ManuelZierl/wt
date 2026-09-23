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
    pub rules: RulesConfig,
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
    #[serde(default)]
    rules: RulesConfig,
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
        Ok(Self {
            schema_version: raw.schema_version,
            scan,
            rules: raw.rules,
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
    pub mode_overrides: std::collections::BTreeMap<String, ModeOverride>,
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
    if config.schema_version != 2 {
        bail!("configuration schema_version must be 2")
    }
    validate(&config)?;
    Ok(Some(config))
}

pub fn validate(config: &ConfigFile) -> Result<()> {
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
    Ok(())
}

pub fn merge(
    global: Option<&ConfigFile>,
    local: Option<&ConfigFile>,
    options: &Value,
) -> Result<EffectiveConfig> {
    let mut scan = ScanConfig::default();
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
    for (name, config, origin) in [("global", global, "global"), ("local", local, "local")] {
        if let Some(config) = config {
            apply_scan(&mut scan, &mut origins, &config.scan, origin);
            overrides.extend(config.rules.mode_overrides.clone());
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
    validate(&ConfigFile {
        schema_version: 2,
        scan: scan.clone(),
        rules: RulesConfig {
            mode_overrides: overrides.clone(),
        },
    })?;
    Ok(EffectiveConfig {
        schema_version: 2,
        scan,
        mode_overrides: overrides,
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
