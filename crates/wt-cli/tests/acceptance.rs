use jsonschema::validator_for;
use serde_json::{json, Value};
use std::ffi::OsString;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use tempfile::tempdir;

fn invoke(root: &Path, global: &Path, args: &[&str], input: Option<&str>) -> Output {
    let owned = args.iter().map(OsString::from).collect::<Vec<_>>();
    invoke_owned(root, global, &owned, input)
}

fn invoke_benchmark(root: &Path, global: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_wt"))
        .current_dir(root)
        .args(args)
        .args(["--detail", "full", "--root"])
        .arg(root)
        .arg("--global-dir")
        .arg(global)
        .env("XDG_CACHE_HOME", global)
        .output()
        .unwrap()
}

fn invoke_owned(root: &Path, global: &Path, args: &[OsString], input: Option<&str>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_wt"));
    command
        .current_dir(root)
        .args(args)
        .args(if args.iter().any(|arg| arg == "check") {
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

fn schema(name: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../schemas")
        .join(format!("{name}.schema.json"));
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn assert_schema(name: &str, instance: &Value) {
    let validator = validator_for(&schema(name))
        .unwrap_or_else(|error| panic!("schema {name} did not compile: {error}"));
    let errors = validator
        .iter_errors(instance)
        .map(|error| error.to_string())
        .collect::<Vec<_>>();
    assert!(errors.is_empty(), "{name} rejected output: {errors:?}");
}

fn assert_result(output: &Output) -> Value {
    let value = json_output(output);
    assert_schema("result", &value);
    value
}

fn text_submission(id: &str, mode: &str) -> Value {
    json!({
        "schema_version": 1,
        "id": id,
        "title": format!("Rule {id}"),
        "documentation": {"source": "# Marker\n\n## Description\n\nReports a marker in UTF-8 text files.\n\n## Rationale\n\nThe marker is retained as a deterministic acceptance fixture.\n\n## Limitations\n\nOnly UTF-8 text files matching the scope are inspected.\n"},
        "mode": mode,
        "severity": "warning",
        "execution": "file",
        "scope": {"include": ["**/*.txt"]},
        "patterns": {"marker": "bad"},
        "diagnostics": {"hit": {"kind": "violation", "message": "marker found", "help": "remove the marker"}},
        "code": {"language": "wt-rule-1", "capabilities": ["text.v1"], "source": "for matched in rx::find_all(file, \"marker\") { emit(matched.span, \"hit\"); }"},
        "tests": {
            "schema_version": 1,
            "cases": [
                {"name": "positive", "files": [{"path": "fixture.txt", "content": "bad\n"}], "expect": [{"path": "fixture.txt", "code": "hit", "kind": "violation"}]},
                {"name": "negative", "files": [{"path": "fixture.txt", "content": "clean\n"}], "expect": []}
            ]
        }
    })
}

fn create_rule(root: &Path, global: &Path, submission: &Value) -> Value {
    let output = invoke(
        root,
        global,
        &["new", "--stdin", "--format", "json"],
        Some(&submission.to_string()),
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_result(&output)
}

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn show_roundtrip_validates_submission_disk_manifest_tests_and_new_package() {
    let first_root = tempdir().unwrap();
    let first_global = tempdir().unwrap();
    let original = text_submission("roundtrip", "advisory");
    let created = create_rule(first_root.path(), first_global.path(), &original);
    assert_eq!(created["data"]["validation"], "passed");

    let show = invoke(
        first_root.path(),
        first_global.path(),
        &["show", "local/roundtrip", "--format", "json"],
        None,
    );
    assert_eq!(show.status.code(), Some(0));
    let shown = assert_result(&show);
    let exported = shown["data"]["rule"].clone();
    assert_schema("submission", &exported);

    let interchange = first_root.path().join("roundtrip.json");
    std::fs::write(&interchange, serde_json::to_vec_pretty(&exported).unwrap()).unwrap();
    let validated = invoke(
        first_root.path(),
        first_global.path(),
        &[
            "validate",
            "--file",
            interchange.to_str().unwrap(),
            "--format",
            "json",
        ],
        None,
    );
    assert_eq!(validated.status.code(), Some(0));
    let validated = assert_result(&validated);
    assert_eq!(validated["data"]["valid"], true);

    let disk_manifest = first_root.path().join(".wt/rules/roundtrip/rule.json");
    let disk_tests = first_root.path().join(".wt/rules/roundtrip/tests.json");
    let disk_manifest_value: Value =
        serde_json::from_slice(&std::fs::read(disk_manifest).unwrap()).unwrap();
    let disk_tests_value: Value =
        serde_json::from_slice(&std::fs::read(disk_tests).unwrap()).unwrap();
    assert_schema("rule", &disk_manifest_value);
    assert_schema("tests", &disk_tests_value);

    let second_root = tempdir().unwrap();
    let second_global = tempdir().unwrap();
    let recreated = create_rule(second_root.path(), second_global.path(), &exported);
    assert_eq!(recreated["data"]["validation"], "passed");
    let recreated_show = invoke(
        second_root.path(),
        second_global.path(),
        &["show", "local/roundtrip", "--format", "json"],
        None,
    );
    assert_eq!(recreated_show.status.code(), Some(0));
    assert_eq!(assert_result(&recreated_show)["data"]["rule"], exported);
}

#[test]
fn documented_skill_minimal_submission_is_present_and_runnable() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../skills/wt/references/minimal-submission.json");
    assert!(
        path.is_file(),
        "missing runnable skill reference: {}",
        path.display()
    );
    let submission: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_schema("submission", &submission);

    let root = tempdir().unwrap();
    let global = tempdir().unwrap();
    std::fs::write(root.path().join("notes.txt"), "TODO: finish this note\n").unwrap();
    create_rule(root.path(), global.path(), &submission);
    let tested = invoke(
        root.path(),
        global.path(),
        &[
            "test",
            "local/no-todo-marker",
            "--no-global",
            "--format",
            "json",
        ],
        None,
    );
    assert_eq!(tested.status.code(), Some(0));
    assert_eq!(assert_result(&tested)["data"]["rules"][0]["passed"], true);

    let checked = invoke(
        root.path(),
        global.path(),
        &[
            "check",
            "--rule",
            "local/no-todo-marker",
            "--no-global",
            "--no-cache",
            "--format",
            "json",
        ],
        None,
    );
    assert_eq!(checked.status.code(), Some(1));
    let checked = assert_result(&checked);
    assert_eq!(checked["diagnostics"].as_array().unwrap().len(), 1);
    assert_eq!(checked["diagnostics"][0]["code"], "todo-marker");
}

#[test]
fn plan_has_digest_queries_and_nested_ir_while_not_executing_sources() {
    let root = tempdir().unwrap();
    let global = tempdir().unwrap();
    create_rule(
        root.path(),
        global.path(),
        &text_submission("planned", "advisory"),
    );
    std::fs::write(root.path().join("unreadable.txt"), [0xff, 0xfe]).unwrap();

    let output = invoke(
        root.path(),
        global.path(),
        &[
            "plan",
            "--rule",
            "local/planned",
            "--no-global",
            "--format",
            "json",
        ],
        None,
    );
    assert_eq!(output.status.code(), Some(0));
    let value = assert_result(&output);
    assert_schema("plan", &value);
    let data = &value["data"];
    assert_eq!(data["executed"], false);
    assert!(data["plan_digest"].as_str().unwrap().starts_with("sha256:"));
    assert!(!data["queries"].as_array().unwrap().is_empty());
    assert!(data["rules"][0]["plan"]["ir"]["queries"].is_array());
}

#[test]
fn disabled_invalid_body_is_skipped_by_check_but_selected_validation_fails() {
    let root = tempdir().unwrap();
    let global = tempdir().unwrap();
    std::fs::write(root.path().join("source.txt"), "clean\n").unwrap();
    create_rule(
        root.path(),
        global.path(),
        &text_submission("disabled-body", "advisory"),
    );

    let mode = invoke(
        root.path(),
        global.path(),
        &[
            "set-mode",
            "local/disabled-body",
            "disabled",
            "--reason",
            "temporarily disabled for parser maintenance",
            "--no-global",
            "--format",
            "json",
        ],
        None,
    );
    assert_eq!(mode.status.code(), Some(0));
    assert_eq!(
        assert_result(&mode)["data"]["reason"],
        "temporarily disabled for parser maintenance"
    );
    let manifest: Value = serde_json::from_str(
        &std::fs::read_to_string(root.path().join(".wt/rules/disabled-body/rule.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        manifest["metadata"]["mode_change"]["reason"],
        "temporarily disabled for parser maintenance"
    );

    std::fs::write(
        root.path().join(".wt/rules/disabled-body/check.wt"),
        "for {\n",
    )
    .unwrap();
    let checked = invoke(
        root.path(),
        global.path(),
        &[
            "check",
            "--rule",
            "local/disabled-body",
            "--no-global",
            "--no-cache",
            "--allow-empty",
            "--format",
            "json",
        ],
        None,
    );
    assert_eq!(checked.status.code(), Some(0));
    let checked = assert_result(&checked);
    assert_eq!(checked["status"], "no_work");
    assert!(checked["errors"].as_array().unwrap().is_empty());
    assert_eq!(checked["rules"][0]["status"], "disabled");

    let validation = invoke(
        root.path(),
        global.path(),
        &[
            "validate",
            "local/disabled-body",
            "--no-global",
            "--format",
            "json",
        ],
        None,
    );
    assert_eq!(validation.status.code(), Some(2));
    let validation = assert_result(&validation);
    assert_eq!(validation["status"], "error");
    assert!(validation["errors"][0]["error"]
        .as_str()
        .unwrap()
        .contains("WT100"));
}

#[test]
fn selected_fixture_test_does_not_compile_or_run_an_unrelated_rule() {
    let root = tempdir().unwrap();
    let global = tempdir().unwrap();
    create_rule(
        root.path(),
        global.path(),
        &text_submission("selected", "advisory"),
    );
    create_rule(
        root.path(),
        global.path(),
        &text_submission("unrelated", "advisory"),
    );
    std::fs::write(root.path().join(".wt/rules/unrelated/check.wt"), "for {\n").unwrap();

    let output = invoke(
        root.path(),
        global.path(),
        &["test", "local/selected", "--no-global", "--format", "json"],
        None,
    );
    assert_eq!(output.status.code(), Some(0));
    let value = assert_result(&output);
    let rules = value["data"]["rules"].as_array().unwrap();
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0]["id"], "local/selected");
    assert_eq!(rules[0]["passed"], true);
}

#[test]
fn no_global_ignores_malformed_global_configuration_and_rules() {
    let root = tempdir().unwrap();
    let global = tempdir().unwrap();
    create_rule(
        root.path(),
        global.path(),
        &text_submission("local-only", "advisory"),
    );
    std::fs::write(global.path().join("config.json"), "{\"schema_version\": 2,").unwrap();
    std::fs::create_dir_all(global.path().join("rules/malformed")).unwrap();
    std::fs::write(global.path().join("rules/malformed/rule.json"), "not json").unwrap();

    let output = invoke(
        root.path(),
        global.path(),
        &[
            "validate",
            "local/local-only",
            "--no-global",
            "--format",
            "json",
        ],
        None,
    );
    assert_eq!(output.status.code(), Some(0));
    let value = assert_result(&output);
    assert_eq!(value["data"]["rules"][0]["id"], "local/local-only");
}

#[test]
fn changed_empty_work_is_no_work_but_full_empty_scan_is_incomplete() {
    let root = tempdir().unwrap();
    let global = tempdir().unwrap();
    git(root.path(), &["init", "-q"]);
    git(root.path(), &["config", "user.email", "wt@example.invalid"]);
    git(root.path(), &["config", "user.name", "WT"]);
    create_rule(
        root.path(),
        global.path(),
        &text_submission("empty-scan", "advisory"),
    );
    git(root.path(), &["add", ".wt"]);
    git(root.path(), &["commit", "-qm", "rule"]);

    let changed = invoke(
        root.path(),
        global.path(),
        &[
            "check",
            "--changed",
            "--rule",
            "local/empty-scan",
            "--no-global",
            "--format",
            "json",
        ],
        None,
    );
    assert_eq!(changed.status.code(), Some(0));
    let changed = assert_result(&changed);
    assert_eq!(changed["status"], "no_work");
    assert_eq!(changed["complete"], true);

    let full = invoke(
        root.path(),
        global.path(),
        &[
            "check",
            "--rule",
            "local/empty-scan",
            "--no-global",
            "--no-cache",
            "--format",
            "json",
        ],
        None,
    );
    assert_eq!(full.status.code(), Some(2));
    let full = assert_result(&full);
    assert_eq!(full["status"], "incomplete");
    assert_eq!(full["complete"], false);
    assert!(full["errors"]
        .as_array()
        .unwrap()
        .iter()
        .any(|error| error["error"] == "no_eligible_files"));
}

#[test]
fn waiver_suppression_retains_full_fields_and_unicode_scalar_coordinates() {
    let root = tempdir().unwrap();
    let global = tempdir().unwrap();
    std::fs::write(root.path().join("unicode.txt"), "λ bad\n").unwrap();
    create_rule(
        root.path(),
        global.path(),
        &text_submission("unicode-waiver", "enforced"),
    );
    let waiver = json!({
        "schema_version": 1,
        "waivers": [{
            "id": "known-unicode-case",
            "rule_id": "local/unicode-waiver",
            "code": "hit",
            "path": "unicode.txt",
            "matched_text_digest": wt_core::digest_bytes(b"bad"),
            "reason": "Reviewed Unicode coordinate case"
        }]
    });
    std::fs::write(
        root.path().join(".wt/waivers.json"),
        serde_json::to_vec_pretty(&waiver).unwrap(),
    )
    .unwrap();

    let output = invoke(
        root.path(),
        global.path(),
        &[
            "check",
            "--rule",
            "local/unicode-waiver",
            "--no-global",
            "--no-cache",
            "--format",
            "json",
        ],
        None,
    );
    assert_eq!(output.status.code(), Some(0));
    let value = assert_result(&output);
    assert!(value["diagnostics"].as_array().unwrap().is_empty());
    assert_eq!(value["suppressed"].as_array().unwrap().len(), 1);
    let suppressed = &value["suppressed"][0];
    for field in [
        "rule_id",
        "rule_digest",
        "code",
        "kind",
        "severity",
        "mode",
        "blocking",
        "path",
        "file_digest",
        "start_byte",
        "end_byte",
        "start_line",
        "start_column",
        "end_line",
        "end_column",
        "message",
        "help",
        "waiver_id",
    ] {
        assert!(
            suppressed.get(field).is_some(),
            "missing suppressed field {field}: {suppressed}"
        );
    }
    assert_eq!(suppressed["waiver_id"], "known-unicode-case");
    assert_eq!(suppressed["start_byte"], 3);
    assert_eq!(suppressed["end_byte"], 6);
    assert_eq!(suppressed["start_line"], 1);
    assert_eq!(suppressed["start_column"], 3);
    assert_eq!(suppressed["end_line"], 1);
    assert_eq!(suppressed["end_column"], 6);
    assert_eq!(suppressed["blocking"], false);
    assert_eq!(value["coordinate_encoding"], "unicode-scalar-columns");
    assert!(value["effective_policy"].is_object());
    assert!(value["notices"].is_array());
}

#[test]
fn update_requires_and_accepts_a_test_removal_reason() {
    let root = tempdir().unwrap();
    let global = tempdir().unwrap();
    let original = text_submission("removal-reason", "advisory");
    let created = create_rule(root.path(), global.path(), &original);
    let digest = created["data"]["digest"].as_str().unwrap();
    let mut replacement = original.clone();
    replacement["tests"]["cases"].as_array_mut().unwrap().pop();
    let replacement_text = replacement.to_string();

    let missing_reason = invoke(
        root.path(),
        global.path(),
        &[
            "update",
            "local/removal-reason",
            "--stdin",
            "--expect-hash",
            digest,
            "--allow-test-removal",
            "--format",
            "json",
        ],
        Some(&replacement_text),
    );
    assert_eq!(missing_reason.status.code(), Some(2));
    assert!(assert_result(&missing_reason)["errors"][0]["error"]
        .as_str()
        .unwrap()
        .contains("requires a reason"));

    let accepted = invoke(
        root.path(),
        global.path(),
        &[
            "update",
            "local/removal-reason",
            "--stdin",
            "--expect-hash",
            digest,
            "--allow-test-removal",
            "--reason",
            "Reviewed replacement of the negative fixture",
            "--format",
            "json",
        ],
        Some(&replacement_text),
    );
    assert_eq!(accepted.status.code(), Some(0));
    assert_eq!(assert_result(&accepted)["data"]["validation"], "passed");
}

#[test]
fn decimal_invalid_tsx_fails_when_parser_is_demanded_but_guard_skips_parser() {
    let example_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/template-number-decimal-step.json");
    let invalid_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/template-number-decimal-step/fixtures/invalid-tsx.tsx");
    let mut decimal: Value =
        serde_json::from_str(&std::fs::read_to_string(example_path).unwrap()).unwrap();
    decimal["id"] = json!("decimal-demand");
    decimal["tests"] = Value::Null;
    decimal.as_object_mut().unwrap().remove("tests");

    let demanded_root = tempdir().unwrap();
    let demanded_global = tempdir().unwrap();
    let invalid_source = std::fs::read_to_string(invalid_path).unwrap();
    std::fs::create_dir_all(demanded_root.path().join("frontend/src")).unwrap();
    std::fs::write(
        demanded_root
            .path()
            .join("frontend/src/task-start-page.tsx"),
        &invalid_source,
    )
    .unwrap();
    create_rule(demanded_root.path(), demanded_global.path(), &decimal);
    let demanded = invoke(
        demanded_root.path(),
        demanded_global.path(),
        &[
            "check",
            "--rule",
            "local/decimal-demand",
            "--no-global",
            "--no-cache",
            "--format",
            "json",
        ],
        None,
    );
    assert_eq!(demanded.status.code(), Some(2));
    let demanded = assert_result(&demanded);
    assert_eq!(demanded["status"], "incomplete");
    assert!(demanded["errors"]
        .as_array()
        .unwrap()
        .iter()
        .any(|error| error.to_string().contains("jsx.v1 parse failure")));

    let mut guarded = decimal;
    guarded["id"] = json!("decimal-guard");
    guarded["code"]["source"] = json!(
        "if false { for input in jsx::inputs(file) { emit(input.span, \"decimal-step\"); } }"
    );
    let guarded_root = tempdir().unwrap();
    let guarded_global = tempdir().unwrap();
    std::fs::create_dir_all(guarded_root.path().join("frontend/src")).unwrap();
    std::fs::write(
        guarded_root.path().join("frontend/src/task-start-page.tsx"),
        &invalid_source,
    )
    .unwrap();
    create_rule(guarded_root.path(), guarded_global.path(), &guarded);
    let guarded_output = invoke(
        guarded_root.path(),
        guarded_global.path(),
        &[
            "check",
            "--rule",
            "local/decimal-guard",
            "--no-global",
            "--no-cache",
            "--format",
            "json",
        ],
        None,
    );
    assert_eq!(guarded_output.status.code(), Some(0));
    let guarded_output = assert_result(&guarded_output);
    assert_eq!(guarded_output["status"], "pass");
    assert!(guarded_output["errors"].as_array().unwrap().is_empty());
    assert!(guarded_output["diagnostics"].as_array().unwrap().is_empty());
}

#[test]
#[ignore = "expensive reproducible 10,000-file matrix; run explicitly when measuring work counts"]
fn performance_matrix_work_counts_10k_files_100mib_and_25_100_1000_rules() {
    for rule_count in [25usize, 100, 1000] {
        let root = tempdir().unwrap();
        let global = tempdir().unwrap();
        let data = root.path().join("data");
        std::fs::create_dir_all(&data).unwrap();
        let mut content = "x".repeat(10_486);
        let identity = root.path().to_string_lossy();
        content.replace_range(..identity.len(), &identity);
        for index in 0..10_000 {
            std::fs::write(data.join(format!("file-{index:05}.txt")), &content).unwrap();
        }
        let rules = root.path().join(".wt/rules");
        std::fs::create_dir_all(&rules).unwrap();
        for index in 0..rule_count {
            let id = format!("matrix-{index:04}");
            let directory = rules.join(&id);
            std::fs::create_dir_all(&directory).unwrap();
            let manifest = json!({
                "schema_version": 1,
                "id": id,
                "title": "Matrix rule",
                "documentation": {"file": "rule.md"},
                "mode": "advisory",
                "severity": "info",
                "execution": "file",
                "scope": {"include": ["data/**/*.txt"]},
                "diagnostics": {"hit": {"kind": "violation", "message": "hit", "help": "review"}},
                "code": {"language": "wt-rule-1", "capabilities": ["text.v1"], "file": "check.wt"}
            });
            std::fs::write(
                directory.join("rule.json"),
                serde_json::to_vec(&manifest).unwrap(),
            )
            .unwrap();
            std::fs::write(
                directory.join("rule.md"),
                "# Matrix rule\n\nCounts bounded file work. This ignored test measures work, not a performance promise.\n",
            )
            .unwrap();
            std::fs::write(
                directory.join("check.wt"),
                "if file.text.contains(\"never\") { emit(file.span, \"hit\"); }",
            )
            .unwrap();
        }
        let started = std::time::Instant::now();
        let output = invoke_benchmark(
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
        );
        let elapsed_ms = started.elapsed().as_millis();
        assert_eq!(
            output.status.code(),
            Some(0),
            "rule_count={rule_count}: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        let value = assert_result(&output);
        assert_eq!(value["summary"]["checked_files"], 10_000);
        assert_eq!(value["stats"]["runtime"]["source_reads"], 10_000);
        eprintln!(
            "rule_count={rule_count} cold_no_cache_elapsed_ms={} work={}",
            elapsed_ms, value["stats"]["runtime"]
        );
        if rule_count == 25 {
            let cached = || {
                let started = std::time::Instant::now();
                let output = invoke_benchmark(
                    root.path(),
                    global.path(),
                    &["check", "--no-global", "--stats", "--format", "json"],
                );
                let elapsed_ms = started.elapsed().as_millis();
                assert_eq!(output.status.code(), Some(0));
                (assert_result(&output), elapsed_ms)
            };
            let (initial, seed_ms) = cached();
            assert_eq!(
                initial["stats"]["cache"]["raw_misses"], 250_000,
                "{initial}"
            );
            eprintln!("rule_count=25 cache_seed_elapsed_ms={seed_ms}");
            let (warm, warm_ms) = cached();
            assert_eq!(warm["stats"]["cache"]["raw_hits"], 250_000);
            assert_eq!(warm["stats"]["runtime"]["source_reads"], 10_000);
            eprintln!(
                "rule_count=25 warm_elapsed_ms={} source_reads={}",
                warm_ms, warm["stats"]["runtime"]["source_reads"]
            );

            std::fs::write(data.join("file-00000.txt"), "y".repeat(10_486)).unwrap();
            let (changed, changed_ms) = cached();
            assert_eq!(changed["stats"]["cache"]["raw_hits"], 249_975);
            assert_eq!(changed["stats"]["cache"]["raw_misses"], 25);
            eprintln!(
                "rule_count=25 one_file_changed_elapsed_ms={} cache={}",
                changed_ms, changed["stats"]["cache"]
            );

            let new_rule = rules.join("matrix-0025");
            std::fs::create_dir(&new_rule).unwrap();
            let mut manifest: Value = serde_json::from_slice(
                &std::fs::read(rules.join("matrix-0000/rule.json")).unwrap(),
            )
            .unwrap();
            manifest["id"] = json!("matrix-0025");
            std::fs::write(
                new_rule.join("rule.json"),
                serde_json::to_vec(&manifest).unwrap(),
            )
            .unwrap();
            std::fs::write(
                new_rule.join("check.wt"),
                "if file.text.contains(\"never\") { emit(file.span, \"hit\"); }",
            )
            .unwrap();
            let (added, added_ms) = cached();
            assert_eq!(added["stats"]["cache"]["raw_hits"], 250_000);
            assert_eq!(added["stats"]["cache"]["raw_misses"], 10_000);
            eprintln!(
                "rule_count=26 one_rule_added_elapsed_ms={} cache={}",
                added_ms, added["stats"]["cache"]
            );

            std::fs::write(
                new_rule.join("check.wt"),
                "if file.text.contains(\"never-again\") { emit(file.span, \"hit\"); }",
            )
            .unwrap();
            let (edited, edited_ms) = cached();
            assert_eq!(edited["stats"]["cache"]["raw_hits"], 250_000);
            assert_eq!(edited["stats"]["cache"]["raw_misses"], 10_000);
            eprintln!(
                "rule_count=26 one_rule_edited_elapsed_ms={} cache={}",
                edited_ms, edited["stats"]["cache"]
            );
        }
    }
}
