//! Repository-owned decisions about occurrences. Detection and raw-result
//! caches never consult this state. Acceptances require current evidence.
use crate::digest::{digest_bytes, semantic_digest};
use crate::parse_json;
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const MAX_TEXT: usize = 128 * 1024;
const MAX_DECISION: usize = 64 * 1024;
const MAX_RECORDS: usize = 10_000;
const MAX_LOADED_BYTES: usize = 16 * 1024 * 1024;
const MAX_EVIDENCE_FILE: usize = 64 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    NeedsReview,
    Acceptable,
    ConfirmedIssue,
    AcceptedRisk,
}
impl Decision {
    fn accepts(&self) -> bool {
        matches!(self, Self::Acceptable | Self::AcceptedRisk)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub schema_version: u64,
    pub finding_id: String,
    pub evidence_digest: String,
    pub decision: Decision,
    pub rule_id: String,
    pub code: String,
    pub path: String,
    pub start_byte: u64,
    pub end_byte: u64,
    pub matched_digest: String,
    pub rule_digest: String,
    pub file_digest: String,
    pub context_digest: String,
    pub engine_digest: String,
    pub watched_files: BTreeMap<String, String>,
    pub rationale_file: String,
    pub previous: Option<String>,
}

pub struct Stored {
    record: Record,
    digest: String,
    rationale: String,
    revision: u32,
}
#[derive(Default)]
pub struct Store {
    records: BTreeMap<String, Stored>,
}

/// Identity is deliberately conservative: no fuzzy matching or automatic
/// acceptance transfer after moves/copies. Evidence separately binds context.
pub fn decorate(finding: &mut Value, matched_digest: &str) -> Result<()> {
    finding["matched_digest"] = json!(matched_digest);
    let identity = json!([
        finding["rule_id"],
        finding["code"],
        finding["path"],
        finding["start_byte"],
        finding["end_byte"],
        matched_digest
    ]);
    let id = semantic_digest("WT-FINDING-1", &[&serde_json::to_vec(&identity)?]);
    let evidence = semantic_digest(
        "WT-REVIEW-EVIDENCE-1",
        &[
            id.as_bytes(),
            string(finding, "rule_digest")?.as_bytes(),
            string(finding, "file_digest")?.as_bytes(),
            string(finding, "context_digest")?.as_bytes(),
            string(finding, "engine_digest")?.as_bytes(),
        ],
    );
    finding["finding_id"] = json!(id);
    finding["evidence_digest"] = json!(evidence);
    Ok(())
}

pub fn load(root: &Path) -> Result<Store> {
    let directory = safe_path(root, ".wt/reviews", true)?;
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Store::default()),
        Err(error) => return Err(error.into()),
    };
    let mut store = Store::default();
    let mut loaded_bytes = 0;
    for entry in entries {
        let entry = entry?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| anyhow!("non-UTF-8 review path"))?;
        if store.records.len() >= MAX_RECORDS {
            bail!("review record limit exceeded");
        }
        let id = format!("sha256:{name}");
        if !valid_digest(&id) || !entry.file_type()?.is_dir() {
            bail!("invalid review directory {name:?}");
        }
        if let Some(stored) = load_latest(root, &name)? {
            loaded_bytes += stored.rationale.len() + serde_json::to_vec(&stored.record)?.len();
            if loaded_bytes > MAX_LOADED_BYTES {
                bail!("review store memory limit exceeded");
            }
            if stored.record.finding_id != id {
                bail!("review directory identity mismatch");
            }
            store.records.insert(id, stored);
        }
    }
    Ok(store)
}

