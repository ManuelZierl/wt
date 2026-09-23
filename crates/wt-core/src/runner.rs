use crate::cache;
use crate::config;
use crate::digest::semantic_digest;
use crate::discovery::{self, Workspace};
use crate::fixtures;
use crate::reviews;
use crate::rule::{self, RulePackage, Submission};
use crate::selection::{self, SelectedFile};
use crate::waivers;
use anyhow::{anyhow, bail, Result};
use globset::{GlobBuilder, GlobSetBuilder};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::Path;
use wt_runtime::{QueryArena, RawDiagnostic, SourceFile};

const COMMON_OPTIONS: &[&str] = &[
    "root",
    "global_dir",
    "format",
    "color",
    "_worker_executable",
];

pub fn dispatch(command: &str, options: &Value) -> Result<Value> {
    validate_options(command, options)?;
    match command {
        "fmt" => format_rules(options),
        "review" => review(options),
        "reviews" => reviews::list(&discovery::resolve_root(
            options.get("root").and_then(Value::as_str),
        )?),
        "init" => init(options),
        "new" => create(options),
        "update" => update(options),
        "check" => check(options),
        "plan" => plan(options),
        "list" => list(options),
        "show" => show(options),
        "validate" => validate(options),
        "test" => test(options),
        "set-mode" => set_mode(options),
        "explain" => explain(options),
        "config" => show_config(options),
        "schema" => schema(options),
        "cache-clear" | "cache" => clear_cache(options),
        _ => bail!("unsupported command {command:?}"),
    }
}

fn validate_options(command: &str, options: &Value) -> Result<()> {
    let object = options
        .as_object()
        .ok_or_else(|| anyhow!("options must be an object"))?;
    let mut allowed = COMMON_OPTIONS.iter().copied().collect::<HashSet<_>>();
    let command_options: &[&str] = match command {
        "fmt" => &["global", "id", "check", "no_global"],
        "review" => &[
            "finding_id",
            "decision",
            "expect_evidence",
            "expect_hash",
            "rationale",
            "watch",
            "rules",
            "no_global",
            "no_host_ignores",
        ],
        "reviews" => &[],
        "init" => &["global"],
        "new" => &["global", "submission"],
        "update" => &[
            "id",
            "submission",
            "expect_hash",
            "allow_test_removal",
            "reason",
        ],
        "check" => &[
            "paths",
            "include_ignored",
            "no_host_ignores",
            "no_global",
            "rules",
            "rule",
            "strict",
            "no_cache",
            "optimizer",
            "changed",
            "base",
            "jobs",
            "max_file_bytes",
            "show_suppressed",
            "allow_empty",
            "stats",
        ],
        "plan" => &["rules", "rule", "no_global", "optimizer"],
        "list" | "config" => &["no_global"],
        "schema" => &["schema", "id"],
        "show" | "validate" | "test" => &["id", "rules", "rule", "no_global", "submission"],
        "set-mode" => &["id", "mode", "reason", "no_global"],
        "explain" => &[
            "paths",
            "id",
            "rule",
            "no_global",
            "include_ignored",
            "no_host_ignores",
        ],
        "cache-clear" | "cache" => &[],
        _ => &[],
    };
    allowed.extend(command_options.iter().copied());
    for key in object.keys() {
        if !allowed.contains(key.as_str()) {
            bail!("option {key:?} is not valid for command {command}")
        }
    }
    if let Some(format) = object.get("format").and_then(Value::as_str) {
        if !matches!(format, "text" | "json") {
            bail!("format must be text or json")
        }
    }
    if let Some(color) = object.get("color").and_then(Value::as_str) {
        if !matches!(color, "auto" | "always" | "never") {
            bail!("color must be auto, always, or never")
        }
    }
    if let Some(jobs) = object.get("jobs") {
        if !jobs.as_u64().is_some_and(|jobs| (1..=3).contains(&jobs)) {
            bail!("jobs must be between 1 and 3 (combined worker memory ceiling)")
        }
    }
    Ok(())
}

fn init(options: &Value) -> Result<Value> {
    let global = bool_option(options, "global", false)?;
    let root = discovery::resolve_root(options.get("root").and_then(Value::as_str))?;
    let target = if global {
        config::global_dir(options.get("global_dir").and_then(Value::as_str))?
    } else {
        root.join(".wt")
    };
    fs::create_dir_all(target.join("rules"))?;
    let config_path = target.join("config.json");
    if !config_path.exists() {
        write_create_new(
            &config_path,
            br#"{
  "schema_version": 2,
  "scan": {
    "respect_gitignore": true,
    "honor_git_local_excludes": true,
    "include_hidden": true,
    "max_file_bytes": 4194304,
    "exclude": []
  },
  "rules": {"mode_overrides": {}}
}
"#,
        )?;
    }
    Ok(envelope(
        "init",
        "pass",
        0,
        json!({"path": target, "scope": if global { "global" } else { "local" }}),
    ))
}

fn create(options: &Value) -> Result<Value> {
    let submission_value = required_submission_value(options)?;
    let submission = rule::prepare_submission(rule::submission_from_value(submission_value)?)?;
    if submission.tests.as_ref().is_some_and(|suite| {
        suite
            .cases
            .iter()
            .flat_map(|case| case.files.iter())
            .any(|file| file.fixture.is_some())
    }) {
        bail!("new submissions may contain inline fixture content only")
    }
    let global = bool_option(options, "global", false)?;
    let root = discovery::resolve_root(options.get("root").and_then(Value::as_str))?;
    let parent = if global {
        config::global_dir(options.get("global_dir").and_then(Value::as_str))?.join("rules")
    } else {
        root.join(".wt/rules")
    };
    fs::create_dir_all(&parent)?;
    let lock = acquire_lock(&parent, &submission.id)?;
    let target = parent.join(&submission.id);
    if target.exists() {
        bail!("rule already exists")
    }
    let replacement = write_replacement(&parent, &submission)?;
    let loaded = rule::load_package(replacement.path(), if global { "global" } else { "local" })?;
    if loaded.manifest.mode == "enforced" && !enforcement_evidence(&loaded) {
        bail!("enforced rule requires applicable positive and nonempty negative fixtures")
    }
    let program = wt_runtime::compile(&loaded.manifest_value, &loaded.source)?;
    if let Some(outcome) = loaded
        .tests
        .as_ref()
        .map(|_| run_fixtures(&loaded, &program, options))
        .transpose()?
    {
        if !outcome.passed {
            bail!("supplied fixture suite failed")
        }
    }
    let result = fs::rename(replacement.path(), &target);
    drop(lock);
    result?;
    let loaded = rule::load_package(&target, if global { "global" } else { "local" })?;
    Ok(envelope(
        "new",
        "pass",
        0,
        json!({
            "qualified_id": loaded.qualified_id,
            "path": loaded.directory,
            "mode": loaded.manifest.mode,
            "digest": loaded.digest,
            "validation": "passed",
            "tests": loaded.tests.as_ref().map(|_| "passed").unwrap_or("untested")
        }),
    ))
}

