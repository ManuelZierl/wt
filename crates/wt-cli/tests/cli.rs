use serde_json::{json, Value};
use std::process::{Command, Output, Stdio};
use tempfile::tempdir;

fn invoke(
    root: &std::path::Path,
    global: &std::path::Path,
    args: &[&str],
    input: Option<&str>,
) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_wt"));
    command
        .current_dir(root)
        .args(args)
        .args(if args.contains(&"check") {
            &["--detail", "full"][..]
        } else {
            &[]
        })
        .arg("--root")
        .arg(root)
        .arg("--global-dir")
        .arg(global)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(input) = input {
        command.stdin(Stdio::piped());
        let mut child = command.spawn().unwrap();
        use std::io::Write;
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    } else {
        command.output().unwrap()
    }
}

fn json_output(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "stdout was not JSON: {error}; stdout={:?}; stderr={:?}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn submission(id: &str, marker: &str, diagnostic: &str) -> String {
    json!({
        "schema_version": 1,
        "id": id,
        "title": id,
        "documentation": {"source": "# Shared query\n\n## Description\n\nA shared-query integration rule.\n\n## Rationale\n\nThe test needs an independent predicate over a common request query.\n\n## Limitations\n\nOnly text files are inspected.\n"},
        "mode": "advisory",
        "severity": "warning",
        "execution": "file",
        "scope": {"include": ["**/*.txt"]},
        "patterns": {"local_pattern": "request\\([^\\r\\n]*\\)"},
        "diagnostics": {diagnostic: {"kind": "violation", "message": "marker found", "help": "review it"}},
        "code": {"language": "wt-rule-1", "capabilities": ["text.v1"], "source": format!("for matched in rx::find_all(file, \"local_pattern\") {{ if matched.text.contains(\"{marker}\") {{ emit(matched.span, \"{diagnostic}\"); }} }}")},
        "tests": {
            "schema_version": 1,
            "cases": [
                {"name": "positive", "files": [{"path": "fixture.txt", "content": format!("request(\"{marker}\")") }], "expect": [{"path": "fixture.txt", "code": diagnostic, "kind": "violation"}]},
                {"name": "negative", "files": [{"path": "fixture.txt", "content": "request(\"other\")"}], "expect": []}
            ]
        }
    })
    .to_string()
}

#[test]
fn cli_creates_shows_tests_checks_and_shares_queries() {
    let root = tempdir().unwrap();
    let global = tempdir().unwrap();
    std::fs::write(root.path().join("source.txt"), "request(\"foo123\")\n").unwrap();
    let first = submission("shared-a", "foo", "contains-foo");
    let second = submission("shared-b", "foo123", "contains-foo123");

    for document in [&first, &second] {
        let output = invoke(
            root.path(),
            global.path(),
            &["new", "--stdin", "--format", "json"],
            Some(document),
        );
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(json_output(&output)["data"]["validation"], "passed");
    }

    let show = invoke(
        root.path(),
        global.path(),
        &["show", "local/shared-a", "--format", "json"],
        None,
    );
    assert_eq!(show.status.code(), Some(0));
    assert_eq!(
        json_output(&show)["data"]["rule"]["code"]["language"],
        "wt-rule-1"
    );

    let test = invoke(
        root.path(),
        global.path(),
        &["test", "shared-a", "--no-global", "--format", "json"],
        None,
    );
    assert_eq!(test.status.code(), Some(0));
    assert_eq!(json_output(&test)["data"]["rules"][0]["passed"], true);

    let check = invoke(
        root.path(),
        global.path(),
        &[
            "check",
            "--no-global",
            "--no-cache",
            "--stats",
            "--format",
            "json",
        ],
        None,
    );
    assert_eq!(
        check.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&check.stdout)
    );
    let result = json_output(&check);
    assert_eq!(result["diagnostics"].as_array().unwrap().len(), 2);
    assert_eq!(result["stats"]["runtime"]["logical_query_requests"], 4);
    assert_eq!(result["stats"]["runtime"]["regex_evaluations"], 1);
    assert_eq!(result["stats"]["runtime"]["text_evaluations"], 2);
    assert!(
        result["stats"]["runtime"]["shared_result_hits"]
            .as_u64()
            .unwrap()
            >= 1
    );
}