fn load_latest(root: &Path, name: &str) -> Result<Option<Stored>> {
    let relative = format!(".wt/reviews/{name}");
    let directory = safe_path(root, &relative, false)?;
    let mut revisions = Vec::new();
    for entry in fs::read_dir(&directory)? {
        let entry = entry?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| anyhow!("invalid revision name"))?;
        if name.len() != 8
            || !name.bytes().all(|b| b.is_ascii_digit())
            || !entry.file_type()?.is_dir()
        {
            bail!("invalid review revision {name:?}");
        }
        revisions.push(name.parse::<u32>()?);
        if revisions.len() > 10_000 {
            bail!("review history limit exceeded");
        }
    }
    revisions.sort_unstable();
    if revisions.is_empty() {
        bail!("review history has no complete revision");
    }
    for (index, revision) in revisions.iter().enumerate() {
        if *revision != index as u32 + 1 {
            bail!("review history has a missing revision");
        }
    }
    let mut latest = None;
    let mut previous_digest: Option<String> = None;
    let finding_id = format!("sha256:{name}");
    for revision in revisions {
        let base = format!("{relative}/{revision:08}");
        let bytes = read_bounded(
            &safe_path(root, &format!("{base}/decision.json"), false)?,
            MAX_DECISION,
        )?;
        let record: Record = serde_json::from_value(parse_json(std::str::from_utf8(&bytes)?)?)?;
        validate(&record)?;
        if record.finding_id != finding_id {
            bail!("review directory identity mismatch");
        }
        let rationale_bytes = read_bounded(
            &safe_path(root, &format!("{base}/rationale.md"), false)?,
            MAX_TEXT,
        )?;
        let rationale = String::from_utf8(rationale_bytes)?;
        if rationale.trim().is_empty() {
            bail!("review rationale must not be empty");
        }
        if record.previous.as_deref() != previous_digest.as_deref() {
            bail!("review history digest link mismatch at revision {revision}");
        }
        let digest = record_digest(&bytes, rationale.as_bytes());
        previous_digest = Some(digest.clone());
        latest = Some(Stored {
            record,
            digest,
            rationale,
            revision,
        });
    }
    Ok(latest)
}

fn validate(record: &Record) -> Result<()> {
    if record.schema_version != crate::CONTRACT_VERSION
        || record.rationale_file != "rationale.md"
        || record.rule_id.is_empty()
        || record.code.is_empty()
        || record.start_byte > record.end_byte
        || record.watched_files.len() > 32
    {
        bail!("invalid review record");
    }
    relative(&record.path)?;
    for digest in [
        &record.finding_id,
        &record.evidence_digest,
        &record.rule_digest,
        &record.file_digest,
        &record.matched_digest,
        &record.context_digest,
        &record.engine_digest,
    ] {
        if !valid_digest(digest) {
            bail!("invalid review digest");
        }
    }
    if record.previous.as_ref().is_some_and(|d| !valid_digest(d)) {
        bail!("invalid previous review digest");
    }
    for (path, digest) in &record.watched_files {
        relative(path)?;
        if !valid_digest(digest) {
            bail!("invalid watched-file digest");
        }
    }
    // Recompute identity and evidence; hand-edited inconsistent records fail closed.
    let mut expected = json!({"rule_id":record.rule_id,"code":record.code,"path":record.path,
        "start_byte":record.start_byte,"end_byte":record.end_byte,
        "rule_digest":record.rule_digest,"file_digest":record.file_digest,
        "context_digest":record.context_digest,"engine_digest":record.engine_digest});
    decorate(&mut expected, &record.matched_digest)?;
    if expected["finding_id"] != record.finding_id
        || expected["evidence_digest"] != record.evidence_digest
    {
        bail!("review identity/evidence is inconsistent");
    }
    Ok(())
}

pub struct Application {
    pub diagnostics: Vec<Value>,
    pub reviewed: Vec<Value>,
    pub records: Vec<Value>,
    pub evidence_reads: usize,
}

