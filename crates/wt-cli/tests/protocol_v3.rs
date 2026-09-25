use jsonschema::validator_for;
use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::process::{Command, Stdio};
use tempfile::tempdir;

fn run(root: &std::path::Path, args: &[&str], input: Option<&Value>) -> Value {
    let mut command = Command::new(env!("CARGO_BIN_EXE_wt"));
    command
        .args(args)
        .args([
            "--root",
            root.to_str().unwrap(),
            "--global-dir",
            root.join("global").to_str().unwrap(),
            "--format",
            "json",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped());
    let mut child = command.spawn().unwrap();
    if let Some(input) = input {
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.to_string().as_bytes())
            .unwrap();
    }
    let output = child.wait_with_output().unwrap();
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        output.status.code(),
        value["exit_code"]
            .as_i64()
            .map(|n| n as i32)
            .or_else(|| (value["command"] == "capabilities").then_some(0)),
        "{value}"
    );
    value
}

#[test]
fn installed_second_encounter_example_reopens_only_changed_evidence() {
    let root = tempdir().unwrap();
    let scenario: Value = serde_json::from_str(include_str!(
        "../../../examples/second-encounter.scenarios.json"
    ))
    .unwrap();
    let submission: Value = serde_json::from_str(include_str!(
        "../../../examples/review-legacy-endpoint.json"
    ))
    .unwrap();
    assert_eq!(scenario["submission"], "review-legacy-endpoint.json");
    let created = run(root.path(), &["new", "--stdin"], Some(&submission));
    assert_eq!(created["exit_code"], 0, "{created}");
    fs::create_dir(root.path().join("src")).unwrap();
    fs::write(
        root.path().join("src/compat.txt"),
        "Keep /v1/legacy for compatibility.\n",
    )
    .unwrap();
    let routing = "legacy remains mapped\n";
    fs::write(root.path().join("src/routing.json"), routing).unwrap();
    let first = run(root.path(), &["check", "--no-global", "--no-cache"], None);
    assert_eq!(first["summary"]["raw_occurrences"], 1, "{first}");
    let finding = &first["diagnostics"][0];
    let reason = root.path().join("reason.md");
    fs::write(
        &reason,
        "# Accepted\n\nRouting retains this compatibility note.",
    )
    .unwrap();
    let watch = format!("src/routing.json={}", wt_core::digest_text(routing));
    let decision = run(
        root.path(),
        &[
            "review",
            finding["finding_id"].as_str().unwrap(),
            "--decision",
            "acceptable",
            "--expect-evidence",
            finding["evidence_digest"].as_str().unwrap(),
            "--watch",
            &watch,
            "--reason-file",
            reason.to_str().unwrap(),
            "--no-global",
        ],
        None,
    );
    assert_eq!(decision["exit_code"], 0, "{decision}");
    let unchanged = run(
        root.path(),
        &["check", "--no-global", "--detail", "full"],
        None,
    );
    assert_eq!(
        unchanged["summary"]["reviewed_occurrences"], 1,
        "{unchanged}"
    );
    assert_eq!(unchanged["summary"]["actionable_occurrences"], 0);
    fs::write(
        root.path().join("src/copy.txt"),
        "Keep /v1/legacy for compatibility.\n",
    )
    .unwrap();
    let copy = run(
        root.path(),
        &["check", "--no-global", "--detail", "full"],
        None,
    );
    assert_eq!(copy["summary"]["reviewed_occurrences"], 1, "{copy}");
    assert_eq!(copy["summary"]["actionable_occurrences"], 1);
    assert_ne!(copy["diagnostics"][0]["finding_id"], finding["finding_id"]);
    fs::write(root.path().join("src/routing.json"), "legacy unmapped\n").unwrap();
    let dependency = run(root.path(), &["check", "--no-global"], None);
    assert!(
        dependency["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["finding_id"] == finding["finding_id"]
                && item["review_state"]["reasons"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|reason| reason == "watched_file_changed:src/routing.json")),
        "{dependency}"
    );
    fs::write(
        root.path().join("src/compat.txt"),
        "Keep /v1/legacy for compatibility.\nRevisit this note.\n",
    )
    .unwrap();
    let owner = run(root.path(), &["check", "--no-global"], None);
    assert!(
        owner["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["finding_id"] == finding["finding_id"]
                && item["review_state"]["reasons"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("owner_file_changed"))),
        "{owner}"
    );
    fs::remove_file(root.path().join("src/compat.txt")).unwrap();
    let missing = run(
        root.path(),
        &["check", "--no-global", "--detail", "full"],
        None,
    );
    assert!(
        missing["review_records"]
            .as_array()
            .unwrap()
            .iter()
            .any(|record| record["finding_id"] == finding["finding_id"]
                && record["validity"] == "not_observed"),
        "{missing}"
    );
}

