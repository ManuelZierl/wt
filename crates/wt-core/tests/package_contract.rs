use std::fs;
use tempfile::tempdir;
use wt_core::{digest_package, dispatch};

fn submission(id: &str, mode: &str, scope: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "schema_version": 2,
        "id": id,
        "title": id,
        "description": "package contract test",
        "rationale": "package contract test",
        "mode": mode,
        "severity": "error",
        "execution": "file",
        "scope": scope,
        "patterns": {"bad": "bad"},
        "diagnostics": {"hit": {"kind": "violation", "message": "bad", "help": "fix"}},
        "limitations": [],
        "code": {
            "language": "wt-rule-1",
            "capabilities": ["regex.v1"],
            "source": "for m in rx::find_all(file, \"bad\") { emit(m.span, \"hit\"); }"
        }
    })
}

fn disk_package(
    root: &std::path::Path,
    id: &str,
    mut manifest: serde_json::Value,
    tests: Option<&str>,
    fixture: Option<(&str, &[u8])>,
) -> std::path::PathBuf {
    let directory = root.join(".wt/rules").join(id);
    fs::create_dir_all(directory.join("fixtures")).unwrap();
    manifest["code"] = serde_json::json!({
        "language": "wt-rule-1",
        "capabilities": ["regex.v1"],
        "file": "check.wt"
    });
    fs::write(
        directory.join("rule.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    fs::write(
        directory.join("check.wt"),
        "for m in rx::find_all(file, \"bad\") { emit(m.span, \"hit\"); }",
    )
    .unwrap();
    if let Some(tests) = tests {
        fs::write(directory.join("tests.json"), tests).unwrap();
    }
    if let Some((path, bytes)) = fixture {
        fs::write(directory.join(path), bytes).unwrap();
    }
    directory
}

fn show(root: &std::path::Path, id: &str) -> serde_json::Value {
    dispatch(
        "show",
        &serde_json::json!({
            "root": root,
            "global_dir": root.join("global"),
            "id": format!("local/{id}")
        }),
    )
    .unwrap()
}

#[test]
fn explicit_tests_file_is_authoritative_and_export_resolves_fixtures() {
    let root = tempdir().unwrap();
    let manifest = serde_json::json!({
        "schema_version": 2,
        "id": "roundtrip",
        "title": "roundtrip",
        "description": "test",
        "rationale": "test",
        "mode": "advisory",
        "severity": "error",
        "execution": "file",
        "scope": {"include": ["**/*.txt"]},
        "patterns": {"bad": "bad"},
        "diagnostics": {"hit": {"kind": "violation", "message": "bad", "help": "fix"}},
        "limitations": [],
        "tests_file": "suite.json"
    });
    let tests = r#"{"schema_version":2,"cases":[{"name":"fixture","files":[{"path":"src/input.txt","fixture":"fixtures/input.txt"}],"expect":[{"path":"src/input.txt","code":"hit","kind":"violation"}]}]}"#;
    let directory = disk_package(
        root.path(),
        "roundtrip",
        manifest,
        None,
        Some(("fixtures/input.txt", b"bad")),
    );
    fs::write(directory.join("suite.json"), tests).unwrap();
    fs::write(directory.join("tests.json"), b"not the authoritative suite").unwrap();

    let exported = show(root.path(), "roundtrip");
    assert_eq!(exported["exit_code"], 0, "{exported}");
    let rule = &exported["rule"];
    assert!(rule.get("tests_file").is_none());
    assert!(rule["code"].get("file").is_none());
    assert_eq!(rule["tests"]["cases"][0]["files"][0]["content"], "bad");
    assert!(rule["tests"]["cases"][0]["files"][0]
        .get("fixture")
        .is_none());

    let validated = dispatch(
        "validate",
        &serde_json::json!({"root": root.path(), "global_dir": root.path().join("global"), "submission": rule}),
    )
    .unwrap();
    assert_eq!(validated["exit_code"], 0, "{validated}");
}

#[test]
fn package_digest_changes_when_a_referenced_fixture_changes() {
    let root = tempdir().unwrap();
    let manifest = serde_json::json!({
        "schema_version": 2, "id": "digest-fixture", "title": "test",
        "description": "test", "rationale": "test", "severity": "error",
        "execution": "file", "scope": {"include": ["**/*.txt"]},
        "patterns": {"bad": "bad"},
        "diagnostics": {"hit": {"kind": "violation", "message": "bad", "help": "fix"}},
        "limitations": [], "tests_file": "tests.json"
    });
    let tests = r#"{"schema_version":2,"cases":[{"name":"case","files":[{"path":"a.txt","fixture":"fixtures/a.txt"}],"expect":[] }]}"#;
    let directory = disk_package(
        root.path(),
        "digest-fixture",
        manifest,
        Some(tests),
        Some(("fixtures/a.txt", b"good")),
    );
    let first = digest_package(
        &directory,
        &["check.wt", "tests.json", "fixtures/a.txt"],
        &fs::read(directory.join("rule.json")).unwrap(),
    )
    .unwrap();
    fs::write(directory.join("fixtures/a.txt"), b"different").unwrap();
    let second = digest_package(
        &directory,
        &["check.wt", "tests.json", "fixtures/a.txt"],
        &fs::read(directory.join("rule.json")).unwrap(),
    )
    .unwrap();
    assert_ne!(first, second);

    let shown = show(root.path(), "digest-fixture");
    assert_eq!(shown["exit_code"], 0, "{shown}");
}

#[cfg(unix)]
#[test]
fn intermediate_symlink_escape_is_rejected_before_fixture_read() {
    use std::os::unix::fs::symlink;

    let root = tempdir().unwrap();
    let outside = tempdir().unwrap();
    fs::write(outside.path().join("input.txt"), b"bad").unwrap();
    let manifest = serde_json::json!({
        "schema_version": 2, "id": "escape", "title": "test", "description": "test",
        "rationale": "test", "severity": "error", "execution": "file",
        "scope": {"include": ["**/*.txt"]}, "patterns": {"bad": "bad"},
        "diagnostics": {"hit": {"kind": "violation", "message": "bad", "help": "fix"}},
        "limitations": [], "tests_file": "tests.json"
    });
    let directory = disk_package(
        root.path(),
        "escape",
        manifest,
        Some(
            r#"{"schema_version":2,"cases":[{"name":"case","files":[{"path":"a.txt","fixture":"fixtures/input.txt"}],"expect":[]}]}"#,
        ),
        None,
    );
    fs::remove_dir(directory.join("fixtures")).unwrap();
    symlink(outside.path(), directory.join("fixtures")).unwrap();

    let result = show(root.path(), "escape");
    assert_eq!(result["exit_code"], 2, "{result}");
}

#[test]
fn oversized_fixture_is_bounded() {
    let root = tempdir().unwrap();
    let manifest = serde_json::json!({
        "schema_version": 2, "id": "large-fixture", "title": "test", "description": "test",
        "rationale": "test", "severity": "error", "execution": "file",
        "scope": {"include": ["**/*.txt"]}, "patterns": {"bad": "bad"},
        "diagnostics": {"hit": {"kind": "violation", "message": "bad", "help": "fix"}},
        "limitations": [], "tests_file": "tests.json"
    });
    let directory = disk_package(
        root.path(),
        "large-fixture",
        manifest,
        Some(
            r#"{"schema_version":2,"cases":[{"name":"case","files":[{"path":"a.txt","fixture":"fixtures/a.txt"}],"expect":[]}]}"#,
        ),
        Some(("fixtures/a.txt", &vec![b'x'; 4 * 1024 * 1024 + 1])),
    );
    let result = show(root.path(), "large-fixture");
    assert_eq!(result["exit_code"], 2, "{result}");
    assert!(directory.join("fixtures/a.txt").exists());
}

#[test]
fn globs_reject_traversal_and_do_not_cross_components() {
    let root = tempdir().unwrap();
    let mut invalid = submission(
        "bad-glob",
        "advisory",
        serde_json::json!({"include": ["src/../*.txt"]}),
    );
    let invalid_result = dispatch(
        "validate",
        &serde_json::json!({"root": root.path(), "global_dir": root.path().join("global"), "submission": invalid.take()}),
    )
    .unwrap();
    assert_eq!(invalid_result["exit_code"], 2, "{invalid_result}");

    let mut valid = submission(
        "component-glob",
        "advisory",
        serde_json::json!({"include": ["*.txt"]}),
    );
    valid["tests"] = serde_json::json!({
        "schema_version": 2,
        "cases": [
            {"name": "root", "files": [{"path": "a.txt", "content": "bad"}], "expect": [{"path": "a.txt", "code": "hit", "kind": "violation"}]},
            {"name": "nested", "files": [{"path": "nested/a.txt", "content": "bad"}], "expect": []}
        ]
    });
    let created = dispatch(
        "new",
        &serde_json::json!({"root": root.path(), "global_dir": root.path().join("global"), "submission": valid}),
    )
    .unwrap();
    assert_eq!(created["exit_code"], 0, "{created}");
    let disk_manifest: serde_json::Value = serde_json::from_slice(
        &fs::read(root.path().join(".wt/rules/component-glob/rule.json")).unwrap(),
    )
    .unwrap();
    assert!(disk_manifest["code"].get("source").is_none());
    assert_eq!(disk_manifest["code"]["file"], "check.wt");
}

#[test]
fn enforced_examples_must_be_applicable_and_nonempty() {
    let root = tempdir().unwrap();
    let mut rule = submission(
        "enforced-noop",
        "enforced",
        serde_json::json!({"include": ["src/**"], "exclude": [{"glob": "src/generated/**", "reason": "generated"}]}),
    );
    rule["tests"] = serde_json::json!({
        "schema_version": 2,
        "cases": [
            {"name": "positive-excluded", "files": [{"path": "src/generated/a.txt", "content": "bad"}], "expect": [{"path": "src/generated/a.txt", "code": "hit", "kind": "violation"}]},
            {"name": "negative-empty", "files": [{"path": "src/generated/b.txt", "content": ""}], "expect": []}
        ]
    });
    let result = dispatch(
        "new",
        &serde_json::json!({"root": root.path(), "global_dir": root.path().join("global"), "submission": rule}),
    )
    .unwrap();
    assert_eq!(result["exit_code"], 2, "{result}");
    assert!(!root.path().join(".wt/rules/enforced-noop").exists());
}

#[test]
fn repository_text_surface_does_not_require_repo_capability() {
    let root = tempdir().unwrap();
    let mut rule = submission(
        "repo-text",
        "advisory",
        serde_json::json!({"include": ["**/*.txt"]}),
    );
    rule["execution"] = "repository".into();
    rule["code"]["capabilities"] = serde_json::json!(["text.v1"]);
    rule["code"]["source"] =
        "for f in repo.files() { if text::contains(f.text, \"bad\") { emit(f.span, \"hit\"); } }"
            .into();
    let result = dispatch(
        "validate",
        &serde_json::json!({"root": root.path(), "global_dir": root.path().join("global"), "submission": rule}),
    )
    .unwrap();
    assert_eq!(result["exit_code"], 0, "{result}");
}

#[test]
fn submissions_reject_package_fixture_references() {
    let root = tempdir().unwrap();
    let mut rule = submission(
        "submission-reference",
        "advisory",
        serde_json::json!({"include": ["**/*.txt"]}),
    );
    rule["tests"] = serde_json::json!({
        "schema_version": 2,
        "cases": [{"name": "reference", "files": [{"path": "a.txt", "fixture": "fixtures/a.txt"}], "expect": []}]
    });
    let result = dispatch(
        "validate",
        &serde_json::json!({"root": root.path(), "global_dir": root.path().join("global"), "submission": rule}),
    )
    .unwrap();
    assert_eq!(result["exit_code"], 2, "{result}");
}
