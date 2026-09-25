use crate::parse_json;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Read;
use std::path::Path;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WaiverFile {
    pub schema_version: u64,
    pub waivers: Vec<Waiver>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Waiver {
    pub id: String,
    pub rule_id: String,
    pub code: String,
    pub path: String,
    pub matched_text_digest: String,
    pub reason: String,
    #[serde(default)]
    pub occurrence: Option<usize>,
}

pub struct WaiverApplication {
    pub suppressed: Vec<String>,
    pub suppressed_indices: Vec<usize>,
    pub stale: Vec<String>,
}

pub fn load(path: &Path) -> Result<Option<WaiverFile>> {
    if !path.exists() {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .with_context(|| format!("unable to read {}", path.display()))?
        .take(4 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 4 * 1024 * 1024 {
        bail!("waivers exceed 4 MiB")
    }
    let value = parse_json(std::str::from_utf8(&bytes)?)?;
    let file: WaiverFile = serde_json::from_value(value).context("invalid waivers")?;
    if file.schema_version != crate::CONTRACT_VERSION {
        bail!("waivers schema_version must be {}", crate::CONTRACT_VERSION)
    }
    let mut ids = std::collections::HashSet::new();
    for waiver in &file.waivers {
        if waiver.id.trim().is_empty()
            || !ids.insert(&waiver.id)
            || !waiver.rule_id.split_once('/').is_some_and(|(scope, id)| {
                matches!(scope, "local" | "global") && crate::rule::valid_rule_id(id)
            })
            || waiver.code.trim().is_empty()
            || !crate::rule::safe_relative(&waiver.path)
            || waiver.matched_text_digest.len() != 71
            || !waiver.matched_text_digest.starts_with("sha256:")
            || !waiver.matched_text_digest.as_bytes()[7..]
                .iter()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
            || waiver.reason.trim().is_empty()
        {
            bail!("invalid waiver {}", waiver.id)
        }
    }
    Ok(Some(file))
}

pub fn apply<F>(
    waivers: Option<&WaiverFile>,
    rule_id: &str,
    code: &str,
    path: &str,
    matched_digests: &[String],
    mut keep: F,
) -> Result<WaiverApplication>
where
    F: FnMut(usize),
{
    let Some(waivers) = waivers else {
        for index in 0..matched_digests.len() {
            keep(index);
        }
        return Ok(WaiverApplication {
            suppressed: Vec::new(),
            suppressed_indices: Vec::new(),
            stale: Vec::new(),
        });
    };
    let mut suppressed = Vec::new();
    let mut used = std::collections::HashSet::new();
    let mut suppressed_indices = Vec::new();
    for waiver in &waivers.waivers {
        if waiver.rule_id != rule_id || waiver.code != code || waiver.path != path {
            continue;
        }
        let candidates = matched_digests
            .iter()
            .enumerate()
            .filter(|(_, digest)| **digest == waiver.matched_text_digest)
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        let chosen = if let Some(occurrence) = waiver.occurrence {
            candidates
                .get(occurrence)
                .copied()
                .into_iter()
                .collect::<Vec<_>>()
        } else if candidates.len() == 1 {
            candidates.clone()
        } else if candidates.is_empty() {
            Vec::new()
        } else {
            bail!("waiver {} ambiguously matches diagnostics", waiver.id)
        };
        if chosen.is_empty() {
            continue;
        }
        for index in chosen {
            used.insert(index);
            suppressed_indices.push(index);
        }
        suppressed.push(waiver.id.clone());
    }
    for index in 0..matched_digests.len() {
        if !used.contains(&index) {
            keep(index);
        }
    }
    let stale = waivers
        .waivers
        .iter()
        .filter(|waiver| waiver.rule_id == rule_id && waiver.code == code && waiver.path == path)
        .filter(|waiver| !suppressed.contains(&waiver.id))
        .map(|waiver| waiver.id.clone())
        .collect();
    Ok(WaiverApplication {
        suppressed,
        suppressed_indices,
        stale,
    })
}
