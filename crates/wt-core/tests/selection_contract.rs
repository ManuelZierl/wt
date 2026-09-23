use std::fs;
use std::process::Command;
use tempfile::tempdir;
use wt_core::dispatch;

fn config(root: &std::path::Path, global: &std::path::Path, no_global: bool) -> serde_json::Value {
    dispatch(
        "config",
        &serde_json::json!({
            "root": root,
            "global_dir": global,
            "no_global": no_global
        }),
    )
    .unwrap()
}

fn explain(
    root: &std::path::Path,
    global: &std::path::Path,
    path: &str,
    extra: serde_json::Value,
) -> serde_json::Value {
    let mut options = serde_json::json!({
        "root": root,
        "global_dir": global,
        "paths": [root.join(path)]
    });
    for (key, value) in extra.as_object().unwrap() {
        options[key] = value.clone();
    }
    dispatch("explain", &options).unwrap()
}

fn check(
    root: &std::path::Path,
    global: &std::path::Path,
    extra: serde_json::Value,
) -> serde_json::Value {
    let mut options = serde_json::json!({
        "root": root,
        "global_dir": global,
        "allow_empty": true
    });
    for (key, value) in extra.as_object().unwrap() {
        options[key] = value.clone();
    }
    dispatch("check", &options).unwrap()
}

fn git(root: &std::path::Path, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(root)
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "git {:?} failed", args);
}

#[test]
fn local_scalar_fields_do_not_reset_global_values() {
    let root = tempdir().unwrap();
    let global = tempdir().unwrap();
    fs::create_dir(root.path().join(".wt")).unwrap();
    fs::write(
        global.path().join("config.json"),
        r#"{"schema_version":2,"scan":{"max_file_bytes":17}}"#,
    )
    .unwrap();
    fs::write(
        root.path().join(".wt/config.json"),
        r#"{"schema_version":2,"scan":{"exclude":[]}}"#,
    )
    .unwrap();

    let result = config(root.path(), global.path(), false);
    assert_eq!(result["exit_code"], 0, "{result}");
    assert_eq!(result["scan"]["max_file_bytes"], 17);
}

#[test]
fn configured_exclusions_are_component_globs_and_report_origin() {
    let root = tempdir().unwrap();
    let global = tempdir().unwrap();
    fs::create_dir(root.path().join(".wt")).unwrap();
    fs::create_dir_all(root.path().join("generated/deep")).unwrap();
    fs::write(root.path().join("generated/one.txt"), "x").unwrap();
    fs::write(root.path().join("generated/deep/two.txt"), "x").unwrap();
    fs::write(
        root.path().join(".wt/config.json"),
        r#"{"schema_version":2,"scan":{"exclude":[{"glob":"generated/*","reason":"generated output"}]}}"#,
    )
    .unwrap();

    let excluded = explain(
        root.path(),
        global.path(),
        "generated/one.txt",
        serde_json::json!({}),
    );
    assert_eq!(excluded["selection"][0]["status"], "excluded", "{excluded}");
    assert_eq!(excluded["selection"][0]["reason"], "generated output");
    assert_eq!(excluded["selection"][0]["origin"], "config.scan.exclude");

    let nested = explain(
        root.path(),
        global.path(),
        "generated/deep/two.txt",
        serde_json::json!({}),
    );
    assert_eq!(nested["selection"][0]["status"], "checked", "{nested}");
}

#[test]
fn tracked_ignored_files_are_kept_and_nested_repositories_are_boundaries() {
    let root = tempdir().unwrap();
    let global = tempdir().unwrap();
    fs::write(
        root.path().join(".gitignore"),
        "ignored.txt\nuntracked.txt\n",
    )
    .unwrap();
    fs::write(root.path().join("ignored.txt"), "tracked").unwrap();
    fs::write(root.path().join("untracked.txt"), "ignored").unwrap();
    fs::create_dir_all(root.path().join("nested/.git")).unwrap();
    fs::write(root.path().join("nested/source.txt"), "must not scan").unwrap();
    git(root.path(), &["init", "-q"]);
    git(root.path(), &["config", "user.email", "wt@example.invalid"]);
    git(root.path(), &["config", "user.name", "WT"]);
    git(root.path(), &["add", ".gitignore"]);
    git(root.path(), &["add", "-f", "ignored.txt"]);
    git(root.path(), &["commit", "-qm", "fixture"]);

    let ignored = explain(
        root.path(),
        global.path(),
        "untracked.txt",
        serde_json::json!({}),
    );
    assert_eq!(ignored["selection"][0]["status"], "ignored", "{ignored}");
    let tracked = explain(
        root.path(),
        global.path(),
        "ignored.txt",
        serde_json::json!({}),
    );
    assert_eq!(tracked["selection"][0]["status"], "checked", "{tracked}");
    let boundary = explain(
        root.path(),
        global.path(),
        "nested/source.txt",
        serde_json::json!({}),
    );
    assert_eq!(
        boundary["selection"][0]["reason"], "nested_repository",
        "{boundary}"
    );
}

#[test]
fn oversized_initial_nul_is_binary_without_an_unbounded_read() {
    let root = tempdir().unwrap();
    let global = tempdir().unwrap();
    fs::write(root.path().join("blob.bin"), [0, b'x', b'x', b'x', b'x']).unwrap();
    let submission = serde_json::json!({"schema_version":2,"id":"binary-domain","title":"Text domain","description":"Exercise binary selection","rationale":"Binary data is not text","severity":"warning","scope":{"include":["**"]},"diagnostics":{"unexpected":{"kind":"violation","message":"unexpected","help":"inspect"}},"limitations":[],"code":{"language":"wt-rule-1","capabilities":["text.v1"],"source":"return;"}});
    let created = dispatch(
        "new",
        &serde_json::json!({"root":root.path(),"global_dir":global.path(),"submission":submission}),
    )
    .unwrap();
    assert_eq!(created["exit_code"], 0, "{created}");
    let result = check(
        root.path(),
        global.path(),
        serde_json::json!({"max_file_bytes": 4}),
    );
    assert_eq!(result["files"][0]["status"], "binary", "{result}");
    assert!(result["gaps"].as_array().unwrap().is_empty(), "{result}");
}

#[test]
fn no_global_reports_inactive_global_overrides() {
    let root = tempdir().unwrap();
    let global = tempdir().unwrap();
    fs::create_dir(root.path().join(".wt")).unwrap();
    fs::write(
        global.path().join("config.json"),
        "malformed and intentionally not loaded",
    )
    .unwrap();
    fs::write(
        root.path().join(".wt/config.json"),
        r#"{"schema_version":2,"rules":{"mode_overrides":{"global/missing":{"mode":"advisory","reason":"fixture"}}}}"#,
    )
    .unwrap();
    let result = config(root.path(), global.path(), true);
    assert_eq!(result["exit_code"], 0, "{result}");
    assert_eq!(result["inactive_overrides"][0], "global/missing");
}
