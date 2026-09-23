use crate::config::{self, EffectiveConfig};
use crate::rule::{load_package, RulePackage};
use anyhow::{bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone)]
pub struct Workspace {
    pub root: PathBuf,
    pub local_dir: PathBuf,
    pub effective_config: EffectiveConfig,
    pub packages: Vec<RulePackage>,
}

pub fn discover(options: &serde_json::Value) -> Result<Workspace> {
    let root = resolve_root(options.get("root").and_then(|value| value.as_str()))?;
    let no_global = options
        .get("no_global")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let global_dir = if no_global {
        None
    } else {
        Some(config::global_dir(
            options.get("global_dir").and_then(|value| value.as_str()),
        )?)
    };
    let global_config = global_dir
        .as_ref()
        .map(|path| config::load(&path.join("config.json")))
        .transpose()?
        .flatten();
    let local_dir = root.join(".wt");
    let local_config = config::load(&local_dir.join("config.json"))?;
    let mut effective_config =
        config::merge(global_config.as_ref(), local_config.as_ref(), options)?;
    if no_global {
        effective_config.inactive_overrides = effective_config
            .mode_overrides
            .keys()
            .filter(|id| id.starts_with("global/"))
            .cloned()
            .collect();
    }
    let mut packages = Vec::new();
    if let Some(global_dir) = global_dir {
        packages.extend(load_scope(&global_dir.join("rules"), "global")?);
    }
    packages.extend(load_scope(&local_dir.join("rules"), "local")?);
    validate_overrides(&effective_config, &packages, no_global)?;
    Ok(Workspace {
        root,
        local_dir,
        effective_config,
        packages,
    })
}

pub fn resolve_root(explicit: Option<&str>) -> Result<PathBuf> {
    if let Some(value) = explicit {
        let path = PathBuf::from(value);
        let path = if path.is_absolute() {
            path
        } else {
            std::env::current_dir()?.join(path)
        };
        return path
            .canonicalize()
            .with_context(|| format!("unable to resolve root {}", path.display()));
    }
    if let Some(root) = git_root() {
        return Ok(root);
    }
    let mut current = std::env::current_dir()?.canonicalize()?;
    loop {
        if current.join(".wt").is_dir() {
            return Ok(current);
        }
        if !current.pop() {
            break;
        }
    }
    std::env::current_dir()?.canonicalize().map_err(Into::into)
}

fn git_root() -> Option<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?;
    PathBuf::from(value.trim()).canonicalize().ok()
}

fn load_scope(rules_dir: &Path, scope: &str) -> Result<Vec<RulePackage>> {
    if !rules_dir.exists() {
        return Ok(Vec::new());
    }
    let mut entries = fs::read_dir(rules_dir)
        .with_context(|| format!("unable to read {}", rules_dir.display()))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    let mut packages = Vec::new();
    for entry in entries {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if let Some(id) = name
            .strip_prefix('.')
            .and_then(|name| name.strip_suffix(".backup"))
        {
            if crate::rule::valid_rule_id(id) && !rules_dir.join(id).exists() {
                bail!(
                    "interrupted rule update: restore {} to {} before checking",
                    path.display(),
                    rules_dir.join(id).display()
                );
            }
        }
        if name.starts_with('.') {
            continue;
        }
        if !entry.file_type()?.is_dir() {
            continue;
        }
        packages.push(load_package(&path, scope)?);
    }
    Ok(packages)
}

fn validate_overrides(
    config: &EffectiveConfig,
    packages: &[RulePackage],
    no_global: bool,
) -> Result<()> {
    for id in config.mode_overrides.keys() {
        if no_global && id.starts_with("global/") {
            continue;
        }
        if !packages.iter().any(|package| package.qualified_id == *id) {
            bail!("mode override targets unknown rule {id}")
        }
    }
    Ok(())
}

pub fn select_packages<'a>(
    packages: &'a [RulePackage],
    requested: Option<&[String]>,
) -> Result<Vec<&'a RulePackage>> {
    let Some(requested) = requested else {
        return Ok(packages.iter().collect());
    };
    let mut selected = Vec::new();
    for id in requested {
        let matches = packages
            .iter()
            .filter(|package| package.qualified_id == *id || package.manifest.id == *id)
            .collect::<Vec<_>>();
        if matches.is_empty() {
            bail!("unknown rule {id}")
        }
        if !id.contains('/') && matches.len() > 1 {
            let candidates = matches
                .iter()
                .map(|package| package.qualified_id.as_str())
                .collect::<Vec<_>>();
            bail!("ambiguous rule {id}; candidates: {}", candidates.join(", "))
        }
        let package = matches[0];
        if !selected
            .iter()
            .any(|item: &&RulePackage| item.qualified_id == package.qualified_id)
        {
            selected.push(package);
        }
    }
    Ok(selected)
}