fn update(options: &Value) -> Result<Value> {
    let id = required_string(options, "id")?;
    let expected = required_string(options, "expect_hash")?;
    let submission = rule::prepare_submission(rule::submission_from_value(
        required_submission_value(options)?,
    )?)?;
    if submission.tests.as_ref().is_some_and(|suite| {
        suite
            .cases
            .iter()
            .flat_map(|case| case.files.iter())
            .any(|file| file.fixture.is_some())
    }) {
        bail!("updates may contain inline fixture content only")
    }
    if submission.id != unqualified_id(&id) {
        bail!("updated submission must retain rule identity")
    }
    let workspace = discovery::discover(options)?;
    let selected =
        discovery::select_packages(&workspace.packages, Some(std::slice::from_ref(&id)))?;
    let package = selected[0];
    let parent = package
        .directory
        .parent()
        .ok_or_else(|| anyhow!("invalid package path"))?;
    let lock = acquire_lock(parent, &submission.id)?;
    let current = rule::load_package(&package.directory, &package.scope_name)?;
    let package = &current;
    if package.digest != expected {
        bail!(
            "stale update hash: expected {expected}, current {}",
            package.digest
        )
    }
    let old_export = rule::export_value(package)?;
    let old_submission = rule::submission_from_value(old_export)?;
    if let Some(old_tests) = &old_submission.tests {
        let new_cases = submission
            .tests
            .as_ref()
            .map(|suite| {
                suite
                    .cases
                    .iter()
                    .map(|case| (case.name.as_str(), case))
                    .collect::<std::collections::HashMap<_, _>>()
            })
            .unwrap_or_default();
        let replaced_case = old_tests.cases.iter().any(|case| {
            new_cases.get(case.name.as_str()).is_none_or(|replacement| {
                serde_json::to_value(case).ok() != serde_json::to_value(replacement).ok()
            })
        });
        if replaced_case {
            if !bool_option(options, "allow_test_removal", false)? {
                bail!("test removal requires allow_test_removal")
            }
            if options
                .get("reason")
                .and_then(Value::as_str)
                .is_none_or(|reason| reason.trim().is_empty())
            {
                bail!("test removal requires a reason")
            }
        }
    }
    let replacement = write_replacement(parent, &submission)?;
    let candidate = rule::load_package(replacement.path(), &package.scope_name)?;
    if candidate.manifest.mode == "enforced" && !enforcement_evidence(&candidate) {
        bail!("enforced rule requires applicable positive and nonempty negative fixtures")
    }
    let program = wt_runtime::compile(&candidate.manifest_value, &candidate.source)?;
    if let Some(outcome) = candidate
        .tests
        .as_ref()
        .map(|_| run_fixtures(&candidate, &program, options))
        .transpose()?
    {
        if !outcome.passed {
            bail!("supplied fixture suite failed")
        }
    }
    let backup = parent.join(format!(".{}.backup", package.manifest.id));
    if backup.exists() {
        bail!(
            "interrupted update backup exists: {}; recover it before updating",
            backup.display()
        )
    }
    fs::rename(&package.directory, &backup)?;
    let result = fs::rename(replacement.path(), &package.directory);
    if result.is_ok() {
        let _ = fs::remove_dir_all(&backup);
    } else {
        let _ = fs::rename(&backup, &package.directory);
    }
    drop(lock);
    result?;
    let loaded = rule::load_package(&package.directory, &package.scope_name)?;
    Ok(envelope(
        "update",
        "pass",
        0,
        json!({"qualified_id": loaded.qualified_id, "path": loaded.directory, "mode": loaded.manifest.mode, "digest": loaded.digest, "validation": "passed", "tests": if loaded.tests.is_some() {"passed"} else {"untested"}}),
    ))
}