// Optimizer mode is deliberately varied across these runs to prove it does
// not change the semantic finding set; strip the fields that legitimately
// differ (stats and the effective policy, which echoes the chosen
// optimizer) before comparing.
fn semantic_check_result(value: &Value) -> Value {
    let mut result = value.clone();
    let object = result.as_object_mut().unwrap();
    object.remove("stats");
    object.remove("effective_policy");
    object.remove("policy_digest");
    if let Some(summary) = object.get_mut("summary").and_then(Value::as_object_mut) {
        summary.remove("elapsed_ms");
    }
    result
}

#[test]
fn serial_parallel_and_reference_checks_share_semantics_and_read_each_file_once() {
    let root = tempdir().unwrap();
    let global = tempdir().unwrap();
    std::fs::write(root.path().join("source.txt"), "request(\"foo123\")\n").unwrap();
    for document in [
        submission("schedule-a", "foo", "contains-foo"),
        submission("schedule-b", "foo123", "contains-foo123"),
    ] {
        let created = invoke(
            root.path(),
            global.path(),
            &["new", "--stdin", "--format", "json"],
            Some(&document),
        );
        assert_eq!(created.status.code(), Some(0), "{created:?}");
    }

    let serial = invoke(
        root.path(),
        global.path(),
        &[
            "check",
            "--no-global",
            "--no-cache",
            "--stats",
            "--jobs",
            "1",
            "--optimizer",
            "auto",
            "--format",
            "json",
        ],
        None,
    );
    let parallel = invoke(
        root.path(),
        global.path(),
        &[
            "check",
            "--no-global",
            "--no-cache",
            "--stats",
            "--jobs",
            "2",
            "--optimizer",
            "auto",
            "--format",
            "json",
        ],
        None,
    );
    let reference = invoke(
        root.path(),
        global.path(),
        &[
            "check",
            "--no-global",
            "--no-cache",
            "--stats",
            "--jobs",
            "1",
            "--optimizer",
            "off",
            "--format",
            "json",
        ],
        None,
    );
    for output in [&serial, &parallel, &reference] {
        assert_eq!(
            output.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
    let serial = json_output(&serial);
    let parallel = json_output(&parallel);
    let reference = json_output(&reference);
    assert_eq!(
        semantic_check_result(&serial),
        semantic_check_result(&parallel)
    );
    assert_eq!(
        semantic_check_result(&serial),
        semantic_check_result(&reference)
    );
    for result in [&serial, &parallel, &reference] {
        assert_eq!(result["stats"]["runtime"]["source_reads"], 1);
    }
    assert_eq!(serial["stats"]["runtime"]["regex_evaluations"], 1);
    assert_eq!(parallel["stats"]["runtime"]["regex_evaluations"], 1);
    assert_eq!(reference["stats"]["runtime"]["regex_evaluations"], 2);
}

#[test]
fn enforced_findings_exit_one_and_text_has_actionable_labels() {
    let root = tempdir().unwrap();
    let global = tempdir().unwrap();
    std::fs::write(root.path().join("source.txt"), "request(\"foo\")\n").unwrap();
    let document = submission("blocking-rule", "foo", "contains-foo");
    let created = invoke(
        root.path(),
        global.path(),
        &["new", "--stdin", "--format", "json"],
        Some(&document),
    );
    assert_eq!(created.status.code(), Some(0));
    let mode = invoke(
        root.path(),
        global.path(),
        &[
            "set-mode",
            "local/blocking-rule",
            "enforced",
            "--reason",
            "reviewed",
            "--no-global",
            "--format",
            "json",
        ],
        None,
    );
    assert_eq!(
        mode.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&mode.stdout)
    );

    let check = invoke(
        root.path(),
        global.path(),
        &["check", "--no-global", "--format", "text"],
        None,
    );
    assert_eq!(check.status.code(), Some(1));
    let text = String::from_utf8(check.stdout).unwrap();
    assert!(text.contains("error[local/blocking-rule]"));
    assert!(text.contains("= help: review it"));
    assert!(text.contains("1 blocking"));
}

#[test]
fn malformed_and_invalid_json_invocations_are_single_json_documents() {
    let root = tempdir().unwrap();
    let global = tempdir().unwrap();

    let missing = invoke(
        root.path(),
        global.path(),
        &["new", "--format", "json"],
        None,
    );
    assert_eq!(missing.status.code(), Some(2));
    assert_eq!(json_output(&missing)["exit_code"], 2);
    assert!(missing.stderr.is_empty());

    let malformed = invoke(
        root.path(),
        global.path(),
        &["new", "--stdin", "--format", "json"],
        Some("{\"schema_version\": 2,}"),
    );
    assert_eq!(malformed.status.code(), Some(2));
    assert!(json_output(&malformed)["errors"][0]["error"]
        .as_str()
        .unwrap()
        .contains("strict JSON"));

    let clap_failure = invoke(
        root.path(),
        global.path(),
        &["new", "--stdin", "--file", "rule.json", "--format", "json"],
        None,
    );
    assert_eq!(clap_failure.status.code(), Some(2));
    assert_eq!(json_output(&clap_failure)["exit_code"], 2);
    assert!(clap_failure.stderr.is_empty());
}

#[test]
fn update_requires_current_hash_and_preserves_explicit_test_removal_reason() {
    let root = tempdir().unwrap();
    let global = tempdir().unwrap();
    let initial = submission("retained-rule", "foo", "contains-foo");
    let created = invoke(
        root.path(),
        global.path(),
        &["new", "--stdin", "--format", "json"],
        Some(&initial),
    );
    assert_eq!(
        created.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&created.stdout)
    );
    let digest = json_output(&created)["data"]["digest"]
        .as_str()
        .unwrap()
        .to_owned();

    let mut replacement: Value = serde_json::from_str(&initial).unwrap();
    replacement["title"] = json!("Updated retained rule");
    replacement["tests"]["cases"].as_array_mut().unwrap().pop();
    let replacement = replacement.to_string();
    let without_permission = invoke(
        root.path(),
        global.path(),
        &[
            "update",
            "local/retained-rule",
            "--stdin",
            "--expect-hash",
            &digest,
            "--format",
            "json",
        ],
        Some(&replacement),
    );
    assert_eq!(without_permission.status.code(), Some(2));
    assert!(json_output(&without_permission)["errors"][0]["error"]
        .as_str()
        .unwrap()
        .contains("test removal"));

    let updated = invoke(
        root.path(),
        global.path(),
        &[
            "update",
            "local/retained-rule",
            "--stdin",
            "--expect-hash",
            &digest,
            "--allow-test-removal",
            "--reason",
            "The old negative case was replaced by the reviewed contract.",
            "--format",
            "json",
        ],
        Some(&replacement),
    );
    assert_eq!(
        updated.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&updated.stdout)
    );
    let updated_value = json_output(&updated);
    let new_digest = updated_value["data"]["digest"].as_str().unwrap();
    assert_ne!(new_digest, digest);

    let stale = invoke(
        root.path(),
        global.path(),
        &[
            "update",
            "local/retained-rule",
            "--stdin",
            "--expect-hash",
            &digest,
            "--format",
            "json",
        ],
        Some(&initial),
    );
    assert_eq!(stale.status.code(), Some(2));
    assert!(json_output(&stale)["errors"][0]["error"]
        .as_str()
        .unwrap()
        .contains("stale update hash"));
}