pub fn apply(root: &Path, diagnostics: Vec<Value>, store: &Store) -> Result<Application> {
    let mut result = Application {
        diagnostics: Vec::new(),
        reviewed: Vec::new(),
        records: Vec::new(),
        evidence_reads: 0,
    };
    let mut observed = BTreeSet::new();
    let mut evidence = BTreeMap::<String, Option<String>>::new();
    for mut finding in diagnostics {
        let id = string(&finding, "finding_id")?.to_owned();
        if !observed.insert(id.clone()) {
            bail!("ambiguous duplicate finding identity");
        }
        let Some(stored) = store.records.get(&id) else {
            finding["review_state"] = json!({"decision":"needs_review", "validity":"unreviewed"});
            result.diagnostics.push(finding);
            continue;
        };
        let mut current = true;
        let mut reasons = Vec::new();
        for (field, previous, reason) in [
            (
                "rule_digest",
                &stored.record.rule_digest,
                "rule_package_changed",
            ),
            (
                "file_digest",
                &stored.record.file_digest,
                "owner_file_changed",
            ),
            (
                "engine_digest",
                &stored.record.engine_digest,
                "runtime_semantics_changed",
            ),
        ] {
            if string(&finding, field)? != previous {
                current = false;
                reasons.push(reason.to_owned());
            }
        }
        if string(&finding, "context_digest")? != stored.record.context_digest {
            current = false;
            if string(&finding, "file_digest")? == stored.record.file_digest {
                reasons.push("repository_input_inventory_changed".to_owned());
            }
        }
        if string(&finding, "evidence_digest")? != stored.record.evidence_digest {
            current = false;
        }
        for (path, expected) in &stored.record.watched_files {
            if !evidence.contains_key(path) {
                let actual = match file_digest(root, path) {
                    Ok(digest) => Some(digest),
                    Err(error)
                        if error
                            .downcast_ref::<std::io::Error>()
                            .is_some_and(|io| io.kind() == std::io::ErrorKind::NotFound) =>
                    {
                        None
                    }
                    Err(error) => {
                        return Err(error.context(format!("unable to evaluate watched file {path}")))
                    }
                };
                evidence.insert(path.clone(), actual);
            }
            let actual = &evidence[path];
            if actual.as_ref() != Some(expected) {
                current = false;
                reasons.push(format!(
                    "{}:{path}",
                    if actual.is_some() {
                        "watched_file_changed"
                    } else {
                        "watched_file_missing"
                    }
                ));
            }
        }
        // An 'acceptable' decision is for review triggers, never a bypass for a
        // rule that now declares a definite violation.
        if matches!(stored.record.decision, Decision::Acceptable) && finding["kind"] != "review" {
            current = false;
            reasons.push("acceptable_requires_review_diagnostic".to_owned());
        }
        finding["review_state"] = json!({"decision":stored.record.decision,
            "validity":if current {"current"} else {"stale"}, "record_digest":stored.digest,
            "revision":stored.revision, "reasons":reasons});
        result
            .records
            .push(json!({"finding_id":id,"decision":stored.record.decision,
            "validity":if current {"current"} else {"stale"}, "record_digest":stored.digest,
            "revision":stored.revision}));
        if current && stored.record.decision.accepts() {
            finding["blocking"] = json!(false);
            result.reviewed.push(finding);
        } else {
            result.diagnostics.push(finding);
        }
    }
    for (id, stored) in &store.records {
        if observed.contains(id) {
            continue;
        }
        let validity = "not_observed";
        result
            .records
            .push(json!({"finding_id":id,"decision":stored.record.decision,
            "validity":validity,"record_digest":stored.digest,"revision":stored.revision}));
    }
    result
        .records
        .sort_by_key(|v| v["finding_id"].as_str().unwrap_or_default().to_owned());
    result.evidence_reads = evidence.len();
    Ok(result)
}

pub fn list(root: &Path, finding_id: Option<&str>) -> Result<Value> {
    let store = load(root)?;
    if finding_id.is_some_and(|id| !store.records.contains_key(id)) {
        bail!("unknown reviewed finding ID {}", finding_id.unwrap());
    }
    Ok(
        json!({"schema_version":crate::CONTRACT_VERSION,"command":"reviews","status":"pass","exit_code":0,
        "reviews":store.records.iter().filter(|(id,_)| finding_id.is_none_or(|wanted| id.as_str()==wanted)).map(|(_,s)| json!({"record":s.record,
            "record_digest":s.digest,"revision":s.revision,"rationale":s.rationale,
            "validity":"not_evaluated"})).collect::<Vec<_>>() }),
    )
}

pub fn stored_detail(root: &Path, id: &str) -> Result<Option<Value>> {
    let store = load(root)?;
    Ok(store.records.get(id).map(|stored| {
        json!({
            "record": stored.record, "record_digest": stored.digest,
            "revision": stored.revision, "rationale": stored.rationale
        })
    }))
}