fn check(options: &Value) -> Result<Value> {
    let mut workspace = discovery::discover(options)?;
    if bool_option(options, "no_host_ignores", false)? {
        workspace.effective_config.scan.honor_git_local_excludes = false;
    }
    if bool_option(options, "include_ignored", false)? {
        workspace.effective_config.scan.respect_gitignore = false;
    }
    let requested = requested_rules(options)?;
    let selected = discovery::select_packages(&workspace.packages, requested.as_deref())?;
    let enabled = selected
        .iter()
        .copied()
        .filter(|package| effective_mode(&workspace, package) != "disabled")
        .collect::<Vec<_>>();
    let allow_empty = bool_option(options, "allow_empty", false)?;
    if enabled.is_empty() && !allow_empty {
        bail!("no enabled rules; use allow_empty to permit an empty check")
    }
    let changed = options
        .get("changed")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let no_cache = bool_option(options, "no_cache", false)?;
    let mut cache = cache::Store::new(!no_cache);
    let mut programs = Vec::new();
    let mut rule_summaries = Vec::new();
    let mut setup_errors = Vec::new();
    for package in &enabled {
        rule_summaries.push(json!({"id": package.qualified_id, "digest": package.digest, "mode": effective_mode(&workspace, package), "status": "pending"}));
        match wt_runtime::compile(&package.manifest_value, &package.source) {
            Ok(program) => {
                let mut gate_passed = true;
                if package.tests.is_some() {
                    let fixture_key = cache::fixture_key(package);
                    let cached = fixture_key
                        .as_ref()
                        .ok()
                        .and_then(|key| cache.read_fixture(key));
                    match (fixture_key, cached) {
                        (Ok(key), Some(outcome)) => {
                            gate_passed = outcome.passed;
                            if !outcome.passed {
                                setup_errors.push(json!({"rule_id": package.qualified_id, "error": "fixture_failed", "cases": outcome.failed_cases}));
                            }
                            let _ = key;
                        }
                        (Ok(key), None) => match run_fixtures(package, &program, options) {
                            Ok(outcome) => {
                                gate_passed = outcome.passed;
                                cache.write_fixture(&key, &outcome);
                                if !outcome.passed {
                                    setup_errors.push(json!({"rule_id": package.qualified_id, "error": "fixture_failed", "cases": outcome.failed_cases}));
                                }
                            }
                            Err(error) => {
                                gate_passed = false;
                                setup_errors.push(json!({"rule_id": package.qualified_id, "error": error.to_string()}));
                            }
                        },
                        (Err(error), _) => {
                            gate_passed = false;
                            setup_errors.push(json!({"rule_id": package.qualified_id, "error": error.to_string()}));
                        }
                    }
                }
                if effective_mode(&workspace, package) == "enforced"
                    && !enforcement_evidence(package)
                {
                    gate_passed = false;
                    setup_errors.push(json!({"rule_id": package.qualified_id, "error": "enforced_rule_requires_positive_and_negative_tests"}));
                }
                if gate_passed {
                    programs.push((*package, program));
                }
            }
            Err(error) => setup_errors
                .push(json!({"rule_id": package.qualified_id, "error": error.to_string()})),
        }
    }
    let optimized = options.get("optimizer").and_then(Value::as_str) != Some("off");
    let jobs = options
        .get("jobs")
        .and_then(Value::as_u64)
        .map(|n| n as usize)
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |n| n.get().min(3)));
    let pool = options
        .get("_worker_executable")
        .and_then(Value::as_str)
        .map(|path| crate::worker::Pool::new(Path::new(path), jobs))
        .transpose()?;
    let mut execution = FileExecution {
        scopes: programs
            .iter()
            .map(|(package, _)| {
                Ok((
                    package.qualified_id.clone(),
                    PreparedScope::new(&workspace.root, package)?,
                ))
            })
            .collect::<Result<BTreeMap<_, _>>>()?,
        key_prefixes: programs
            .iter()
            .map(|(package, _)| {
                (
                    package.qualified_id.clone(),
                    cache::raw_key(package, "", ""),
                )
            })
            .collect(),
        programs: &programs,
        cache: &mut cache,
        pool,
        arena: QueryArena::new(optimized),
        optimized,
        max_file_bytes: workspace.effective_config.scan.max_file_bytes as usize,
        records: Vec::new(),
        errors: setup_errors,
        relevant_files: HashSet::new(),
        checked: BTreeMap::new(),
        stats: json!({}),
    };
    let mut batch = Vec::new();
    let selection = selection::select_streamed(
        &workspace.root,
        &workspace.effective_config,
        options,
        &enabled,
        |file| {
            batch.push(file.clone());
            if batch.len() >= jobs {
                execution.run_batch(&batch)?;
                batch.clear();
            }
            Ok(())
        },
    )?;
    execution.run_batch(&batch)?;
    let mut records = execution.records;
    let mut errors = execution.errors;
    let mut relevant_files = execution.relevant_files;
    let checked = execution.checked;
    let mut runtime_stats = execution.stats;
    merge_runtime_stats(&mut runtime_stats, &execution.arena.stats());
    let mut pool = execution.pool;
    let mut arena = QueryArena::new(optimized);
    for (package, program) in programs
        .iter()
        .filter(|(package, _)| package.manifest.execution == "repository")
    {
        let applicable = applicable_files(&workspace.root, package, &selection.repository_files)?;
        if applicable.is_empty() {
            continue;
        }
        relevant_files.extend(applicable.iter().map(|file| file.path.clone()));
        if selection
            .gaps
            .iter()
            .any(|gap| scope_accepts_path(&workspace.root, package, &gap.path))
        {
            errors.push(json!({"rule_id": package.qualified_id, "error": "repository_input_gap"}));
            continue;
        }
        let sources = applicable
            .iter()
            .map(|file| SourceFile {
                path: file.path.clone(),
                text: file.text.clone().into(),
            })
            .collect::<Vec<_>>();
        let key = cache::repo_raw_key(
            package,
            &applicable
                .iter()
                .map(|file| (file.path.as_str(), file.digest.as_str()))
                .collect::<Vec<_>>(),
        );
        let cached = cache.read_raw(&key).filter(|diagnostics| {
            let valid = diagnostics.iter().all(|diagnostic| {
                package.manifest.diagnostics.contains_key(&diagnostic.code)
                    && sources.iter().any(|source| {
                        source.path == diagnostic.path
                            && diagnostic.start_byte <= diagnostic.end_byte
                            && source
                                .text
                                .get(diagnostic.start_byte..diagnostic.end_byte)
                                .is_some()
                    })
            });
            if !valid {
                cache.stats.raw_hits = cache.stats.raw_hits.saturating_sub(1);
                cache.stats.raw_misses += 1;
                cache
                    .notices
                    .push("discarded invalid repository cache entry".to_owned());
            }
            valid
        });
        let result = if let Some(cached) = cached {
            Ok(cached)
        } else if let Some(pool) = &mut pool {
            pool.execute(crate::worker::WorkerRequest {
                rules: vec![worker_rule(package)],
                files: sources
                    .iter()
                    .map(|file| crate::worker::SourceFileWire {
                        path: file.path.clone(),
                        text: file.text.to_string(),
                    })
                    .collect(),
                optimized,
                max_file_bytes: workspace.effective_config.scan.max_file_bytes as usize,
                repository: true,
            })
            .and_then(|result| {
                merge_runtime_stats(&mut runtime_stats, &result.stats["arena"]);
                let outcome = result
                    .results
                    .into_iter()
                    .next()
                    .ok_or_else(|| anyhow!("missing worker result"))?;
                if let Some(error) = outcome.error {
                    bail!("{error}")
                }
                Ok(outcome.diagnostics)
            })
        } else {
            program.execute_with_limits(
                &sources,
                &mut arena,
                wt_runtime::RuntimeLimits {
                    max_file_bytes: workspace.effective_config.scan.max_file_bytes as usize,
                },
            )
        };
        match result {
            Ok(diagnostics) => {
                cache.write_raw(&key, &diagnostics);
                add_records(
                    &mut records,
                    package,
                    diagnostics,
                    applicable.iter().copied(),
                );
            }
            Err(error) => {
                errors.push(json!({"rule_id": package.qualified_id, "error": error.to_string()}))
            }
        }
    }
    merge_runtime_stats(&mut runtime_stats, &arena.stats());
    for summary in &mut rule_summaries {
        let id = summary["id"].as_str().unwrap_or_default();
        let failed = errors.iter().any(|error| error["rule_id"] == id);
        let count = checked.get(id).copied().unwrap_or_else(|| {
            programs
                .iter()
                .find(|(package, _)| {
                    package.qualified_id == id && package.manifest.execution == "repository"
                })
                .and_then(|(package, _)| {
                    applicable_files(&workspace.root, package, &selection.repository_files).ok()
                })
                .map_or(0, |files| files.len())
        });
        summary["status"] = json!(if failed {
            "failed"
        } else if count == 0 {
            "not_applicable"
        } else {
            "completed"
        });
        summary["checked_files"] = json!(count);
    }
    for package in selected
        .iter()
        .filter(|package| effective_mode(&workspace, package) == "disabled")
    {
        rule_summaries.push(json!({"id":package.qualified_id,"digest":package.digest,"mode":"disabled","status":"disabled","checked_files":0}));
    }
    rule_summaries.sort_by_key(|rule| rule["id"].as_str().unwrap_or_default().to_owned());
    let relevant_gaps = selection
        .gaps
        .iter()
        .filter(|gap| {
            programs
                .iter()
                .any(|(package, _)| scope_accepts_path(&workspace.root, package, &gap.path))
        })
        .cloned()
        .collect::<Vec<_>>();
    let waiver_file = waivers::load(&workspace.local_dir.join("waivers.json"))?;
    let (diagnostics, suppressed, stale) =
        render_records(&records, &workspace, waiver_file.as_ref(), options)?;
    if !stale.is_empty() && bool_option(options, "strict", false)? {
        errors.push(json!({"error": "stale_waivers", "waivers": stale}));
    }
    let review_result = reviews::load(&workspace.root).and_then(|store| {
        reviews::apply(&workspace.root, diagnostics.clone(), &suppressed, &store)
    });
    let (diagnostics, reviewed, review_records) = match review_result {
        Ok(state) => (state.diagnostics, state.reviewed, state.records),
        Err(error) => {
            errors.push(json!({"error": format!("invalid_review_state: {error}")}));
            (diagnostics, Vec::new(), Vec::new())
        }
    };
    let mut notices = stale.clone();
    notices.extend(cache.notices.clone());
    let blocking = diagnostics
        .iter()
        .filter(|value| value["blocking"] == true)
        .count();
    let no_work =
        changed && !enabled.is_empty() && relevant_files.is_empty() && relevant_gaps.is_empty();
    if !allow_empty && !no_work && relevant_files.is_empty() {
        errors.push(json!({"error":"no_eligible_files", "help":"Use --allow-empty only when empty coverage is intended."}));
    }
    errors.sort_by_key(Value::to_string);
    let complete = errors.is_empty() && relevant_gaps.is_empty();
    let exit_code = if !complete {
        2
    } else if blocking > 0 {
        1
    } else {
        0
    };
    let status = if !complete {
        "incomplete"
    } else if diagnostics.is_empty() && relevant_files.is_empty() {
        "no_work"
    } else if diagnostics.is_empty() {
        "pass"
    } else {
        "findings"
    };
    let policy_digest = semantic_digest(
        "WT-POLICY-2",
        &[
            serde_json::to_vec(&workspace.effective_config)?.as_slice(),
            serde_json::to_vec(
                &workspace
                    .packages
                    .iter()
                    .map(|package| &package.digest)
                    .collect::<Vec<_>>(),
            )?
            .as_slice(),
        ],
    );
    let binary_skips = selection
        .coverage
        .iter()
        .filter(|file| file.status == "binary")
        .count();
    let mut result = json!({
        "schema_version": 2,
        "command": "check",
        "status": status,
        "exit_code": if !allow_empty && !no_work && (enabled.is_empty() || relevant_files.is_empty()) { 2 } else { exit_code },
        "complete": complete && (allow_empty || (!enabled.is_empty() && (!relevant_files.is_empty() || no_work))),
        "root": workspace.root,
        "scope": {"partial": selection.partial, "include_ignored": options.get("include_ignored").and_then(Value::as_bool).unwrap_or(false), "host_ignores": workspace.effective_config.scan.honor_git_local_excludes && !options.get("no_host_ignores").and_then(Value::as_bool).unwrap_or(false)},
        "changed_base": selection.changed_base,
        "policy_digest": policy_digest,
        "effective_policy": workspace.effective_config,
        "coordinate_encoding": "unicode-scalar-columns",
        "diagnostics": diagnostics,
        "suppressed": suppressed,
        "reviewed": reviewed,
        "review_records": review_records,
        "notices": notices,
        "errors": errors,
        "rules": rule_summaries,
        "files": selection.coverage,
        "summary": {"raw_findings": diagnostics.len()+reviewed.len()+suppressed.len(), "reviewed_findings": reviewed.len(), "actionable_findings": diagnostics.len(), "checked_files": relevant_files.len(), "blocking_diagnostics": blocking, "advisory_diagnostics": diagnostics.iter().filter(|value| value["blocking"] == false).count(), "review_diagnostics": diagnostics.iter().filter(|value| value["kind"] == "review").count(), "binary_skips": binary_skips, "analysis_gaps": relevant_gaps.len()},
        "gaps": relevant_gaps,
    });
    if bool_option(options, "stats", false)? {
        runtime_stats["source_reads"] = json!(selection.source_reads);
        runtime_stats["snapshot_reads"] = json!(selection.source_reads);
        runtime_stats["bytes_hashed"] = json!(selection.bytes_hashed);
        runtime_stats["relevant_rule_file_pairs"] = json!(checked.values().sum::<usize>());
        result["stats"] = json!({"runtime": runtime_stats, "cache": cache.stats_value()});
    }
    Ok(result)
}

