use crate::cache;
use crate::config;
use crate::digest::semantic_digest;
use crate::discovery::{self, Workspace};
use crate::fixtures;
use crate::reviews;
use crate::rule::{self, RulePackage, Submission};
use crate::selection::{self, SelectedFile};
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
        "reviews" => reviews::list(
            &discovery::resolve_root(options.get("root").and_then(Value::as_str))?,
            options.get("finding_id").and_then(Value::as_str),
        ),
        "inspect" => inspect(options),
        "capabilities" => capabilities(),
        "guide" => guide(options),
        "init" => init(options),
        "new" => create(options),
        "update" => update(options),
        "check" => check(options),
        "stats" => stats(options),
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
        "reviews" => &["finding_id"],
        "inspect" => &["finding_id", "no_global", "no_host_ignores"],
        "capabilities" => &[],
        "guide" => &["topic"],
        "init" => &["global"],
        "new" => &["global", "submission"],
        "update" => &[
            "id",
            "submission",
            "expect_hash",
            "allow_test_removal",
            "reason",
            "preview",
        ],
        "check" => &[
            "detail",
            "submission",
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
            "show_reviewed",
            "allow_empty",
            "stats",
        ],
        "stats" => &[
            "paths",
            "include_ignored",
            "no_host_ignores",
            "no_global",
            "rules",
            "no_cache",
            "optimizer",
            "changed",
            "base",
            "jobs",
            "max_file_bytes",
        ],
        "plan" => &["rules", "rule", "no_global", "optimizer"],
        "list" => &["no_global"],
        "config" => &["no_global", "max_file_bytes"],
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
            bail!("jobs must be between 1 and 3 under the absolute worker memory ceiling")
        }
    }
    if let Some(detail) = object.get("detail") {
        if !matches!(detail.as_str(), Some("summary" | "full")) {
            bail!("detail must be summary or full")
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
  "schema_version": 1,
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
    let global_dir = config::global_dir(options.get("global_dir").and_then(Value::as_str))?;
    let global_config = config::load(&global_dir.join("config.json"))?;
    let local_config = if global {
        None
    } else {
        config::load(&root.join(".wt/config.json"))?
    };
    let effective = config::merge(global_config.as_ref(), local_config.as_ref(), options)?;
    let limits = effective.runtime.limits(effective.scan.max_file_bytes);
    let parent = if global {
        global_dir.join("rules")
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
        .map(|_| {
            run_fixtures_with_limits(
                &loaded,
                &program,
                options,
                limits,
                effective.runtime.memory(),
            )
        })
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
    let preview = bool_option(options, "preview", false)?;
    let lock = if preview {
        None
    } else {
        Some(acquire_lock(parent, &submission.id)?)
    };
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
    let preview_dir = if preview {
        Some(tempfile::tempdir()?)
    } else {
        None
    };
    let replacement = write_replacement(
        preview_dir.as_ref().map_or(parent, |dir| dir.path()),
        &submission,
    )?;
    let candidate = rule::load_package(replacement.path(), &package.scope_name)?;
    if candidate.manifest.mode == "enforced" && !enforcement_evidence(&candidate) {
        bail!("enforced rule requires applicable positive and nonempty negative fixtures")
    }
    let program = wt_runtime::compile(&candidate.manifest_value, &candidate.source)?;
    if let Some(outcome) = candidate
        .tests
        .as_ref()
        .map(|_| {
            run_fixtures_with_limits(
                &candidate,
                &program,
                options,
                workspace
                    .effective_config
                    .runtime
                    .limits(workspace.effective_config.scan.max_file_bytes),
                workspace.effective_config.runtime.memory(),
            )
        })
        .transpose()?
    {
        if !outcome.passed {
            bail!("supplied fixture suite failed")
        }
    }
    let changes = {
        let old_cases = old_submission.tests.as_ref().map(|suite| &suite.cases);
        let new_cases = submission.tests.as_ref().map(|suite| &suite.cases);
        let old_by_name = old_cases
            .into_iter()
            .flatten()
            .map(|case| (case.name.as_str(), case))
            .collect::<BTreeMap<_, _>>();
        let new_by_name = new_cases
            .into_iter()
            .flatten()
            .map(|case| (case.name.as_str(), case))
            .collect::<BTreeMap<_, _>>();
        json!({
            "added_cases": new_by_name.keys().filter(|name| !old_by_name.contains_key(*name)).collect::<Vec<_>>(),
            "removed_cases": old_by_name.keys().filter(|name| !new_by_name.contains_key(*name)).collect::<Vec<_>>(),
            "changed_inputs": old_by_name.iter().filter_map(|(name, old)| new_by_name.get(name).filter(|new| json!(old.files) != json!(new.files)).map(|_| name)).collect::<Vec<_>>(),
            "changed_expectations": old_by_name.iter().filter_map(|(name, old)| new_by_name.get(name).filter(|new| json!(old.expect) != json!(new.expect)).map(|_| name)).collect::<Vec<_>>(),
            "code": package.source != candidate.source,
            "patterns": json!(package.manifest.patterns) != json!(candidate.manifest.patterns),
            "diagnostics": json!(package.manifest.diagnostics) != json!(candidate.manifest.diagnostics),
            "documentation": old_submission.documentation.source != submission.documentation.source,
            "scope": serde_json::to_value(&package.manifest.scope)? != serde_json::to_value(&candidate.manifest.scope)?,
            "mode": package.manifest.mode != candidate.manifest.mode,
            "execution": package.manifest.execution != candidate.manifest.execution,
            "intent": package.manifest.intent != candidate.manifest.intent,
            "severity": package.manifest.severity != candidate.manifest.severity,
            "metadata": package.manifest.metadata != candidate.manifest.metadata
        })
    };
    if preview {
        return Ok(envelope(
            "update",
            "pass",
            0,
            json!({"preview":true, "qualified_id": package.qualified_id,
            "previous_digest": package.digest, "candidate_digest": candidate.digest,
            "changes": changes, "validation":"passed", "tests": if candidate.tests.is_some() {"passed"} else {"untested"}}),
        ));
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
        json!({"qualified_id": loaded.qualified_id, "path": loaded.directory, "mode": loaded.manifest.mode, "digest": loaded.digest, "previous_digest": expected, "changes": changes, "validation": "passed", "tests": if loaded.tests.is_some() {"passed"} else {"untested"}}),
    ))
}

fn check(options: &Value) -> Result<Value> {
    let started = std::time::Instant::now();
    let mut discovery_options = options.clone();
    if options.get("submission").is_some() {
        discovery_options["_candidate_preview"] = json!(true);
    }
    let mut workspace = discovery::discover(&discovery_options)?;
    let preview_dir = if let Some(value) = options.get("submission") {
        if requested_rules(options)?.is_some() {
            bail!("candidate preview cannot select installed rules")
        }
        let submission = rule::prepare_submission(rule::submission_from_value(value.clone())?)?;
        if submission.tests.as_ref().is_some_and(|suite| {
            suite
                .cases
                .iter()
                .flat_map(|case| case.files.iter())
                .any(|file| file.fixture.is_some())
        }) {
            bail!("candidate preview requires inline fixture content")
        }
        let temp = tempfile::tempdir()?;
        let package_dir = write_replacement(temp.path(), &submission)?;
        let candidate = rule::load_package(package_dir.path(), "candidate")?;
        workspace.packages = vec![candidate];
        Some((temp, package_dir))
    } else {
        None
    };
    let preview = preview_dir.is_some();
    if bool_option(options, "no_host_ignores", false)? {
        workspace.effective_config.scan.honor_git_local_excludes = false;
        workspace
            .effective_config
            .origins
            .push(config::ValueOrigin {
                key: "scan.honor_git_local_excludes".to_owned(),
                origin: "cli".to_owned(),
            });
    }
    if bool_option(options, "include_ignored", false)? {
        workspace.effective_config.scan.respect_gitignore = false;
        workspace
            .effective_config
            .origins
            .push(config::ValueOrigin {
                key: "scan.respect_gitignore".to_owned(),
                origin: "cli".to_owned(),
            });
    }
    let requested = requested_rules(options)?;
    let selected = discovery::select_packages(&workspace.packages, requested.as_deref())?;
    let enabled = selected
        .iter()
        .copied()
        .filter(|package| effective_mode(&workspace, package) != "disabled")
        .collect::<Vec<_>>();
    let allow_empty = bool_option(options, "allow_empty", false)?;
    if enabled.is_empty() && !allow_empty && workspace.effective_config.expectations.is_empty() {
        bail!("no enabled rules; use allow_empty to permit an empty check")
    }
    let changed = options
        .get("changed")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let no_cache = preview || bool_option(options, "no_cache", false)?;
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
                    let limits = workspace
                        .effective_config
                        .runtime
                        .limits(workspace.effective_config.scan.max_file_bytes);
                    let fixture_key = cache::fixture_key_with_limits(package, limits);
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
                        (Ok(key), None) => {
                            match run_fixtures_with_limits(
                                package,
                                &program,
                                options,
                                limits,
                                workspace.effective_config.runtime.memory(),
                            ) {
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
                            }
                        }
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
    let optimized = workspace.effective_config.optimizer.mode != "off";
    let limits = workspace
        .effective_config
        .runtime
        .limits(workspace.effective_config.scan.max_file_bytes);
    let memory = workspace.effective_config.runtime.memory();
    let max_jobs = memory.max_workers()?;
    let jobs = options
        .get("jobs")
        .and_then(Value::as_u64)
        .map(|n| n as usize)
        .unwrap_or_else(|| {
            std::thread::available_parallelism().map_or(1, |n| n.get().min(max_jobs))
        });
    if jobs > max_jobs {
        bail!("--jobs {jobs} exceeds configured memory capacity of {max_jobs} worker(s)");
    }
    let pool = options
        .get("_worker_executable")
        .and_then(Value::as_str)
        .map(|path| crate::worker::Pool::with_memory(Path::new(path), jobs, memory))
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
                    cache::raw_key_with_limits(package, "", "", limits),
                )
            })
            .collect(),
        programs: &programs,
        cache: &mut cache,
        pool,
        arena: QueryArena::new(optimized),
        optimized,
        limits,
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
        let key = cache::repo_raw_key_with_limits(
            package,
            &applicable
                .iter()
                .map(|file| (file.path.as_str(), file.digest.as_str()))
                .collect::<Vec<_>>(),
            limits,
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
                limits,
                repository: true,
            })
            .and_then(|result| {
                merge_runtime_stats(&mut runtime_stats, &result.stats["arena"]);
                merge_runtime_stats(
                    &mut runtime_stats["compilation"],
                    &result.stats["compilation"],
                );
                merge_runtime_stats(&mut runtime_stats["transport"], &result.stats["transport"]);
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
            program.execute_with_limits(&sources, &mut arena, limits)
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
    let diagnostics = render_records(&records, &workspace, options)?;
    let review_result = (if preview {
        Ok(reviews::Store::default())
    } else {
        reviews::load(&workspace.root)
    })
    .and_then(|store| reviews::apply(&workspace.root, diagnostics.clone(), &store));
    let (diagnostics, reviewed, review_records, evidence_reads) = match review_result {
        Ok(state) => (
            state.diagnostics,
            state.reviewed,
            state.records,
            Some(state.evidence_reads),
        ),
        Err(error) => {
            errors.push(json!({"error": format!("invalid_review_state: {error}")}));
            (diagnostics, Vec::new(), Vec::new(), None)
        }
    };
    let mut notices = Vec::new();
    notices.extend(cache.notices.clone());
    let partial_policy = selection.partial || requested.is_some() || preview;
    let coverage_expectations = workspace.effective_config.expectations.iter().map(|entry| {
        let rule = rule_summaries.iter().find(|rule| rule["id"] == entry.expectation.rule_id);
        let relevant_gap = workspace.packages.iter()
            .find(|package| package.qualified_id == entry.expectation.rule_id)
            .is_some_and(|package| selection.gaps.iter().any(|gap| scope_accepts_path(&workspace.root, package, &gap.path)));
        let checked_files = rule.and_then(|rule| rule["checked_files"].as_u64()).unwrap_or(0);
        let omitted_global = !preview && options["no_global"] == true && entry.expectation.rule_id.starts_with("global/");
        let status = if omitted_global {
            "missing_rule"
        } else if partial_policy {
            "not_evaluated_partial"
        } else if rule.is_none() {
            "missing_rule"
        } else if rule.is_some_and(|rule| rule["status"] == "disabled") {
            "disabled"
        } else if rule.is_some_and(|rule| rule["status"] == "failed") || relevant_gap {
            "incomplete"
        } else if rule.is_some_and(|rule| rule["status"] == "not_applicable") {
            "not_applicable"
        } else if checked_files < entry.expectation.minimum_files {
            "insufficient_files"
        } else {
            "satisfied"
        };
        json!({"rule_id": entry.expectation.rule_id, "minimum_files": entry.expectation.minimum_files,
            "reason": entry.expectation.reason, "origin": entry.origin, "completed_files": if rule.is_some_and(|rule| rule["status"] == "failed") {0} else {checked_files},
            "source_gaps": selection.gaps.iter().filter(|gap| workspace.packages.iter().any(|package| package.qualified_id == entry.expectation.rule_id && scope_accepts_path(&workspace.root, package, &gap.path))).count(),
            "status": status})
    }).collect::<Vec<_>>();
    for expectation in &coverage_expectations {
        if !matches!(
            expectation["status"].as_str(),
            Some("satisfied" | "not_evaluated_partial")
        ) {
            errors.push(json!({"error":"coverage_expectation_unmet", "rule_id":expectation["rule_id"], "status":expectation["status"]}));
        }
    }
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
    let mut semantic_policy = config::effective_value(&workspace.effective_config);
    semantic_policy.as_object_mut().unwrap().remove("optimizer");
    if let Some(origins) = semantic_policy
        .get_mut("origins")
        .and_then(Value::as_array_mut)
    {
        origins.retain(|entry| entry["key"] != "optimizer.mode");
    }
    let policy_digest = semantic_digest(
        "WT-POLICY-3",
        &[
            serde_json::to_vec(&semantic_policy)?.as_slice(),
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
        "schema_version": crate::CONTRACT_VERSION,
        "command": "check",
        "status": status,
        "exit_code": if !allow_empty && !no_work && (enabled.is_empty() || relevant_files.is_empty()) { 2 } else { exit_code },
        "complete": complete && (allow_empty || (!enabled.is_empty() && (!relevant_files.is_empty() || no_work))),
        "root": workspace.root,
        "scope": {"partial": partial_policy, "include_ignored": options.get("include_ignored").and_then(Value::as_bool).unwrap_or(false), "host_ignores": workspace.effective_config.scan.honor_git_local_excludes && !options.get("no_host_ignores").and_then(Value::as_bool).unwrap_or(false)},
        "changed_base": selection.changed_base,
        "policy_digest": policy_digest,
        "effective_policy": workspace.effective_config,
        "coverage_expectations": coverage_expectations,
        "coordinate_encoding": "unicode-scalar-columns",
        "diagnostics": diagnostics,
        "reviewed": reviewed,
        "review_records": review_records,
        "notices": notices,
        "errors": errors,
        "rules": rule_summaries,
        "files": selection.coverage,
        "summary": {"raw_findings": diagnostics.len()+reviewed.len(), "reviewed_findings": reviewed.len(), "actionable_findings": diagnostics.len(), "checked_files": relevant_files.len(), "blocking_diagnostics": blocking, "advisory_diagnostics": diagnostics.iter().filter(|value| value["blocking"] == false).count(), "review_diagnostics": diagnostics.iter().filter(|value| value["kind"] == "review").count(), "binary_skips": binary_skips, "analysis_gaps": relevant_gaps.len()},
        "gaps": relevant_gaps,
    });
    if bool_option(options, "stats", false)? {
        runtime_stats["admitted_jobs"] = json!(jobs);
        runtime_stats["worker_reservation_bytes"] = json!(memory.worker_bytes);
        runtime_stats["parent_reservation_bytes"] = json!(memory.parent_bytes);
        runtime_stats["total_scheduling_bytes"] = json!(memory.total_bytes);
        runtime_stats["source_reads"] = json!(selection.source_reads);
        runtime_stats["snapshot_reads"] = json!(selection.source_reads);
        runtime_stats["bytes_hashed"] = json!(selection.bytes_hashed);
        runtime_stats["relevant_rule_file_pairs"] = json!(checked.values().sum::<usize>());
        result["stats"] = json!({"runtime": runtime_stats, "cache": cache.stats_value(),
            "measurement":{"elapsed_ms":started.elapsed().as_millis(),
                "review_evidence_reads":evidence_reads.map_or(json!("not_measured"), |n| json!(n)), "peak_rss_bytes":"not_measured"}});
    }
    if preview {
        result["preview"] = json!(true);
    }
    Ok(result)
}

/// `wt stats`: a read-only view over data `wt check` and the review store
/// already produce. No history is kept; every number is recomputed from the
/// current repository, rules and stored review records on each run.
///
/// Per rule this reports the mode/severity/intent, how many files in this
/// repository are in this rule's scope after the same include/exclude and
/// ignore handling `wt check` uses (`scoped_files`), any of the rule's own
/// include globs that individually match none of those files
/// (`unmatched_include`, informational; e.g. a typo'd extension next to a
/// glob that does match), the raw findings from a check run with the same
/// scope and flags as `wt check`, review decisions by outcome, how many of
/// those decisions are currently stale, and one derived signal, computed in
/// this priority order:
/// - `disabled`: the rule does not currently execute.
/// - `unknown`: the underlying check was incomplete; a signal would be a guess.
/// - `useful`: at least one occurrence was confirmed as a real issue.
/// - `dead`: the rule's scope matches no file in this repository, so it
///   cannot fire here (a fixture failure already makes the whole run
///   incomplete, so a broken rule is reported `unknown`, never `dead`).
/// - `watch`: an `intent: watch` rule (one whose purpose is to force
///   re-review whenever specific code changes) has current findings or
///   decisions; each acceptance is expected to reopen when the watched code
///   changes, so this is healthy, not noise.
/// - `quiet`: scope matches files, and there are no current findings and no
///   pending decisions — a healthy, working rule.
/// - `noisy`: most decided findings were accepted as acceptable.
/// - `active`: none of the above; the rule has activity that has not settled
///   into a clear pattern yet.
fn stats(options: &Value) -> Result<Value> {
    let mut check_options = Map::new();
    for key in COMMON_OPTIONS.iter().copied().chain([
        "paths",
        "include_ignored",
        "no_host_ignores",
        "no_global",
        "rules",
        "no_cache",
        "optimizer",
        "changed",
        "base",
        "jobs",
        "max_file_bytes",
    ]) {
        if let Some(value) = options.get(key) {
            check_options.insert(key.to_owned(), value.clone());
        }
    }
    // A rule that is disabled, or a repository with nothing currently
    // enabled or matching, is still meaningful stats output (every rule
    // reports as `disabled` or `dead`), not a hard failure.
    check_options.insert("allow_empty".to_owned(), json!(true));
    let checked = check(&Value::Object(check_options))?;
    let complete = checked["complete"] == true;

    let workspace = discovery::discover(options)?;
    let requested = requested_rules(options)?;
    let selected = discovery::select_packages(&workspace.packages, requested.as_deref())?;

    // Files that survived ignore/exclude handling and were actually read as
    // text, under the same scope and flags as the check above (a superset
    // across every currently enabled rule's scope, since `check` unions them
    // before reading any bytes). Each rule's own scope is matched against
    // this list below, independently of whether that rule's own fixtures
    // passed: a fixture failure already leaves an error in `checked["errors"]`
    // and makes `complete` false, so `dead`/`quiet` never need to special-case
    // it themselves — such a rule is reported `unknown`, not a guess.
    let eligible_paths = checked["files"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|file| {
            matches!(
                file["status"].as_str(),
                Some("checked" | "unchanged_dependency")
            )
        })
        .filter_map(|file| file["path"].as_str())
        .collect::<Vec<_>>();

    // A corrupt or inconsistent review store is already surfaced through
    // `checked["errors"]` and `complete == false`; do not let it turn the
    // whole stats command into a hard error.
    let all_reviews =
        reviews::list(&workspace.root, None).unwrap_or_else(|_| json!({"reviews": []}));
    let mut finding_rule = BTreeMap::<String, String>::new();
    let mut review_by_rule = BTreeMap::<String, [u64; 4]>::new();
    for entry in all_reviews["reviews"].as_array().into_iter().flatten() {
        let rule_id = entry["record"]["rule_id"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let finding_id = entry["record"]["finding_id"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        finding_rule.insert(finding_id, rule_id.clone());
        let counts = review_by_rule.entry(rule_id).or_insert([0; 4]);
        match entry["record"]["decision"].as_str() {
            Some("acceptable") => counts[0] += 1,
            Some("confirmed_issue") => counts[1] += 1,
            Some("accepted_risk") => counts[2] += 1,
            Some("needs_review") => counts[3] += 1,
            _ => {}
        }
    }

    let mut stale_by_rule = BTreeMap::<String, u64>::new();
    let mut raw_by_rule = BTreeMap::<String, u64>::new();
    if complete {
        for record in checked["review_records"].as_array().into_iter().flatten() {
            if record["validity"] == "stale" {
                if let Some(rule_id) = record["finding_id"]
                    .as_str()
                    .and_then(|id| finding_rule.get(id))
                {
                    *stale_by_rule.entry(rule_id.clone()).or_default() += 1;
                }
            }
        }
        for finding in checked["diagnostics"]
            .as_array()
            .into_iter()
            .flatten()
            .chain(checked["reviewed"].as_array().into_iter().flatten())
        {
            if let Some(rule_id) = finding["rule_id"].as_str() {
                *raw_by_rule.entry(rule_id.to_owned()).or_default() += 1;
            }
        }
    }

    let mut rules_out = Vec::new();
    for package in selected {
        let id = package.qualified_id.clone();
        let mode = effective_mode(&workspace, package);
        let severity = package.manifest.severity.clone();
        let intent = rule::effective_intent(&package.manifest);
        let raw_findings = raw_by_rule.get(&id).copied().unwrap_or(0);
        let counts = review_by_rule.get(&id).copied().unwrap_or([0; 4]);
        let stale = stale_by_rule.get(&id).copied().unwrap_or(0);
        let decided = counts[0] + counts[1] + counts[2];

        let prepared_scope = PreparedScope::new(&workspace.root, package)?;
        let scoped_files = eligible_paths
            .iter()
            .copied()
            .filter(|path| prepared_scope.matches(path))
            .count();
        let unmatched_include = if prepared_scope.applicable {
            package
                .manifest
                .scope
                .include
                .iter()
                .filter(|glob| {
                    GlobBuilder::new(glob)
                        .literal_separator(true)
                        .build()
                        .map(|glob| glob.compile_matcher())
                        .is_ok_and(|matcher| {
                            !eligible_paths
                                .iter()
                                .copied()
                                .any(|path| matcher.is_match(path))
                        })
                })
                .cloned()
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };

        let signal = if mode == "disabled" {
            "disabled"
        } else if !complete {
            "unknown"
        } else if counts[1] > 0 {
            "useful"
        } else if scoped_files == 0 {
            "dead"
        } else if intent == "watch" && (raw_findings > 0 || decided > 0 || counts[3] > 0) {
            "watch"
        } else if raw_findings == 0 && decided == 0 && counts[3] == 0 {
            "quiet"
        } else if decided > 0 && counts[0] * 2 > decided {
            "noisy"
        } else {
            "active"
        };
        rules_out.push(json!({
            "id": id,
            "mode": mode,
            "severity": severity,
            "intent": intent,
            "scoped_files": scoped_files,
            "unmatched_include": unmatched_include,
            "raw_findings": if complete { json!(raw_findings) } else { Value::Null },
            "review_decisions": {"acceptable": counts[0], "confirmed_issue": counts[1],
                "accepted_risk": counts[2], "needs_review": counts[3]},
            "stale_decisions": if complete { json!(stale) } else { Value::Null },
            "signal": signal,
        }));
    }
    rules_out.sort_by(|left, right| {
        signal_rank(left["signal"].as_str().unwrap_or(""))
            .cmp(&signal_rank(right["signal"].as_str().unwrap_or("")))
            .then_with(|| left["id"].as_str().cmp(&right["id"].as_str()))
    });
    let mut signals = Map::new();
    for signal in [
        "dead", "noisy", "active", "useful", "watch", "quiet", "disabled", "unknown",
    ] {
        let count = rules_out
            .iter()
            .filter(|rule| rule["signal"] == signal)
            .count();
        signals.insert(signal.to_owned(), json!(count));
    }
    Ok(envelope(
        "stats",
        if complete { "pass" } else { "incomplete" },
        if complete { 0 } else { 2 },
        json!({
            "complete": complete,
            "root": workspace.root,
            "scope": checked["scope"],
            "rules": rules_out,
            "signals": signals,
            "errors": checked["errors"],
        }),
    ))
}

fn signal_rank(signal: &str) -> u8 {
    // Problems worth fixing sort first (dead scope, noisy thresholds, an
    // unsettled pattern, a confirmed issue to act on); healthy, working
    // states (`watch`, `quiet`) sort after them; statuses that cannot be
    // acted on directly (`unknown` needs the check fixed first, `disabled`
    // was opted out on purpose) sort last, `unknown` just above `disabled` as
    // in the original order.
    match signal {
        "dead" => 0,
        "noisy" => 1,
        "active" => 2,
        "useful" => 3,
        "watch" => 4,
        "quiet" => 5,
        "unknown" => 6,
        "disabled" => 7,
        _ => 8,
    }
}

struct FileExecution<'a> {
    scopes: BTreeMap<String, PreparedScope>,
    key_prefixes: BTreeMap<String, String>,
    programs: &'a [(&'a RulePackage, wt_runtime::Program)],
    cache: &'a mut cache::Store,
    pool: Option<crate::worker::Pool>,
    arena: QueryArena,
    optimized: bool,
    limits: wt_runtime::RuntimeLimits,
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
                        self.limits,
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
                    limits: self.limits,
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
                        merge_runtime_stats(
                            &mut self.stats["compilation"],
                            &result.stats["compilation"],
                        );
                        merge_runtime_stats(
                            &mut self.stats["transport"],
                            &result.stats["transport"],
                        );
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
    let optimized = workspace.effective_config.optimizer.mode != "off";
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
        json!({"rules": rules, "queries":queries,"plan_digest":plan_digest,"optimizer": workspace.effective_config.optimizer.mode, "executed": false}),
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
        match run_fixtures_with_limits(
            package,
            &program,
            options,
            workspace
                .effective_config
                .runtime
                .limits(workspace.effective_config.scan.max_file_bytes),
            workspace.effective_config.runtime.memory(),
        ) {
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
        let outcome = run_fixtures_with_limits(
            package,
            &program,
            options,
            workspace
                .effective_config
                .runtime
                .limits(workspace.effective_config.scan.max_file_bytes),
            workspace.effective_config.runtime.memory(),
        )?;
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
        json!({"selection": selection.coverage, "gaps": selection.gaps, "rules": rules,
            "coverage_expectations": workspace.effective_config.expectations}),
    ))
}