/// Persist exactly one explicit decision, with optimistic concurrency and
/// immutable numbered history. No command automatically accepts findings.
pub fn record(
    root: &Path,
    finding: &Value,
    options: &Value,
    revalidate: impl FnOnce() -> Result<()>,
) -> Result<Value> {
    let expected = string(options, "expect_evidence")?;
    if string(finding, "evidence_digest")? != expected {
        bail!("stale finding evidence; run check and review again");
    }
    let decision: Decision = serde_json::from_value(options["decision"].clone())?;
    if matches!(decision, Decision::Acceptable) && finding["kind"] != "review" {
        bail!(
            "acceptable is only for review diagnostics; a deliberate violation needs accepted_risk"
        );
    }
    let rationale = string(options, "rationale")?;
    if rationale.trim().is_empty() || rationale.len() > MAX_TEXT {
        bail!("rationale must be nonempty and at most 128 KiB");
    }
    let id = string(finding, "finding_id")?;
    if !valid_digest(id) {
        bail!("invalid finding ID");
    }
    let mut watched = BTreeMap::new();
    if let Some(watch) = options.get("watch") {
        let values = watch
            .as_array()
            .ok_or_else(|| anyhow!("watch must be an array of PATH=sha256:HASH"))?;
        for value in values {
            let pair = value
                .as_str()
                .ok_or_else(|| anyhow!("watch must be PATH=sha256:HASH"))?;
            let (path, expected) = pair
                .rsplit_once('=')
                .ok_or_else(|| anyhow!("watch must include expected digest: PATH=sha256:HASH"))?;
            relative(path)?;
            if !valid_digest(expected)
                || watched
                    .insert(path.to_owned(), expected.to_owned())
                    .is_some()
            {
                bail!("invalid or duplicate watched evidence");
            }
        }
    }
    let store_path = safe_path(root, ".wt/reviews", true)?;
    fs::create_dir_all(&store_path)?;
    let name = &id[7..];
    let lock_path = crate::coordination::lock_path(
        "review",
        &format!("{}:{name}", root.canonicalize()?.display()),
    )?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)?;
    lock.try_lock().context("review is locked")?;
    let occurrence = safe_path(root, &format!(".wt/reviews/{name}"), true)?;
    let existing = if occurrence.exists() {
        load_latest(root, name)?
    } else {
        None
    };
    let expected_record = options.get("expect_hash").and_then(Value::as_str);
    match (&existing, expected_record) {
        (Some(old), Some(hash)) if old.digest == hash => {}
        (None, None) => {}
        _ => bail!("stale or missing review hash; inspect wt reviews before replacing a decision"),
    }
    let record = Record {
        schema_version: crate::CONTRACT_VERSION,
        finding_id: id.to_owned(),
        evidence_digest: expected.to_owned(),
        decision,
        rule_id: string(finding, "rule_id")?.to_owned(),
        code: string(finding, "code")?.to_owned(),
        path: string(finding, "path")?.to_owned(),
        start_byte: finding["start_byte"]
            .as_u64()
            .ok_or_else(|| anyhow!("invalid finding span"))?,
        end_byte: finding["end_byte"]
            .as_u64()
            .ok_or_else(|| anyhow!("invalid finding span"))?,
        matched_digest: string(finding, "matched_digest")?.to_owned(),
        rule_digest: string(finding, "rule_digest")?.to_owned(),
        file_digest: string(finding, "file_digest")?.to_owned(),
        context_digest: string(finding, "context_digest")?.to_owned(),
        engine_digest: string(finding, "engine_digest")?.to_owned(),
        watched_files: watched,
        rationale_file: "rationale.md".to_owned(),
        previous: existing.as_ref().map(|s| s.digest.clone()),
    };
    validate(&record)?;
    revalidate()?;
    if file_digest(root, &record.path)? != record.file_digest {
        bail!("owning file changed after check");
    }
    for (path, expected) in &record.watched_files {
        if file_digest(root, path)? != *expected {
            bail!("watched file changed: {path}");
        }
    }
    let bytes = serde_json::to_vec_pretty(&record)?;
    let digest = record_digest(&bytes, rationale.as_bytes());
    let revision = existing.as_ref().map_or(1, |s| s.revision + 1);
    if revision > 10_000 {
        bail!("review history limit exceeded");
    }
    fs::create_dir_all(&occurrence)?;
    let temp = tempfile::Builder::new()
        .prefix(".pending-")
        .tempdir_in(&occurrence)?;
    write_synced(&temp.path().join("decision.json"), &bytes)?;
    write_synced(&temp.path().join("rationale.md"), rationale.as_bytes())?;
    let target = occurrence.join(format!("{revision:08}"));
    if target.exists() {
        bail!("review revision already exists");
    }
    fs::rename(temp.path(), &target)?;
    Ok(
        json!({"schema_version":crate::CONTRACT_VERSION,"command":"review","status":"pass","exit_code":0,
        "finding_id":id,"record_digest":digest,"revision":revision,"path":target,"decision":record.decision}),
    )
}