struct FileExecution<'a> {
    scopes: BTreeMap<String, PreparedScope>,
    key_prefixes: BTreeMap<String, String>,
    programs: &'a [(&'a RulePackage, wt_runtime::Program)],
    cache: &'a mut cache::Store,
    pool: Option<crate::worker::Pool>,
    arena: QueryArena,
    optimized: bool,
    max_file_bytes: usize,
    records: Vec<Record>,
    errors: Vec<Value>,
    relevant_files: HashSet<String>,
    checked: BTreeMap<String, usize>,
    stats: Value,
}

struct PreparedScope {
    include: globset::GlobSet,
    exclude: globset::GlobSet,
    applicable: bool,
}
impl PreparedScope {
    fn new(root: &Path, package: &RulePackage) -> Result<Self> {
        let mut include = GlobSetBuilder::new();
        let mut exclude = GlobSetBuilder::new();
        for glob in &package.manifest.scope.include {
            include.add(GlobBuilder::new(glob).literal_separator(true).build()?);
        }
        for entry in &package.manifest.scope.exclude {
            exclude.add(
                GlobBuilder::new(&entry.glob)
                    .literal_separator(true)
                    .build()?,
            );
        }
        let applicable = package.manifest.scope.require_files.iter().all(|path| {
            root.join(path).is_file()
                && root
                    .join(path)
                    .canonicalize()
                    .is_ok_and(|path| path.starts_with(root))
        });
        Ok(Self {
            include: include.build()?,
            exclude: exclude.build()?,
            applicable,
        })
    }
    fn matches(&self, path: &str) -> bool {
        self.applicable && self.include.is_match(path) && !self.exclude.is_match(path)
    }
}

fn worker_rule(package: &RulePackage) -> crate::worker::WorkerRule {
    crate::worker::WorkerRule {
        id: package.qualified_id.clone(),
        manifest: package.manifest_value.clone(),
        source: package.source.clone(),
    }
}