#[test]
fn configured_execution_limits_invalidate_warm_results_and_cli_optimizer_wins() {
    let root = tempdir().unwrap();
    let schema: Value =
        serde_json::from_str(include_str!("../../../schemas/result-v3.schema.json")).unwrap();
    let validator = validator_for(&schema).unwrap();
    let submission = json!({"schema_version":3,"id":"budget-marker","title":"Budget marker","mode":"advisory","severity":"info","scope":{"include":["**/*.txt"]},
        "patterns":{"mark":"bad"},"diagnostics":{"hit":{"kind":"review","message":"Review marker","help":"Inspect"}},
        "documentation":{"source":"# Intended constraint\nReview marker."},
        "code":{"language":"wt-rule-1","capabilities":["regex.v1"],"source":"for m in rx::find_all(file, \"mark\") { emit(m.span, \"hit\"); }"}});
    assert_eq!(
        run(root.path(), &["new", "--stdin"], Some(&submission))["exit_code"],
        0
    );
    fs::write(
        root.path().join("note.txt"),
        format!("bad {}", root.path().display()),
    )
    .unwrap();
    let warm = run(root.path(), &["check", "--no-global", "--stats"], None);
    assert_eq!(warm["exit_code"], 0, "{warm}");
    assert!(
        warm["stats"]["runtime"]["compilation"]["programs_compiled"]
            .as_u64()
            .unwrap()
            > 0,
        "{warm}"
    );
    assert!(
        warm["stats"]["runtime"]["transport"]["request_bytes"]
            .as_u64()
            .unwrap()
            > 0,
        "{warm}"
    );
    let cached = run(root.path(), &["check", "--no-global", "--stats"], None);
    assert_eq!(cached["stats"]["cache"]["raw_hits"], 1, "{cached}");
    let legacy = run(
        root.path(),
        &[
            "check",
            "--no-global",
            "--output-version",
            "2",
            "--detail",
            "full",
            "--stats",
        ],
        None,
    );
    let legacy_schema: Value =
        serde_json::from_str(include_str!("../../../schemas/result.schema.json")).unwrap();
    assert!(
        validator_for(&legacy_schema).unwrap().is_valid(&legacy),
        "{legacy}"
    );

    fs::write(
        root.path().join(".wt/config.json"),
        json!({"schema_version":3,
        "runtime":{"file_native_bytes":1}, "optimizer":{"mode":"off"}})
        .to_string(),
    )
    .unwrap();
    let config = run(root.path(), &["config", "--no-global"], None);
    assert_eq!(
        config["data"]["effective"]["runtime"]["file_native_bytes"],
        1
    );
    assert_eq!(config["data"]["effective"]["optimizer"]["mode"], "off");
    let strict = run(root.path(), &["check", "--no-global", "--stats"], None);
    assert!(validator.is_valid(&strict), "{strict}");
    assert_eq!(strict["exit_code"], 2, "{strict}");
    assert!(strict["errors"].to_string().contains("native"), "{strict}");
    assert_eq!(strict["stats"]["cache"]["raw_hits"], 0, "{strict}");
    assert_eq!(strict["effective_policy"]["optimizer"]["mode"], "off");

    let override_check = run(
        root.path(),
        &["check", "--no-global", "--optimizer", "auto"],
        None,
    );
    assert_eq!(
        override_check["effective_policy"]["optimizer"]["mode"],
        "auto"
    );
    assert_eq!(override_check["exit_code"], 2, "{override_check}");
    let plan = run(root.path(), &["plan", "--no-global"], None);
    assert_eq!(plan["data"]["optimizer"], "off");
    let legacy = run(
        root.path(),
        &[
            "check",
            "--no-global",
            "--output-version",
            "2",
            "--detail",
            "full",
        ],
        None,
    );
    assert_eq!(legacy["exit_code"], 2, "{legacy}");
    assert!(legacy["error"].as_str().unwrap().contains("profile"));

    fs::write(
        root.path().join(".wt/config.json"),
        json!({"schema_version":3,
        "runtime":{"total_memory_bytes":536870912}})
        .to_string(),
    )
    .unwrap();
    let one_worker = run(
        root.path(),
        &["check", "--no-global", "--stats", "--no-cache"],
        None,
    );
    assert!(validator.is_valid(&one_worker), "{one_worker}");
    assert_eq!(one_worker["exit_code"], 0, "{one_worker}");
    assert_eq!(one_worker["stats"]["runtime"]["admitted_jobs"], 1);
    assert_eq!(
        one_worker["stats"]["runtime"]["total_scheduling_bytes"],
        536870912
    );
    let rejected = run(root.path(), &["check", "--no-global", "--jobs", "2"], None);
    assert_eq!(rejected["exit_code"], 2, "{rejected}");
    assert!(rejected["errors"]
        .to_string()
        .contains("configured memory capacity"));
}