#[test]
fn global_and_local_same_ids_are_both_visible_and_short_ids_are_ambiguous() {
    let root = tempdir().unwrap();
    let global = tempdir().unwrap();
    std::fs::write(root.path().join("source.txt"), "request(\"foo\")\n").unwrap();
    let local = submission("same-rule", "foo", "local-hit");
    let global_submission = submission("same-rule", "foo", "global-hit");
    assert_eq!(
        invoke(
            root.path(),
            global.path(),
            &["new", "--stdin", "--format", "json"],
            Some(&local)
        )
        .status
        .code(),
        Some(0)
    );
    assert_eq!(
        invoke(
            root.path(),
            global.path(),
            &["new", "--global", "--stdin", "--format", "json"],
            Some(&global_submission)
        )
        .status
        .code(),
        Some(0)
    );

    let list = invoke(
        root.path(),
        global.path(),
        &["list", "--format", "json"],
        None,
    );
    assert_eq!(list.status.code(), Some(0));
    assert_eq!(
        json_output(&list)["data"]["rules"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    let ambiguous = invoke(
        root.path(),
        global.path(),
        &["show", "same-rule", "--format", "json"],
        None,
    );
    assert_eq!(ambiguous.status.code(), Some(2));
    assert!(json_output(&ambiguous)["errors"][0]["error"]
        .as_str()
        .unwrap()
        .contains("ambiguous"));

    let check = invoke(
        root.path(),
        global.path(),
        &["check", "--format", "json"],
        None,
    );
    assert_eq!(check.status.code(), Some(0));
    let diagnostics = json_output(&check)["diagnostics"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(diagnostics.len(), 2);
    assert!(diagnostics
        .iter()
        .any(|value| value["rule_id"] == "local/same-rule"));
    assert!(diagnostics
        .iter()
        .any(|value| value["rule_id"] == "global/same-rule"));

    let local_only = invoke(
        root.path(),
        global.path(),
        &["check", "--no-global", "--format", "json"],
        None,
    );
    assert_eq!(local_only.status.code(), Some(0));
    assert_eq!(
        json_output(&local_only)["diagnostics"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    let no_host = invoke(
        root.path(),
        global.path(),
        &[
            "check",
            "--no-global",
            "--no-host-ignores",
            "--format",
            "json",
        ],
        None,
    );
    assert_eq!(no_host.status.code(), Some(0));
    assert_eq!(json_output(&no_host)["scope"]["host_ignores"], false);
}

#[test]
fn plan_compiles_rules_without_reading_application_sources() {
    let root = tempdir().unwrap();
    let global = tempdir().unwrap();
    let document = submission("plan-only", "foo", "contains-foo");
    let created = invoke(
        root.path(),
        global.path(),
        &["new", "--stdin", "--format", "json"],
        Some(&document),
    );
    assert_eq!(created.status.code(), Some(0));
    std::fs::write(root.path().join("unreadable.txt"), [0xff, 0xfe]).unwrap();

    let plan = invoke(
        root.path(),
        global.path(),
        &[
            "plan",
            "--rule",
            "local/plan-only",
            "--no-global",
            "--format",
            "json",
        ],
        None,
    );
    assert_eq!(
        plan.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&plan.stdout)
    );
    let result = json_output(&plan);
    assert_eq!(result["data"]["executed"], false);
    assert_eq!(result["data"]["rules"].as_array().unwrap().len(), 1);
}

#[test]
fn schema_command_prints_the_bundled_schema_verbatim() {
    let root = tempdir().unwrap();
    let global = tempdir().unwrap();
    for (name, file) in [
        ("rule", "rule.schema.json"),
        ("submission", "submission.schema.json"),
        ("tests", "tests.schema.json"),
        ("config", "config.schema.json"),
        ("result", "result.schema.json"),
        ("plan", "plan.schema.json"),
    ] {
        let expected = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../schemas")
                .join(file),
        )
        .unwrap();
        let output = invoke(
            root.path(),
            global.path(),
            &["schema", name, "--format", "json"],
            None,
        );
        assert_eq!(output.status.code(), Some(0), "schema {name}");
        if matches!(name, "rule" | "submission") {
            let projected = json_output(&output);
            let expected_value: Value = serde_json::from_str(&expected).unwrap();
            assert_eq!(projected, expected_value, "schema {name}");
            continue;
        }
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            expected,
            "schema {name}"
        );
        assert!(output.stderr.is_empty());
    }
}

#[test]
fn decimal_reference_submission_validates_and_runs_all_retained_fixture_vectors() {
    let root = tempdir().unwrap();
    let global = tempdir().unwrap();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/template-number-decimal-step.json");
    let document = std::fs::read_to_string(path).unwrap();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/template-number-decimal-step.json");
    let path = path.to_str().unwrap();
    let validation = invoke(
        root.path(),
        global.path(),
        &["validate", "--file", path, "--format", "json"],
        None,
    );
    assert_eq!(
        validation.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&validation.stdout)
    );

    let created = invoke(
        root.path(),
        global.path(),
        &["new", "--stdin", "--format", "json"],
        Some(&document),
    );
    assert_eq!(
        created.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&created.stdout)
    );

    let tested = invoke(
        root.path(),
        global.path(),
        &[
            "test",
            "local/template-number-decimal-step",
            "--no-global",
            "--format",
            "json",
        ],
        None,
    );
    assert_eq!(
        tested.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&tested.stdout)
    );
    assert_eq!(json_output(&tested)["data"]["rules"][0]["passed"], true);
}