impl FileExecution<'_> {
    fn run_batch(&mut self, files: &[SelectedFile]) -> Result<()> {
        let mut requests = Vec::new();
        let mut pending = Vec::new();
        for file in files {
            let mut missing = Vec::new();
            for (package, program) in self
                .programs
                .iter()
                .filter(|(package, _)| package.manifest.execution == "file")
            {
                if !self.scopes[&package.qualified_id].matches(&file.path) {
                    continue;
                }
                self.relevant_files.insert(file.path.clone());
                *self
                    .checked
                    .entry(package.qualified_id.clone())
                    .or_default() += 1;
                let key = semantic_digest(
                    "WT-FILE-INSTANCE-1",
                    &[
                        self.key_prefixes[&package.qualified_id].as_bytes(),
                        file.path.as_bytes(),
                        file.digest.as_bytes(),
                    ],
                );
                if let Some(diagnostics) = self.cache.read_raw_validated(
                    &key,
                    &file.path,
                    &file.text,
                    package.manifest.diagnostics.keys(),
                ) {
                    add_records(
                        &mut self.records,
                        package,
                        diagnostics,
                        std::slice::from_ref(file),
                    );
                } else if self.pool.is_some() {
                    missing.push((*package, key));
                } else {
                    let result = program.execute_with_limits(
                        &[SourceFile {
                            path: file.path.clone(),
                            text: file.text.clone().into(),
                        }],
                        &mut self.arena,
                        wt_runtime::RuntimeLimits {
                            max_file_bytes: self.max_file_bytes,
                        },
                    );
                    match result {
                        Ok(diagnostics) => {self.cache.write_raw(&key,&diagnostics); add_records(&mut self.records,package,diagnostics,std::slice::from_ref(file));}
                        Err(error) => self.errors.push(json!({"rule_id":package.qualified_id,"path":file.path,"error":error.to_string()})),
                    }
                }
            }
            self.arena.clear_file_results();
            if !missing.is_empty() {
                requests.push(crate::worker::WorkerRequest {
                    rules: missing
                        .iter()
                        .map(|(package, _)| worker_rule(package))
                        .collect(),
                    files: vec![crate::worker::SourceFileWire {
                        path: file.path.clone(),
                        text: file.text.clone(),
                    }],
                    optimized: self.optimized,
                    max_file_bytes: self.max_file_bytes,
                    repository: false,
                });
                pending.push((file, missing));
            }
        }
        if let Some(pool) = &mut self.pool {
            for ((file, missing), result) in pending.into_iter().zip(pool.execute_many(requests)) {
                match result {
                    Ok(result) => {
                        merge_runtime_stats(&mut self.stats, &result.stats["arena"]);
                        for (package, key) in missing {
                            let outcome = result
                                .results
                                .iter()
                                .find(|result| result.id == package.qualified_id);
                            match outcome {
                                Some(outcome) if outcome.error.is_none() => {
                                    self.cache.write_raw(&key,&outcome.diagnostics);
                                    add_records(&mut self.records,package,outcome.diagnostics.clone(),std::slice::from_ref(file));
                                }
                                _ => self.errors.push(json!({"rule_id":package.qualified_id,"path":file.path,"error":outcome.and_then(|outcome| outcome.error.as_deref()).unwrap_or("missing worker invocation result")})),
                            }
                        }
                    }
                    Err(error) => {
                        for (package, _) in missing {
                            self.errors.push(json!({"rule_id":package.qualified_id,"path":file.path,"error":error.to_string()}));
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

fn plan(options: &Value) -> Result<Value> {
    let workspace = discovery::discover(options)?;
    let requested = requested_rules(options)?;
    let selected = discovery::select_packages(&workspace.packages, requested.as_deref())?;
    let mut rules = Vec::new();
    let mut queries: BTreeMap<String, Value> = BTreeMap::new();
    for package in selected {
        let program = wt_runtime::compile(&package.manifest_value, &package.source)?;
        for query in &program.rule_ir().queries {
            let node = queries.entry(query.canonical_identity.clone()).or_insert_with(|| json!({
                "operation":query.operation,"result_shape":query.result_shape,"input":query.input_symbolic_expression,
                "pattern":query.pattern_expression,"flags":query.pattern_flags,"consumers":[],
                "decision":"demand-driven memoization", "reason":"Equal resolved operations and authorized runtime inputs share only when demanded."
            }));
            node["consumers"].as_array_mut().unwrap().push(json!({"rule_id":package.qualified_id,"call_site":query.call_site,"alias":query.pattern_alias,"source":query.source,"guards":query.guards}));
        }
        rules.push(json!({"id": package.qualified_id, "digest": package.digest, "scope": package.manifest.scope, "plan": program.plan()}));
    }
    let optimized = options.get("optimizer").and_then(Value::as_str) != Some("off");
    let queries = queries
        .into_values()
        .enumerate()
        .map(|(index, mut query)| {
            query["id"] = json!(format!("q{index}"));
            query["sharing_candidate"] = json!(
                optimized
                    && query["consumers"]
                        .as_array()
                        .is_some_and(|consumers| consumers.len() > 1)
            );
            if !optimized {
                query["decision"] = json!("independent reference queries");
            }
            query
        })
        .collect::<Vec<_>>();
    let plan_digest = semantic_digest(
        "WT-PLAN-2",
        &[
            serde_json::to_vec(&rules)?.as_slice(),
            serde_json::to_vec(&queries)?.as_slice(),
        ],
    );
    Ok(envelope(
        "plan",
        "pass",
        0,
        json!({"rules": rules, "queries":queries,"plan_digest":plan_digest,"optimizer": options.get("optimizer").and_then(Value::as_str).unwrap_or("auto"), "executed": false}),
    ))
}

fn list(options: &Value) -> Result<Value> {
    let workspace = discovery::discover(options)?;
    let mut rules = Vec::new();
    for package in &workspace.packages {
        rules.push(json!({"id": package.qualified_id, "unqualified_id": package.manifest.id, "origin": package.scope_name, "declared_mode": package.manifest.mode, "effective_mode": effective_mode(&workspace, package), "scope": package.manifest.scope, "tests": package.tests.is_some(), "digest": package.digest}));
    }
    Ok(envelope("list", "pass", 0, json!({"rules": rules})))
}

fn show(options: &Value) -> Result<Value> {
    if options.get("submission").is_some() {
        let submission = rule::submission_from_value(required_submission_value(options)?)?;
        return Ok(envelope(
            "show",
            "pass",
            0,
            json!({"rule": serde_json::to_value(submission)?}),
        ));
    }
    let workspace = discovery::discover(options)?;
    let id = required_string(options, "id")?;
    let selected =
        discovery::select_packages(&workspace.packages, Some(std::slice::from_ref(&id)))?;
    Ok(envelope(
        "show",
        "pass",
        0,
        json!({"rule": rule::export_value(selected[0])?, "digest": selected[0].digest}),
    ))
}

fn validate(options: &Value) -> Result<Value> {
    if options.get("submission").is_some() {
        let submission = rule::submission_from_value(required_submission_value(options)?)?;
        let value = rule::map_with_code_source(&submission);
        let program = wt_runtime::compile(&value, submission.code.source.as_deref().unwrap_or(""))?;
        return Ok(envelope(
            "validate",
            "pass",
            0,
            json!({"id": submission.id, "valid": true, "plan": program.plan()}),
        ));
    }
    let workspace = discovery::discover(options)?;
    let requested = requested_rules(options)?;
    let selected = discovery::select_packages(&workspace.packages, requested.as_deref())?;
    let mut rules = Vec::new();
    for package in selected {
        let program = wt_runtime::compile(&package.manifest_value, &package.source)?;
        rules.push(json!({"id": package.qualified_id, "valid": true, "digest": package.digest, "plan": program.plan(), "tests": package.tests.is_some()}));
    }
    Ok(envelope("validate", "pass", 0, json!({"rules": rules})))
}

fn test(options: &Value) -> Result<Value> {
    let workspace = discovery::discover(options)?;
    let requested = requested_rules(options)?;
    let selected = discovery::select_packages(&workspace.packages, requested.as_deref())?;
    let mut outcomes = Vec::new();
    let mut failed = false;
    let mut incomplete = false;
    for package in selected {
        let program = wt_runtime::compile(&package.manifest_value, &package.source)?;
        match run_fixtures(package, &program, options) {
            Ok(outcome) => {
                failed |= !outcome.passed;
                outcomes.push(json!({"id": package.qualified_id, "passed": outcome.passed, "cases": outcome.cases, "failed_cases": outcome.failed_cases}));
            }
            Err(error) => {
                incomplete = true;
                outcomes.push(json!({"id": package.qualified_id, "passed": false, "error": error.to_string()}));
            }
        }
    }
    Ok(envelope(
        "test",
        if incomplete {
            "incomplete"
        } else if failed {
            "findings"
        } else {
            "pass"
        },
        if incomplete {
            2
        } else if failed {
            1
        } else {
            0
        },
        json!({"rules": outcomes}),
    ))
}

fn set_mode(options: &Value) -> Result<Value> {
    let id = required_string(options, "id")?;
    let mode = required_string(options, "mode")?;
    let reason = required_string(options, "reason")?;
    if !matches!(mode.as_str(), "advisory" | "enforced" | "disabled") || reason.trim().is_empty() {
        bail!("set-mode requires a valid mode and nonempty reason")
    }
    let workspace = discovery::discover(options)?;
    let selected =
        discovery::select_packages(&workspace.packages, Some(std::slice::from_ref(&id)))?;
    let package = selected[0];
    let lock = acquire_lock(
        package.directory.parent().unwrap_or(&package.directory),
        &package.manifest.id,
    )?;
    let current = rule::load_package(&package.directory, &package.scope_name)?;
    let package = &current;
    if mode == "enforced" {
        let program = wt_runtime::compile(&package.manifest_value, &package.source)?;
        let _suite = package
            .tests
            .as_ref()
            .ok_or_else(|| anyhow!("enforced rule requires a test suite"))?;
        if !enforcement_evidence(package) {
            bail!("enforced rule requires positive and nonempty negative fixtures")
        }
        let outcome = run_fixtures(package, &program, options)?;
        if !outcome.passed {
            bail!("cannot enforce a failing fixture suite")
        }
    }
    let mut value = package.manifest_value.clone();
    value["mode"] = Value::String(mode.clone());
    if !value.get("metadata").is_some_and(Value::is_object) {
        let old_metadata = value.get("metadata").cloned().unwrap_or(Value::Null);
        value["metadata"] = json!({"previous_metadata": old_metadata});
    }
    value["metadata"]["mode_change"] = json!({"mode": mode, "reason": reason});
    write_atomic_file(
        &package.directory.join("rule.json"),
        serde_json::to_vec_pretty(&value)?.as_slice(),
    )?;
    drop(lock);
    let loaded = rule::load_package(&package.directory, &package.scope_name)?;
    Ok(envelope(
        "set-mode",
        "pass",
        0,
        json!({"qualified_id": loaded.qualified_id, "mode": mode, "reason": reason, "digest": loaded.digest}),
    ))
}

fn explain(options: &Value) -> Result<Value> {
    let paths = options
        .get("paths")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("explain requires one path"))?;
    if paths.len() != 1 {
        bail!("explain requires exactly one path")
    }
    let workspace = discovery::discover(options)?;
    let mut narrowed = options.clone();
    narrowed["paths"] = Value::Array(paths.clone());
    let selection = selection::select(&workspace.root, &workspace.effective_config, &narrowed)?;
    let requested = requested_rules(options)?;
    let selected = discovery::select_packages(&workspace.packages, requested.as_deref())?;
    let mut rules = Vec::new();
    for package in selected {
        rules.push(json!({"id": package.qualified_id, "mode": effective_mode(&workspace, package), "scope": package.manifest.scope}));
    }
    Ok(envelope(
        "explain",
        "pass",
        0,
        json!({"selection": selection.coverage, "gaps": selection.gaps, "rules": rules}),
    ))
}

fn show_config(options: &Value) -> Result<Value> {
    let workspace = discovery::discover(options)?;
    Ok(envelope(
        "config",
        "pass",
        0,
        config::effective_value(&workspace.effective_config),
    ))
}

fn clear_cache(_options: &Value) -> Result<Value> {
    cache::clear()?;
    Ok(envelope("cache-clear", "pass", 0, json!({"cleared": true})))
}

fn schema(options: &Value) -> Result<Value> {
    let name = options
        .get("schema")
        .or_else(|| options.get("id"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| anyhow!("schema name is required"))?;
    let schema = match name.as_str() {
        "rule" => include_str!("../../../schemas/rule.schema.json"),
        "submission" => include_str!("../../../schemas/submission.schema.json"),
        "tests" => include_str!("../../../schemas/tests.schema.json"),
        "config" => include_str!("../../../schemas/config.schema.json"),
        "result" => include_str!("../../../schemas/result.schema.json"),
        "plan" => include_str!("../../../schemas/plan.schema.json"),
        "waivers" => include_str!("../../../schemas/waivers.schema.json"),
        _ => bail!("unknown schema {name:?}"),
    };
    crate::parse_json(schema)
}

fn required_submission_value(options: &Value) -> Result<Value> {
    options
        .get("submission")
        .cloned()
        .ok_or_else(|| anyhow!("one submission source is required"))
}

fn requested_rules(options: &Value) -> Result<Option<Vec<String>>> {
    let value = options
        .get("rules")
        .or_else(|| options.get("rule"))
        .or_else(|| options.get("id"));
    let Some(value) = value else { return Ok(None) };
    if let Some(value) = value.as_str() {
        return Ok(Some(vec![value.to_owned()]));
    }
    Ok(Some(
        value
            .as_array()
            .ok_or_else(|| anyhow!("rule selection must be a string or array"))?
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| anyhow!("rule selection must contain strings"))
            })
            .collect::<Result<Vec<_>>>()?,
    ))
}

fn required_string(options: &Value, key: &str) -> Result<String> {
    options
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| anyhow!("{key} is required"))
}

fn bool_option(options: &Value, key: &str, default: bool) -> Result<bool> {
    match options.get(key) {
        None => Ok(default),
        Some(value) => value
            .as_bool()
            .ok_or_else(|| anyhow!("{key} must be boolean")),
    }
}

fn effective_mode(workspace: &Workspace, package: &RulePackage) -> String {
    workspace
        .effective_config
        .mode_overrides
        .get(&package.qualified_id)
        .map(|value| value.mode.clone())
        .unwrap_or_else(|| package.manifest.mode.clone())
}

fn applicable_files<'a>(
    root: &Path,
    package: &RulePackage,
    files: &'a [SelectedFile],
) -> Result<Vec<&'a SelectedFile>> {
    let mut includes = GlobSetBuilder::new();
    for value in &package.manifest.scope.include {
        includes.add(GlobBuilder::new(value).literal_separator(true).build()?);
    }
    let includes = includes.build()?;
    let mut excludes = GlobSetBuilder::new();
    for value in &package.manifest.scope.exclude {
        excludes.add(
            GlobBuilder::new(&value.glob)
                .literal_separator(true)
                .build()?,
        );
    }
    let excludes = excludes.build()?;
    for path in &package.manifest.scope.require_files {
        let required = root.join(path);
        if !required.is_file() || !required.canonicalize()?.starts_with(root) {
            return Ok(Vec::new());
        }
    }
    Ok(files
        .iter()
        .filter(|file| includes.is_match(&file.path) && !excludes.is_match(&file.path))
        .collect())
}