fn write_synced(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}
fn record_digest(record: &[u8], rationale: &[u8]) -> String {
    semantic_digest("WT-REVIEW-RECORD-1", &[record, rationale])
}
fn valid_digest(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
fn string<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value[key].as_str().ok_or_else(|| anyhow!("missing {key}"))
}
fn relative(path: &str) -> Result<()> {
    if path.is_empty()
        || path.contains(['\\', ':'])
        || path
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == "..")
    {
        bail!("unsafe root-relative review path {path:?}");
    }
    Ok(())
}

fn safe_path(root: &Path, path: &str, allow_missing: bool) -> Result<PathBuf> {
    relative(path)?;
    let mut current = root.to_path_buf();
    for component in path.split('/') {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(meta) if meta.file_type().is_symlink() => {
                bail!("review/evidence paths must not use symlinks")
            }
            Ok(_) => {}
            Err(error) if allow_missing && error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(current)
}
fn read_bounded(path: &Path, max: usize) -> Result<Vec<u8>> {
    let file = File::open(path)?;
    let before = file.metadata()?;
    if !before.is_file() {
        bail!("review input must be a regular file");
    }
    let mut bytes = Vec::new();
    file.take((max + 1) as u64).read_to_end(&mut bytes)?;
    if bytes.len() > max {
        bail!("review input exceeds size limit");
    }
    let after = fs::metadata(path)?;
    if before.len() != after.len() || before.modified()? != after.modified()? {
        bail!("review evidence changed while reading");
    }
    Ok(bytes)
}
fn file_digest(root: &Path, path: &str) -> Result<String> {
    Ok(digest_bytes(&read_bounded(
        &safe_path(root, path, false)?,
        MAX_EVIDENCE_FILE,
    )?))
}

#[cfg(test)]
mod golden_vectors {
    use super::*;

    #[test]
    fn pinned_finding_evidence_and_record_digest_framing() {
        let mut finding = json!({"rule_id":"local/vector","code":"hit","path":"src/a.txt",
            "start_byte":0,"end_byte":3,"rule_digest":format!("sha256:{}", "1".repeat(64)),
            "file_digest":format!("sha256:{}", "2".repeat(64)),
            "context_digest":format!("sha256:{}", "3".repeat(64)),
            "engine_digest":format!("sha256:{}", "4".repeat(64))});
        decorate(&mut finding, &format!("sha256:{}", "0".repeat(64))).unwrap();
        assert_eq!(
            finding["finding_id"],
            "sha256:109740956a93be6e9d49fe92d252a3494bd0ddaa1d8937357c5a3ca5cbfa9a0f"
        );
        assert_eq!(
            finding["evidence_digest"],
            "sha256:02eacd4c8a1a066d526eca74010b3846f21079f4d109c9b3c69d36cf5d24878b"
        );
        assert_eq!(
            record_digest(br#"{"schema_version":1}"#, b"# Reviewed\n"),
            "sha256:33d87a000036666295070847b760e97e496d2672ed2ed0a2d730982ce4b900dc"
        );
    }
}