fn show_config(options: &Value) -> Result<Value> {
    let workspace = discovery::discover(options)?;
    let mut fields = config::effective_value(&workspace.effective_config);
    fields["configured"] = json!({"global":workspace.global_config,"local":workspace.local_config});
    fields["effective"] = config::effective_value(&workspace.effective_config);
    Ok(envelope("config", "pass", 0, fields))
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
    if matches!(name.as_str(), "rule" | "submission") {
        return crate::package_schema(&name, crate::CONTRACT_VERSION);
    }
    let schema = match name.as_str() {
        "rule" => include_str!("../../../schemas/rule.schema.json"),
        "submission" => include_str!("../../../schemas/submission.schema.json"),
        "tests" => include_str!("../../../schemas/tests.schema.json"),
        "config" => include_str!("../../../schemas/config.schema.json"),
        "result" => include_str!("../../../schemas/result.schema.json"),
        "plan" => include_str!("../../../schemas/plan.schema.json"),
        "review" => include_str!("../../../schemas/review.schema.json"),
        "capabilities" => include_str!("../../../schemas/capabilities.schema.json"),
        _ => bail!("unknown schema {name:?}"),
    };
    crate::parse_json(schema)
}

const AUTHOR_GUIDE: &str = "Choose the cheapest reliable protection first. When WT is useful, state exactly what the detector recognizes in rule.md. Submit JSON with documentation.source, code.source, raw-positive and raw-negative fixtures. New/update validate, format and test before storage. An acceptable review signal is still a raw-positive fixture; do not narrow a detector merely to make it disappear. Run wt check to inspect actual scope and findings. Optional intent: watch (default detect) marks a rule whose purpose is forcing re-review whenever specific code changes, not finding new occurrences; findings and repeated acceptances are its expected steady state, and wt stats reports it as watch instead of noisy.";
const REVIEW_GUIDE: &str = "Inspect the raw occurrence and its rule contract before deciding. wt inspect FINDING_ID evaluates current source and retained rationale. For contextual review signals use wt review FINDING_ID --decision acceptable --expect-evidence HASH --reason-file PATH. Use accepted-risk for a deliberately retained violation. Declare supporting evidence with --watch PATH=sha256:HASH. Fresh source/rule/dependency changes reopen acceptances. No bulk approval is available.";
const LANGUAGE_GUIDE: &str = "wt-rule-1 is a restricted top-level statement body. Use declared static pattern names with rx::find_all(file, \"pattern\") and emit(matched.span, \"diagnostic\"); text.v1 and ast.v1 are explicit capabilities. ast.v1 adds file.ast_match(language, pattern) for structural matches, m.node(\"NAME\") for a captured metavariable, and m.ast_match(language, pattern) to search inside a match; a file with any parse error node is an analysis gap, never a silent no-match. Only finite WT sequences can be iterated; detector programs cannot access the filesystem, external commands, or review state. Use wt plan for query inspection.";
const STATS_GUIDE: &str = "wt stats runs a check with the same scope and flags as wt check, then reports per qualified rule: mode, severity, intent, how many repository files are in scope after the same include/exclude and ignore handling (scoped_files), any of the rule's own include globs that individually match none of them (unmatched_include, informational), raw findings from that run, stored review decisions by outcome (acceptable, confirmed_issue, accepted_risk, needs_review), and how many of those decisions are currently stale. It keeps no history; every number is recomputed from the current repository, rules and review store. Each rule gets exactly one derived signal, in this priority order: disabled (mode is disabled), unknown (the underlying check was incomplete; never reported as dead), useful (at least one confirmed_issue decision), dead (scoped_files is zero; the rule cannot fire here), watch (an intent: watch rule with current findings or decisions), quiet (scope matches files, no current findings, no pending decisions: a healthy, working rule), noisy (more than half of decided findings were accepted as acceptable), otherwise active. Run it before adding many new rules, or when a check has become noisy.";
fn guide(options: &Value) -> Result<Value> {
    let topic = options
        .get("topic")
        .and_then(Value::as_str)
        .unwrap_or("author");
    let text = match topic {
        "author" => AUTHOR_GUIDE,
        "review" => REVIEW_GUIDE,
        "language" => LANGUAGE_GUIDE,
        "stats" => STATS_GUIDE,
        _ => bail!(
            "unknown installed guide topic {topic:?}; choose author, review, language or stats"
        ),
    };
    Ok(envelope(
        "guide",
        "pass",
        0,
        json!({"topic":topic,"text":text,
        "digest":crate::digest::digest_bytes(text.as_bytes())}),
    ))
}