#[test]
fn default_protocol_is_compact_and_full_is_explicit() {
    let root = tempdir().unwrap();
    let schema: Value =
        serde_json::from_str(include_str!("../../../schemas/result-v3.schema.json")).unwrap();
    let validator = validator_for(&schema).unwrap();
    let submission = json!({"schema_version":3,"id":"marker","title":"Marker","mode":"advisory","severity":"info",
        "scope":{"include":["**/*.txt"]},"patterns":{"word":"bad"},
        "diagnostics":{"hit":{"kind":"review","message":"Inspect marker","help":"Inspect context"}},
        "documentation":{"source":"# Intended constraint\nReview marker."},
        "code":{"language":"wt-rule-1","capabilities":["regex.v1"],"source":"for m in rx::find_all(file, \"word\") { emit(m.span, \"hit\"); }"}});
    let created = run(root.path(), &["new", "--stdin"], Some(&submission));
    assert!(validator.is_valid(&created), "{created}");
    assert_eq!(created["data"]["qualified_id"], "local/marker");
    fs::write(root.path().join("note.txt"), "bad").unwrap();
    let compact = run(root.path(), &["check", "--no-global"], None);
    assert!(validator.is_valid(&compact), "{compact}");
    assert_eq!(compact["diagnostics"].as_array().unwrap().len(), 1);
    assert!(compact.get("files").is_none());
    assert_eq!(compact["inventory"]["files"], false);
    assert_eq!(compact["summary"]["raw_occurrences"], 1);
    let full = run(
        root.path(),
        &["check", "--no-global", "--detail", "full"],
        None,
    );
    assert!(validator.is_valid(&full), "{full}");
    assert_eq!(full["files"].as_array().unwrap().len(), 1);
    assert_eq!(full["diagnostics"], compact["diagnostics"]);
    let stats = run(root.path(), &["check", "--no-global", "--stats"], None);
    assert!(stats["stats"]["measurement"]["elapsed_ms"].is_number());
    assert_eq!(stats["stats"]["measurement"]["review_evidence_reads"], 0);
    assert_eq!(
        stats["stats"]["measurement"]["peak_rss_bytes"],
        "not_measured"
    );
    let reason = root.path().join("reason.md");
    fs::write(&reason, "This marker is an approved example in this note.").unwrap();
    let finding = &compact["diagnostics"][0];
    let reviewed = run(
        root.path(),
        &[
            "review",
            finding["finding_id"].as_str().unwrap(),
            "--decision",
            "acceptable",
            "--expect-evidence",
            finding["evidence_digest"].as_str().unwrap(),
            "--reason-file",
            reason.to_str().unwrap(),
            "--no-global",
        ],
        None,
    );
    assert_eq!(reviewed["exit_code"], 0, "{reviewed}");
    let accepted = run(root.path(), &["check", "--no-global"], None);
    assert!(validator.is_valid(&accepted), "{accepted}");
    assert!(accepted["diagnostics"].as_array().unwrap().is_empty());
    assert_eq!(accepted["summary"]["raw_occurrences"], 1);
    assert_eq!(accepted["summary"]["reviewed_occurrences"], 1);
    assert_eq!(accepted["inventory"]["reviewed"], false);
    let inspected = run(
        root.path(),
        &[
            "inspect",
            finding["finding_id"].as_str().unwrap(),
            "--no-global",
        ],
        None,
    );
    assert!(validator.is_valid(&inspected), "{inspected}");
    assert_eq!(
        inspected["data"]["previous"]["rationale"],
        "This marker is an approved example in this note."
    );
    fs::write(root.path().join("second.txt"), "bad").unwrap();
    let recurrence = run(
        root.path(),
        &["check", "--no-global", "--detail", "full"],
        None,
    );
    assert_eq!(recurrence["summary"]["raw_occurrences"], 2);
    assert_eq!(recurrence["summary"]["reviewed_occurrences"], 1);
    assert_eq!(recurrence["diagnostics"].as_array().unwrap().len(), 1);
    assert_ne!(
        recurrence["diagnostics"][0]["finding_id"],
        finding["finding_id"]
    );
    fs::write(root.path().join("note.txt"), "bad and edited").unwrap();
    let stale = run(
        root.path(),
        &["check", "--no-global", "--detail", "full"],
        None,
    );
    let reopened = stale["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["finding_id"] == finding["finding_id"])
        .unwrap();
    assert_eq!(reopened["review_state"]["validity"], "stale");
    assert!(reopened["review_state"]["reasons"]
        .as_array()
        .unwrap()
        .contains(&json!("owner_file_changed")));
    let capabilities = run(root.path(), &["capabilities"], None);
    assert_eq!(capabilities["schema_version"], 1);
    assert_eq!(capabilities["schemas"]["result"], json!([2, 3]));
    let capability_schema: Value =
        serde_json::from_str(include_str!("../../../schemas/capabilities.schema.json")).unwrap();
    assert!(validator_for(&capability_schema)
        .unwrap()
        .is_valid(&capabilities));
    assert_eq!(capabilities["specification_profile"]["target_revision"], 3);
    assert!(capabilities["build"]["commit"].as_str().is_some());
    let guide = run(root.path(), &["guide", "author"], None);
    assert_eq!(
        guide["data"]["digest"],
        capabilities["guide_digests"]["author"]
    );
    let config = run(
        root.path(),
        &["config", "--no-global", "--max-file-bytes", "2048"],
        None,
    );
    assert_eq!(config["data"]["effective"]["scan"]["max_file_bytes"], 2048);
    assert_eq!(config["data"]["configured"]["local"], Value::Null);
    let schema_output: Value = serde_json::from_slice(
        &Command::new(env!("CARGO_BIN_EXE_wt"))
            .args(["schema", "result"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    assert_eq!(schema_output, schema);
    let submission_schema: Value = serde_json::from_slice(
        &Command::new(env!("CARGO_BIN_EXE_wt"))
            .args(["schema", "submission"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    assert_eq!(
        submission_schema["properties"]["schema_version"]["const"],
        3
    );
    assert!(validator_for(&submission_schema)
        .unwrap()
        .is_valid(&submission));
    let error = run(root.path(), &["show", "unknown-rule"], None);
    assert!(validator.is_valid(&error), "{error}");
    assert_eq!(error["exit_code"], 2);
    assert!(error["errors"][0]["error"]
        .as_str()
        .unwrap()
        .contains("unknown rule"));
    for (field, value, pointer) in [
        ("id", json!("Invalid/Name"), "/id"),
        (
            "scope",
            json!({"include":["../escaped.txt"]}),
            "/scope/include/0",
        ),
        (
            "diagnostics",
            json!({"bad key!":{"kind":"review","message":"Review","help":"Inspect"}}),
            "/diagnostics/bad key!",
        ),
    ] {
        let mut invalid = submission.clone();
        invalid[field] = value;
        let path = root.path().join("invalid-submission.json");
        fs::write(&path, invalid.to_string()).unwrap();
        let rejected = run(
            root.path(),
            &["validate", "--file", path.to_str().unwrap()],
            None,
        );
        assert_eq!(rejected["exit_code"], 2, "{rejected}");
        assert!(
            rejected["errors"].to_string().contains(pointer),
            "{rejected}"
        );
    }
}

#[test]
fn candidate_preview_is_isolated_and_legacy_projection_rejects_coverage_policy() {
    let root = tempdir().unwrap();
    let candidate = json!({"schema_version":3,"id":"draft","title":"Draft","mode":"advisory","severity":"info",
        "scope":{"include":["**/*.txt"]},"patterns":{"word":"bad"},
        "diagnostics":{"hit":{"kind":"review","message":"Inspect marker","help":"Inspect context"}},
        "documentation":{"source":"# Draft\nReview marker."},
        "code":{"language":"wt-rule-1","capabilities":["regex.v1"],"source":"for m in rx::find_all(file, \"word\") { emit(m.span, \"hit\"); }"}});
    let input = root.path().join("draft.json");
    fs::write(&input, candidate.to_string()).unwrap();
    fs::write(root.path().join("note.txt"), "bad").unwrap();
    let preview = run(
        root.path(),
        &[
            "check",
            "--submission",
            input.to_str().unwrap(),
            "--no-global",
        ],
        None,
    );
    assert_eq!(preview["preview"], true, "{preview}");
    assert_eq!(preview["diagnostics"][0]["rule_id"], "candidate/draft");
    assert!(!root.path().join(".wt/rules/draft").exists());
    assert!(!root.path().join(".wt/reviews").exists());
    fs::create_dir_all(root.path().join(".wt/rules/unfinished")).unwrap();
    let still_isolated = run(
        root.path(),
        &[
            "check",
            "--submission",
            input.to_str().unwrap(),
            "--no-global",
        ],
        None,
    );
    assert_eq!(
        still_isolated["diagnostics"][0]["rule_id"], "candidate/draft",
        "{still_isolated}"
    );
    let legacy = run(
        root.path(),
        &[
            "check",
            "--submission",
            input.to_str().unwrap(),
            "--no-global",
            "--output-version",
            "2",
        ],
        None,
    );
    assert_eq!(legacy["exit_code"], 2, "{legacy}");
    assert!(legacy["error"].as_str().unwrap().contains("protocol 2"));

    fs::create_dir_all(root.path().join(".wt")).unwrap();
    fs::write(root.path().join(".wt/config.json"), json!({"schema_version":3,"coverage":{"expectations":[{"rule_id":"local/draft","minimum_files":1,"reason":"Required"}]}}).to_string()).unwrap();
    let with_policy = run(
        root.path(),
        &[
            "check",
            "--submission",
            input.to_str().unwrap(),
            "--no-global",
        ],
        None,
    );
    assert_eq!(
        with_policy["coverage_expectations"][0]["status"], "not_evaluated_partial",
        "{with_policy}"
    );
    let legacy_policy = run(
        root.path(),
        &[
            "check",
            "--submission",
            input.to_str().unwrap(),
            "--no-global",
            "--output-version",
            "2",
        ],
        None,
    );
    assert_eq!(legacy_policy["exit_code"], 2);
}