#[test]
fn minimal_submission_lifecycle_reports_untested_then_checks_successfully() {
    let root = tempdir().unwrap();
    let global = tempdir().unwrap();
    std::fs::write(root.path().join("source.txt"), "request(\"foo\")\n").unwrap();
    let mut minimal: Value =
        serde_json::from_str(&submission("minimal-lifecycle", "foo", "contains-foo")).unwrap();
    minimal.as_object_mut().unwrap().remove("tests");
    let created = invoke(
        root.path(),
        global.path(),
        &["new", "--stdin", "--format", "json"],
        Some(&minimal.to_string()),
    );
    assert_eq!(
        created.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&created.stdout)
    );
    assert_eq!(json_output(&created)["data"]["tests"], "untested");

    let tested = invoke(
        root.path(),
        global.path(),
        &[
            "test",
            "local/minimal-lifecycle",
            "--no-global",
            "--format",
            "json",
        ],
        None,
    );
    assert_eq!(tested.status.code(), Some(2));
    assert_eq!(json_output(&tested)["status"], "incomplete");

    let checked = invoke(
        root.path(),
        global.path(),
        &["check", "--no-global", "--no-cache", "--format", "json"],
        None,
    );
    assert_eq!(checked.status.code(), Some(0));
    assert_eq!(
        json_output(&checked)["diagnostics"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn stats_reports_a_quiet_rule_in_text_and_json() {
    let root = tempdir().unwrap();
    let global = tempdir().unwrap();
    // The rule's scope matches this file, but its pattern never fires on it:
    // scope matches files, fixtures pass, no findings — a healthy `quiet`
    // rule, not `dead`.
    std::fs::write(root.path().join("source.txt"), "request(\"other\")\n").unwrap();
    let created = invoke(
        root.path(),
        global.path(),
        &["new", "--stdin", "--format", "json"],
        Some(&submission("stats-example", "foo", "contains-foo")),
    );
    assert_eq!(created.status.code(), Some(0), "{created:?}");

    let json = invoke(
        root.path(),
        global.path(),
        &["stats", "--no-global", "--no-cache", "--format", "json"],
        None,
    );
    assert_eq!(json.status.code(), Some(0), "{json:?}");
    let result = json_output(&json);
    assert_eq!(result["command"], "stats");
    assert_eq!(result["data"]["complete"], true);
    let rules = result["data"]["rules"].as_array().unwrap();
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0]["id"], "local/stats-example");
    assert_eq!(rules[0]["signal"], "quiet");
    assert_eq!(rules[0]["scoped_files"], 1);
    assert_eq!(rules[0]["raw_findings"], 0);
    assert_eq!(result["data"]["signals"]["quiet"], 1);

    let text = invoke(
        root.path(),
        global.path(),
        &["stats", "--no-global", "--no-cache"],
        None,
    );
    assert_eq!(text.status.code(), Some(0), "{text:?}");
    let rendered = String::from_utf8_lossy(&text.stdout);
    assert!(rendered.contains("QUIET"));
    assert!(rendered.contains("local/stats-example"));
    assert!(rendered.contains("analysis complete"));
}