fn capabilities() -> Result<Value> {
    Ok(
        json!({"schema_version":crate::CONTRACT_VERSION,"command":"capabilities","build":{"version":env!("CARGO_PKG_VERSION"),"commit":option_env!("WT_BUILD_COMMIT").unwrap_or("unknown")},
        "features":["readable_rules","occurrence_reviews","coverage_expectations","compact_result_protocol","candidate_preview","configurable_logical_budgets"],
        "schemas":{"rule":[crate::CONTRACT_VERSION],"submission":[crate::CONTRACT_VERSION],"config":[crate::CONTRACT_VERSION],"tests":[crate::CONTRACT_VERSION],"review":[crate::CONTRACT_VERSION],"result":[crate::CONTRACT_VERSION],"plan":[crate::CONTRACT_VERSION],"capabilities":[crate::CONTRACT_VERSION]},
        "language":"wt-rule-1", "helpers":["text.v1","ast.v1"],
        "ast":{"engine":wt_runtime::AST_ENGINE,"languages":wt_runtime::ast_supported_language_grammars().into_iter().map(|(language,grammar)| json!({"language":language,"grammar":grammar})).collect::<Vec<_>>()},
        "commands":["init","capabilities","guide","new","update","check","stats","fmt","plan","list","show","validate","test","review","reviews","inspect","set-mode","explain","config","schema","cache clear"],
        "guide_digests":{
            "author":crate::digest::digest_bytes(AUTHOR_GUIDE.as_bytes()),
            "review":crate::digest::digest_bytes(REVIEW_GUIDE.as_bytes()),
            "language":crate::digest::digest_bytes(LANGUAGE_GUIDE.as_bytes()),
            "stats":crate::digest::digest_bytes(STATS_GUIDE.as_bytes())
        },
        "configuration_keys":["scan.respect_gitignore","scan.honor_git_local_excludes","scan.include_hidden","scan.max_file_bytes","scan.exclude","runtime.file_steps","runtime.file_native_bytes","runtime.repository_steps","runtime.repository_native_bytes","runtime.worker_memory_bytes","runtime.parent_memory_bytes","runtime.total_memory_bytes","optimizer.mode","rules.mode_overrides","coverage.expectations"],
        "check_options":{"optimizer":["auto","off"],"maximum_jobs":3,"maximum_file_bytes":67108864},
        "resource_support":{"platform":std::env::consts::OS,"worker_processes":true,"hard_worker_memory_limit":cfg!(target_os="linux"),
            "worker_reservation_bytes":crate::worker::WORKER_MEMORY_RESERVATION_BYTES,
            "parent_reservation_bytes":crate::worker::PARENT_MEMORY_RESERVATION_BYTES,
            "combined_scheduling_budget_bytes":crate::worker::MEMORY_BUDGET_BYTES,
            "runtime_ceilings":crate::config::RuntimeConfig::default()}}),
    )
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
    options: &Value,
) -> Result<Vec<Value>> {
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
    ordered
        .iter()
        .map(|record| diagnostic_value(record, workspace, options))
        .collect::<Result<Vec<_>>>()
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
    object.insert("schema_version".to_owned(), json!(crate::CONTRACT_VERSION));
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
    fs::write(
        temp.path().join("rule.md"),
        submission
            .documentation
            .source
            .as_deref()
            .unwrap_or_default(),
    )?;
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
    let lock_path = crate::coordination::lock_path(
        "package",
        &format!("{}:{id}", parent.canonicalize()?.display()),
    )?;
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

fn run_fixtures_with_limits(
    package: &RulePackage,
    program: &wt_runtime::Program,
    options: &Value,
    limits: wt_runtime::RuntimeLimits,
    memory: crate::worker::MemoryProfile,
) -> Result<fixtures::TestOutcome> {
    let Some(executable) = options.get("_worker_executable").and_then(Value::as_str) else {
        return fixtures::run_suite_with(package, |files| {
            program.execute_with_limits(files, &mut QueryArena::new(true), limits)
        });
    };
    let mut pool = crate::worker::Pool::with_memory(Path::new(executable), 1, memory)?;
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
            limits,
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
    let check_options = Value::Object(check_options);
    let checked = check(&check_options)?;
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
        bail!("finding is absent or ambiguous; run check again");
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
    reviews::record(&workspace.root, finding, options, || {
        // Re-evaluate this rule's full source scope under the per-record lock.
        // Other independent rules were validated by the first check. Repository
        // rules still rehash their entire authorized input inventory.
        let mut recheck = check_options.clone();
        recheck["rules"] = json!([finding["rule_id"]]);
        let fresh = check(&recheck)?;
        if fresh["complete"] != true {
            bail!("review evidence became incomplete before publication");
        }
        let current = fresh["diagnostics"]
            .as_array()
            .into_iter()
            .flatten()
            .chain(fresh["reviewed"].as_array().into_iter().flatten())
            .filter(|candidate| candidate["finding_id"] == id)
            .collect::<Vec<_>>();
        if current.len() != 1 || current[0]["evidence_digest"] != finding["evidence_digest"] {
            bail!("review evidence changed before publication; run check again");
        }
        Ok(())
    })
}

fn inspect(options: &Value) -> Result<Value> {
    let id = required_string(options, "finding_id")?;
    let mut check_options = Map::new();
    for key in COMMON_OPTIONS
        .iter()
        .copied()
        .chain(["no_global", "no_host_ignores"])
    {
        if let Some(value) = options.get(key) {
            check_options.insert(key.to_owned(), value.clone());
        }
    }
    check_options.insert("no_cache".to_owned(), json!(true));
    let checked = check(&Value::Object(check_options))?;
    if checked["complete"] != true {
        bail!(
            "cannot inspect current evidence after incomplete analysis: {}",
            checked["errors"]
        );
    }
    let findings = checked["diagnostics"]
        .as_array()
        .into_iter()
        .flatten()
        .chain(checked["reviewed"].as_array().into_iter().flatten())
        .filter(|finding| finding["finding_id"] == id)
        .collect::<Vec<_>>();
    if findings.len() > 1 {
        bail!("ambiguous finding identity");
    }
    let workspace = discovery::discover(options)?;
    let previous = reviews::stored_detail(&workspace.root, &id)?;
    if findings.is_empty() && previous.is_none() {
        bail!("unknown finding ID {id}");
    }
    let current = findings.first().copied();
    let rule_id = current.map(|finding| &finding["rule_id"]).or_else(|| {
        previous
            .as_ref()
            .map(|previous| &previous["record"]["rule_id"])
    });
    let contract = workspace
        .packages
        .iter()
        .find(|package| Some(package.qualified_id.as_str()) == rule_id.and_then(Value::as_str))
        .map(|package| {
            if current.is_some_and(|finding| finding["rule_digest"] != package.digest) {
                bail!("rule package changed while inspecting; retry with fresh evidence")
            }
            rule::export_value(package)
        })
        .transpose()?
        .and_then(|value| value["documentation"]["source"].as_str().map(str::to_owned));
    if current.is_some()
        && !workspace
            .packages
            .iter()
            .any(|package| Some(package.qualified_id.as_str()) == rule_id.and_then(Value::as_str))
    {
        bail!("rule package disappeared while inspecting; retry with fresh evidence");
    }
    let reasons = current
        .and_then(|finding| finding["review_state"]["reasons"].as_array().cloned())
        .unwrap_or_default();
    let previous_diff = previous
        .as_ref()
        .map(|previous| crate::inspection::previous_diff(&workspace.root, previous, current))
        .transpose()?
        .flatten();
    Ok(envelope(
        "inspect",
        "pass",
        0,
        json!({
            "finding_id": id, "observation": if current.is_some() {"present"} else {"not_observed"},
            "finding": current, "previous": previous, "rule_markdown": contract,
            "stale_reasons": reasons,
            "previous_content": if previous_diff.is_some() {"verified_local_vcs_object"} else {"previous_content_unavailable"},
            "source_diff": previous_diff,
            "coverage": checked["rules"]
        }),
    ))
}