fn scope_accepts_path(root: &Path, package: &RulePackage, path: &str) -> bool {
    if package.manifest.scope.require_files.iter().any(|required| {
        let path = root.join(required);
        !path.is_file()
            || path
                .canonicalize()
                .map_or(true, |path| !path.starts_with(root))
    }) {
        return false;
    }
    let mut includes = GlobSetBuilder::new();
    let mut excludes = GlobSetBuilder::new();
    for glob in &package.manifest.scope.include {
        if let Ok(glob) = GlobBuilder::new(glob).literal_separator(true).build() {
            includes.add(glob);
        }
    }
    for glob in &package.manifest.scope.exclude {
        if let Ok(glob) = GlobBuilder::new(&glob.glob).literal_separator(true).build() {
            excludes.add(glob);
        }
    }
    includes
        .build()
        .map(|includes| {
            includes.is_match(path)
                && !excludes
                    .build()
                    .is_ok_and(|excludes| excludes.is_match(path))
        })
        .unwrap_or(false)
}

fn merge_runtime_stats(target: &mut Value, source: &Value) {
    if let Some(source) = source.as_object() {
        for (key, value) in source {
            if let Some(value) = value.as_u64() {
                let previous = target[key].as_u64().unwrap_or(0);
                target[key] = json!(if key.contains("peak") {
                    previous.max(value)
                } else {
                    previous.saturating_add(value)
                });
            }
        }
    }
}