#[test]
fn stats_reports_a_dead_rule_and_an_unmatched_include_glob() {
    let root = tempdir().unwrap();
    let global = tempdir().unwrap();
    std::fs::write(root.path().join("source.txt"), "request(\"other\")\n").unwrap();
    // This scope only ever covers the synthetic fixture path, which the real
    // repository never has, so the rule cannot fire here.
    let dead = json!({
        "schema_version": 1,
        "id": "dead-example",
        "title": "dead-example",
        "documentation": {"source": "# Dead\n\n## Description\n\nNever applicable here.\n\n## Rationale\n\nTest fixture.\n\n## Limitations\n\nNone.\n"},
        "mode": "advisory",
        "severity": "warning",
        "execution": "file",
        "scope": {"include": ["fixture.txt"]},
        "patterns": {"needle": "foo"},
        "diagnostics": {"hit": {"kind": "violation", "message": "marker found", "help": "review it"}},
        "code": {"language": "wt-rule-1", "capabilities": ["text.v1"],
            "source": "for m in rx::find_all(file, \"needle\") { emit(m.span, \"hit\"); }"},
        "tests": {"schema_version": 1, "cases": [
            {"name": "positive", "files": [{"path": "fixture.txt", "content": "foo"}], "expect": [{"path": "fixture.txt", "code": "hit", "kind": "violation"}]},
            {"name": "negative", "files": [{"path": "fixture.txt", "content": "unrelated"}], "expect": []}
        ]}
    })
    .to_string();
    // This scope's second glob never matches anything real, even though the
    // first glob does; that is reported without changing the rule's signal.
    let partly_unmatched = json!({
        "schema_version": 1,
        "id": "partly-unmatched-example",
        "title": "partly-unmatched-example",
        "documentation": {"source": "# Partly unmatched\n\n## Description\n\nOne glob never matches.\n\n## Rationale\n\nTest fixture.\n\n## Limitations\n\nNone.\n"},
        "mode": "advisory",
        "severity": "warning",
        "execution": "file",
        "scope": {"include": ["**/*.txt", "**/*.nomatch"]},
        "patterns": {"needle": "foo"},
        "diagnostics": {"hit": {"kind": "violation", "message": "marker found", "help": "review it"}},
        "code": {"language": "wt-rule-1", "capabilities": ["text.v1"],
            "source": "for m in rx::find_all(file, \"needle\") { emit(m.span, \"hit\"); }"},
        "tests": {"schema_version": 1, "cases": [
            {"name": "positive", "files": [{"path": "fixture.txt", "content": "foo"}], "expect": [{"path": "fixture.txt", "code": "hit", "kind": "violation"}]},
            {"name": "negative", "files": [{"path": "fixture.txt", "content": "unrelated"}], "expect": []}
        ]}
    })
    .to_string();
    for document in [&dead, &partly_unmatched] {
        let created = invoke(
            root.path(),
            global.path(),
            &["new", "--stdin", "--format", "json"],
            Some(document),
        );
        assert_eq!(created.status.code(), Some(0), "{created:?}");
    }

    let json = invoke(
        root.path(),
        global.path(),
        &["stats", "--no-global", "--no-cache", "--format", "json"],
        None,
    );
    assert_eq!(json.status.code(), Some(0), "{json:?}");
    let result = json_output(&json);
    assert_eq!(result["data"]["complete"], true);
    let rules = result["data"]["rules"].as_array().unwrap();
    let dead_rule = rules
        .iter()
        .find(|rule| rule["id"] == "local/dead-example")
        .unwrap();
    assert_eq!(dead_rule["signal"], "dead");
    assert_eq!(dead_rule["scoped_files"], 0);
    assert_eq!(result["data"]["signals"]["dead"], 1);
    let unmatched_rule = rules
        .iter()
        .find(|rule| rule["id"] == "local/partly-unmatched-example")
        .unwrap();
    assert_ne!(unmatched_rule["signal"], "dead");
    assert_eq!(unmatched_rule["unmatched_include"], json!(["**/*.nomatch"]));

    let text = invoke(
        root.path(),
        global.path(),
        &["stats", "--no-global", "--no-cache"],
        None,
    );
    assert_eq!(text.status.code(), Some(0), "{text:?}");
    let rendered = String::from_utf8_lossy(&text.stdout);
    assert!(rendered.contains("DEAD"));
    assert!(rendered.contains("local/dead-example"));
    assert!(rendered.contains("unmatched include glob(s): **/*.nomatch"));
}