#[derive(Clone)]
struct Record {
    rule_id: String,
    diagnostic: RawDiagnostic,
    matched_digest: String,
    start: (usize, usize),
    end: (usize, usize),
    file_digest: String,
    context_digest: String,
}

fn add_records<'a>(
    records: &mut Vec<Record>,
    package: &RulePackage,
    diagnostics: Vec<RawDiagnostic>,
    files: impl IntoIterator<Item = &'a SelectedFile>,
) {
    let snapshots = files
        .into_iter()
        .map(|file| {
            (
                file.path.as_str(),
                (file.text.as_str(), file.digest.as_str()),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let context_parts = snapshots
        .iter()
        .flat_map(|(path, (_, digest))| [path.as_bytes(), digest.as_bytes()])
        .collect::<Vec<_>>();
    let context_digest = semantic_digest("WT-REVIEW-CONTEXT-1", &context_parts);
    for diagnostic in diagnostics {
        let (source, file_digest) = snapshots
            .get(diagnostic.path.as_str())
            .cloned()
            .unwrap_or_default();
        records.push(Record {
            rule_id: package.qualified_id.clone(),
            matched_digest: crate::digest::digest_bytes(
                source
                    .get(diagnostic.start_byte..diagnostic.end_byte)
                    .unwrap_or_default()
                    .as_bytes(),
            ),
            start: coordinates(source, diagnostic.start_byte),
            end: coordinates(source, diagnostic.end_byte),
            diagnostic,
            file_digest: file_digest.to_owned(),
            context_digest: context_digest.clone(),
        });
    }
}

fn render_records(
    records: &[Record],
    workspace: &Workspace,
    waiver_file: Option<&waivers::WaiverFile>,
    options: &Value,
) -> Result<(Vec<Value>, Vec<Value>, Vec<String>)> {
    let mut ordered = records.iter().collect::<Vec<_>>();
    ordered.sort_by(|left, right| {
        (
            left.diagnostic.path.as_str(),
            left.diagnostic.start_byte,
            left.diagnostic.end_byte,
            left.rule_id.as_str(),
            left.diagnostic.code.as_str(),
        )
            .cmp(&(
                right.diagnostic.path.as_str(),
                right.diagnostic.start_byte,
                right.diagnostic.end_byte,
                right.rule_id.as_str(),
                right.diagnostic.code.as_str(),
            ))
    });
    ordered.dedup_by(|right, left| {
        right.rule_id == left.rule_id
            && right.diagnostic.path == left.diagnostic.path
            && right.diagnostic.code == left.diagnostic.code
            && right.diagnostic.start_byte == left.diagnostic.start_byte
            && right.diagnostic.end_byte == left.diagnostic.end_byte
    });
    let mut groups: BTreeMap<(String, String, String), Vec<usize>> = BTreeMap::new();
    for (index, record) in ordered.iter().enumerate() {
        groups
            .entry((
                record.rule_id.clone(),
                record.diagnostic.code.clone(),
                record.diagnostic.path.clone(),
            ))
            .or_default()
            .push(index);
    }
    let mut suppressed_indices = HashSet::new();
    let mut suppressed = Vec::new();
    let mut stale = Vec::new();
    let mut considered = HashSet::new();
    for ((rule_id, code, path), indexes) in groups {
        considered.insert((rule_id.clone(), code.clone(), path.clone()));
        let digests = indexes
            .iter()
            .map(|index| ordered[*index].matched_digest.clone())
            .collect::<Vec<_>>();
        let application = waivers::apply(waiver_file, &rule_id, &code, &path, &digests, |_| {})?;
        if waiver_file.is_some() {
            for (waiver_id, local_index) in application
                .suppressed
                .iter()
                .zip(application.suppressed_indices)
            {
                if let Some(index) = indexes.get(local_index) {
                    suppressed_indices.insert(*index);
                    let mut diagnostic = diagnostic_value(ordered[*index], workspace, options)?;
                    diagnostic["waiver_id"] = json!(waiver_id);
                    diagnostic["blocking"] = json!(false);
                    suppressed.push(diagnostic);
                }
            }
        }
        stale.extend(application.stale);
    }
    if let Some(file) = waiver_file {
        for waiver in &file.waivers {
            let key = (
                waiver.rule_id.clone(),
                waiver.code.clone(),
                waiver.path.clone(),
            );
            if !considered.contains(&key)
                && !suppressed
                    .iter()
                    .any(|value| value["waiver_id"] == waiver.id)
            {
                stale.push(waiver.id.clone());
            }
        }
    }
    stale.sort();
    stale.dedup();
    let diagnostics = ordered
        .iter()
        .enumerate()
        .filter(|(index, _)| !suppressed_indices.contains(index))
        .map(|(_, record)| diagnostic_value(record, workspace, options))
        .collect::<Result<Vec<_>>>()?;
    Ok((diagnostics, suppressed, stale))
}

fn diagnostic_value(record: &Record, workspace: &Workspace, options: &Value) -> Result<Value> {
    let package = workspace
        .packages
        .iter()
        .find(|package| package.qualified_id == record.rule_id)
        .ok_or_else(|| anyhow!("unknown result rule"))?;
    let definition = package
        .manifest
        .diagnostics
        .get(&record.diagnostic.code)
        .ok_or_else(|| anyhow!("unknown diagnostic code"))?;
    let mode = effective_mode(workspace, package);
    let strict = options
        .get("strict")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let start = record.start;
    let end = record.end;
    let mut value = json!({
        "rule_id": package.qualified_id,
        "rule_digest": package.digest,
        "code": record.diagnostic.code,
        "kind": definition.kind,
        "severity": package.manifest.severity,
        "mode": mode,
        "blocking": mode == "enforced" || (strict && mode == "advisory"),
        "path": record.diagnostic.path,
        "file_digest": record.file_digest,
        "context_digest": record.context_digest,
        "engine_digest": cache::compiled_semantic_identity(),
        "start_byte": record.diagnostic.start_byte,
        "end_byte": record.diagnostic.end_byte,
        "start_line": start.0,
        "start_column": start.1,
        "end_line": end.0,
        "end_column": end.1,
        "message": definition.message,
        "help": definition.help
    });
    reviews::decorate(&mut value, &record.matched_digest)?;
    Ok(value)
}

fn coordinates(source: &str, offset: usize) -> (usize, usize) {
    let offset = offset.min(source.len());
    let mut line = 1;
    let mut column = 1;
    for (index, character) in source.char_indices() {
        if index >= offset {
            break;
        }
        if character == '\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }
    (line, column)
}

fn envelope(command: &str, status: &str, exit_code: i32, fields: Value) -> Value {
    let mut object = match fields {
        Value::Object(value) => value,
        _ => Map::new(),
    };
    object.insert("schema_version".to_owned(), json!(2));
    object.insert("command".to_owned(), json!(command));
    object.insert("status".to_owned(), json!(status));
    object.insert("exit_code".to_owned(), json!(exit_code));
    Value::Object(object)
}

fn unqualified_id(value: &str) -> &str {
    value.rsplit_once('/').map_or(value, |(_, id)| id)
}

fn write_replacement(parent: &Path, submission: &Submission) -> Result<tempfile::TempDir> {
    let (manifest, tests) = rule::submission_manifest(submission);
    let temp = tempfile::Builder::new()
        .prefix(".wt-rule-")
        .tempdir_in(parent)?;
    fs::write(
        temp.path().join("rule.json"),
        serde_json::to_vec_pretty(&rule::disk_manifest_value(&manifest))?,
    )?;
    fs::write(
        temp.path().join("check.wt"),
        submission
            .code
            .source
            .as_deref()
            .unwrap_or_default()
            .as_bytes(),
    )?;
    if let Some(doc) = &submission.documentation {
        fs::write(
            temp.path().join("rule.md"),
            doc.source.as_deref().unwrap_or_default(),
        )?;
    }
    if submission.tests.is_some() {
        fs::write(
            temp.path().join("tests.json"),
            serde_json::to_vec_pretty(&tests)?,
        )?;
    }
    Ok(temp)
}

struct LockGuard {
    _file: fs::File,
}

fn acquire_lock(parent: &Path, id: &str) -> Result<LockGuard> {
    let lock_path = parent.join(format!(".{id}.lock"));
    use std::fs::OpenOptions;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|error| anyhow!("unable to acquire package lock: {error}"))?;
    file.try_lock()
        .map_err(|error| anyhow!("package is locked: {error}"))?;
    let backup = parent.join(format!(".{id}.backup"));
    if backup.exists() {
        let target = parent.join(id);
        if target.exists() {
            fs::remove_dir_all(backup)?;
        } else {
            fs::rename(backup, target)?;
        }
    }
    Ok(LockGuard { _file: file })
}

fn write_create_new(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::fs::OpenOptions;
    use std::io::Write;
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn write_atomic_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("invalid target path"))?;
    let temp = tempfile::NamedTempFile::new_in(parent)?;
    fs::write(temp.path(), bytes)?;
    temp.persist(path)
        .map_err(|error| anyhow!("unable to replace {}: {error}", path.display()))?;
    Ok(())
}

fn enforcement_evidence(package: &RulePackage) -> bool {
    package.tests.as_ref().is_some_and(|suite| {
        suite.has_applicable_positive_and_negative(&package.manifest, &package.fixture_contents)
    })
}

fn run_fixtures(
    package: &RulePackage,
    program: &wt_runtime::Program,
    options: &Value,
) -> Result<fixtures::TestOutcome> {
    let Some(executable) = options.get("_worker_executable").and_then(Value::as_str) else {
        return fixtures::run_suite(package, program);
    };
    let mut pool = crate::worker::Pool::new(Path::new(executable), 1)?;
    fixtures::run_suite_with(package, |files| {
        let result = pool.execute(crate::worker::WorkerRequest {
            rules: vec![worker_rule(package)],
            files: files
                .iter()
                .map(|file| crate::worker::SourceFileWire {
                    path: file.path.clone(),
                    text: file.text.to_string(),
                })
                .collect(),
            optimized: true,
            max_file_bytes: 4 * 1024 * 1024,
            repository: package.manifest.execution == "repository",
        })?;
        let outcome = result
            .results
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("missing fixture worker result"))?;
        if let Some(error) = outcome.error {
            bail!("fixture analysis failed: {error}")
        }
        Ok(outcome.diagnostics)
    })
}

/// Format all selected packages before writing any of them. This command does
/// not migrate manifests, mutate fixtures, or execute application code.
fn format_rules(options: &Value) -> Result<Value> {
    let global = bool_option(options, "global", false)?;
    let mut discovery_options = options.clone();
    if !global {
        discovery_options["no_global"] = json!(true);
    }
    let workspace = discovery::discover(&discovery_options)?;
    let requested = requested_rules(options)?;
    let selected = discovery::select_packages(&workspace.packages, requested.as_deref())?;
    let wanted_scope = if global { "global" } else { "local" };
    if requested.is_some() && selected.iter().any(|p| p.scope_name != wanted_scope) {
        bail!("selected rule is outside the explicitly chosen formatting scope");
    }
    let mut pending = Vec::new();
    for package in selected
        .into_iter()
        .filter(|p| p.scope_name == wanted_scope)
    {
        wt_runtime::compile(&package.manifest_value, &package.source)?;
        let formatted = crate::formatting::format_source(&package.source)?;
        wt_runtime::compile(&package.manifest_value, &formatted)?;
        if formatted != package.source {
            pending.push((package, formatted));
        }
    }
    let check_only = bool_option(options, "check", false)?;
    let mut changed = Vec::new();
    for (package, formatted) in pending {
        if !check_only {
            let _lock = acquire_lock(package.directory.parent().unwrap(), &package.manifest.id)?;
            let current = rule::load_package(&package.directory, &package.scope_name)?;
            if current.digest != package.digest {
                bail!("rule changed while formatting");
            }
            write_atomic_file(&package.directory.join("check.wt"), formatted.as_bytes())?;
        }
        changed.push(package.qualified_id.clone());
    }
    Ok(envelope(
        "fmt",
        if changed.is_empty() {
            "pass"
        } else {
            "findings"
        },
        i32::from(check_only && !changed.is_empty()),
        json!({"changed":changed,"check_only":check_only}),
    ))
}

fn review(options: &Value) -> Result<Value> {
    let id = required_string(options, "finding_id")?;
    let mut check_options = serde_json::Map::new();
    for key in COMMON_OPTIONS
        .iter()
        .copied()
        .chain(["rules", "no_global", "no_host_ignores"])
    {
        if let Some(value) = options.get(key) {
            check_options.insert(key.to_owned(), value.clone());
        }
    }
    check_options.insert("no_cache".to_owned(), json!(true));
    let checked = check(&Value::Object(check_options))?;
    if checked["complete"] != true {
        bail!("cannot record review after incomplete analysis");
    }
    let findings = checked["diagnostics"]
        .as_array()
        .into_iter()
        .flatten()
        .chain(checked["reviewed"].as_array().into_iter().flatten())
        .filter(|f| f["finding_id"] == id)
        .collect::<Vec<_>>();
    if findings.len() != 1 {
        bail!("finding is absent, waived, or ambiguous; run check again");
    }
    let finding = findings[0];
    let workspace = discovery::discover(options)?;
    let current = workspace
        .packages
        .iter()
        .find(|p| finding["rule_id"] == p.qualified_id)
        .ok_or_else(|| anyhow!("reviewed rule disappeared"))?;
    if finding["rule_digest"] != current.digest {
        bail!("rule changed after check");
    }
    reviews::record(&workspace.root, finding, options)
}
