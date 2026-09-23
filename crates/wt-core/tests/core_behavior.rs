use std::fs;
use std::process::Command;
use tempfile::tempdir;
use wt_core::{digest_bytes, dispatch};

fn submission(id: &str, code: &str) -> serde_json::Value {
    serde_json::json!({
        "schema_version": 2,
        "id": id,
        "title": id,
        "description": "test rule",
        "rationale": "test rationale",
        "mode": "advisory",
        "severity": "warning",
        "execution": "file",
        "scope": {"include": ["**/*.txt"]},
        "patterns": {"bad": "bad"},
        "diagnostics": {"hit": {"kind": "violation", "message": "bad", "help": "fix"}},
        "limitations": [],
        "code": {"language": "wt-rule-1", "capabilities": ["regex.v1"], "source": code}
    })
}

fn new_rule(
    root: &std::path::Path,
    global: &std::path::Path,
    id: &str,
    code: &str,
) -> serde_json::Value {
    new_submission(root, global, submission(id, code))
}

fn new_submission(
    root: &std::path::Path,
    global: &std::path::Path,
    submission: serde_json::Value,
) -> serde_json::Value {
    dispatch(
        "new",
        &serde_json::json!({
            "root": root,
            "global_dir": global,
            "submission": submission
        }),
    )
    .unwrap()
}

fn check(
    root: &std::path::Path,
    global: &std::path::Path,
    extra: serde_json::Value,
) -> serde_json::Value {
    let mut options = serde_json::json!({"root": root, "global_dir": global, "no_global": true});
    for (key, value) in extra.as_object().unwrap() {
        options[key] = value.clone();
    }
    dispatch("check", &options).unwrap()
}

#[test]
fn identical_ids_are_ambiguous_but_qualified_rules_both_run() {
    let root = tempdir().unwrap();
    let global = tempdir().unwrap();
    fs::write(root.path().join("a.txt"), "bad").unwrap();
    let code = "for m in rx::find_all(file, \"bad\") { emit(m.span, \"hit\"); }";
    assert_eq!(
        new_rule(root.path(), global.path(), "same", code)["exit_code"],
        0
    );
    let global_result = dispatch(
        "new",
        &serde_json::json!({
            "root": root.path(),
            "global_dir": global.path(),
            "global": true,
            "submission": submission("same", code)
        }),
    )
    .unwrap();
    assert_eq!(global_result["exit_code"], 0, "{global_result}");

    let ambiguous = dispatch(
        "show",
        &serde_json::json!({"root": root.path(), "global_dir": global.path(), "id": "same"}),
    )
    .unwrap();
    assert_eq!(ambiguous["exit_code"], 2);

    let both = dispatch(
        "check",
        &serde_json::json!({
            "root": root.path(),
            "global_dir": global.path(),
            "rules": ["global/same", "local/same"]
        }),
    )
    .unwrap();
    assert_eq!(both["exit_code"], 0, "{both}");
    assert_eq!(both["diagnostics"].as_array().unwrap().len(), 2);
}

#[test]
fn git_and_non_git_ignore_keep_tracked_files() {
    for use_git in [false, true] {
        let root = tempdir().unwrap();
        let global = tempdir().unwrap();
        fs::write(root.path().join(".gitignore"), "ignored.txt\n").unwrap();
        fs::write(root.path().join("tracked.txt"), "bad").unwrap();
        fs::write(root.path().join("ignored.txt"), "bad").unwrap();
        if use_git {
            git(root.path(), &["init", "-q"]);
            git(root.path(), &["config", "user.email", "wt@example.invalid"]);
            git(root.path(), &["config", "user.name", "WT"]);
            git(root.path(), &["add", ".gitignore", "tracked.txt"]);
            git(root.path(), &["commit", "-qm", "fixture"]);
        }
        let code = "for m in rx::find_all(file, \"bad\") { emit(m.span, \"hit\"); }";
        assert_eq!(
            new_rule(root.path(), global.path(), "ignore", code)["exit_code"],
            0
        );
        let result = check(root.path(), global.path(), serde_json::json!({}));
        assert_eq!(result["exit_code"], 0, "{result}");
        assert_eq!(
            result["diagnostics"].as_array().unwrap().len(),
            1,
            "{result}"
        );
        assert_eq!(result["diagnostics"][0]["path"], "tracked.txt");
        assert!(result["files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|file| { file["path"] == "ignored.txt" && file["status"] == "ignored" }));
    }
}

#[test]
fn scope_excludes_unrelated_gaps_and_waivers_rerender() {
    let root = tempdir().unwrap();
    let global = tempdir().unwrap();
    fs::create_dir(root.path().join("src")).unwrap();
    fs::write(root.path().join("src/a.txt"), "ok").unwrap();
    fs::write(root.path().join("outside.txt"), "too-large").unwrap();
    let mut rule = submission(
        "scoped",
        "for m in rx::find_all(file, \"bad\") { emit(m.span, \"hit\"); }",
    );
    rule["scope"]["include"] = serde_json::json!(["src/**"]);
    assert_eq!(
        new_submission(root.path(), global.path(), rule)["exit_code"],
        0
    );
    let result = check(
        root.path(),
        global.path(),
        serde_json::json!({"max_file_bytes": 2}),
    );
    assert_eq!(result["exit_code"], 0, "{result}");
    assert_eq!(result["summary"]["analysis_gaps"], 0);

    fs::write(root.path().join("src/a.txt"), "bad").unwrap();
    let waiver = serde_json::json!({"schema_version": 2, "waivers": [{
        "id": "w1", "rule_id": "local/scoped", "code": "hit", "path": "src/a.txt",
        "matched_text_digest": digest_bytes(b"bad"), "reason": "known test exception"
    }]});
    fs::write(
        root.path().join(".wt/waivers.json"),
        serde_json::to_vec_pretty(&waiver).unwrap(),
    )
    .unwrap();
    let waived = check(
        root.path(),
        global.path(),
        serde_json::json!({"max_file_bytes": 4}),
    );
    assert_eq!(waived["exit_code"], 0, "{waived}");
    assert!(waived["diagnostics"].as_array().unwrap().is_empty());
    assert_eq!(waived["suppressed"].as_array().unwrap().len(), 1);

    fs::write(root.path().join("src/a.txt"), "ok").unwrap();
    let stale = check(
        root.path(),
        global.path(),
        serde_json::json!({"max_file_bytes": 4}),
    );
    assert!(stale["notices"]
        .as_array()
        .unwrap()
        .iter()
        .any(|notice| notice == "w1"));
}

#[test]
fn update_requires_current_digest_and_preserves_old_package_on_stale_hash() {
    let root = tempdir().unwrap();
    let global = tempdir().unwrap();
    let code = "for m in rx::find_all(file, \"bad\") { emit(m.span, \"hit\"); }";
    let created = new_rule(root.path(), global.path(), "update-me", code);
    let stale = dispatch(
        "update",
        &serde_json::json!({
            "root": root.path(), "global_dir": global.path(), "id": "local/update-me",
            "expect_hash": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            "submission": submission("update-me", code)
        }),
    )
    .unwrap();
    assert_eq!(stale["exit_code"], 2);
    let shown = dispatch(
        "show",
        &serde_json::json!({"root": root.path(), "global_dir": global.path(), "id": "local/update-me"}),
    )
    .unwrap();
    assert_eq!(shown["exit_code"], 0);
    assert_eq!(shown["rule"]["code"]["source"], code);
    assert!(created["digest"].as_str().is_some());
}

#[test]
fn raw_cache_is_per_rule_and_fixture_cache_is_digest_gated() {
    let root = tempdir().unwrap();
    let global = tempdir().unwrap();
    fs::write(root.path().join("a.txt"), "bad").unwrap();
    let code = "for m in rx::find_all(file, \"bad\") { emit(m.span, \"hit\"); }";
    assert_eq!(
        new_rule(root.path(), global.path(), "cache-a", code)["exit_code"],
        0
    );
    let first = check(
        root.path(),
        global.path(),
        serde_json::json!({"stats": true}),
    );
    assert_eq!(first["exit_code"], 0, "{first}");

    assert_eq!(
        new_rule(root.path(), global.path(), "cache-b", code)["exit_code"],
        0
    );
    let second = dispatch(
        "check",
        &serde_json::json!({
            "root": root.path(), "global_dir": global.path(), "no_global": true,
            "rules": ["local/cache-a", "local/cache-b"], "stats": true
        }),
    )
    .unwrap();
    assert_eq!(second["exit_code"], 0, "{second}");
    assert!(second["stats"]["cache"]["raw_hits"].as_u64().unwrap_or(0) >= 1);
    let uncached = check(
        root.path(),
        global.path(),
        serde_json::json!({"rules": ["local/cache-a"], "stats": true, "no_cache": true}),
    );
    assert_eq!(uncached["exit_code"], 0, "{uncached}");
    assert_eq!(uncached["stats"]["cache"]["enabled"], false);

    let mut tested = submission("cache-tests", code);
    let fixture_prefix = root.path().display().to_string();
    tested["tests"] = serde_json::json!({
        "schema_version": 2,
        "cases": [
            {"name": "bad", "files": [{"path": "a.txt", "content": format!("bad-{fixture_prefix}")}], "expect": [{"path": "a.txt", "code": "hit", "kind": "violation"}]},
            {"name": "good", "files": [{"path": "a.txt", "content": format!("ok-{fixture_prefix}")}], "expect": []}
        ]
    });
    let created = new_submission(root.path(), global.path(), tested.clone());
    assert_eq!(created["exit_code"], 0, "{created}");
    let gated = check(
        root.path(),
        global.path(),
        serde_json::json!({"rules": ["local/cache-tests"], "stats": true}),
    );
    assert_eq!(gated["exit_code"], 0, "{gated}");
    assert!(
        gated["stats"]["cache"]["fixture_misses"]
            .as_u64()
            .unwrap_or(0)
            >= 1,
        "{gated}"
    );

    tested["tests"]["cases"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
        "name": "also-good", "files": [{"path": "a.txt", "content": "still-ok"}], "expect": []
        }));
    let updated = dispatch(
        "update",
        &serde_json::json!({
            "root": root.path(), "global_dir": global.path(), "id": "local/cache-tests",
            "expect_hash": created["digest"], "submission": tested
        }),
    )
    .unwrap();
    assert_eq!(updated["exit_code"], 0, "{updated}");
    let after_edit = check(
        root.path(),
        global.path(),
        serde_json::json!({"rules": ["local/cache-tests"], "stats": true}),
    );
    assert_eq!(after_edit["exit_code"], 0, "{after_edit}");
    assert!(
        after_edit["stats"]["cache"]["fixture_misses"]
            .as_u64()
            .unwrap_or(0)
            >= 1
    );
}

#[test]
fn changed_repository_rules_use_full_dependency_snapshots() {
    let root = tempdir().unwrap();
    let global = tempdir().unwrap();
    fs::write(root.path().join("old.txt"), "bad").unwrap();
    fs::write(root.path().join("changed.txt"), "ok").unwrap();
    git(root.path(), &["init", "-q"]);
    git(root.path(), &["config", "user.email", "wt@example.invalid"]);
    git(root.path(), &["config", "user.name", "WT"]);
    git(root.path(), &["add", "old.txt", "changed.txt"]);
    git(root.path(), &["commit", "-qm", "base"]);
    fs::write(root.path().join("changed.txt"), "changed").unwrap();

    let mut rule = submission(
        "repo-rule",
        "for f in repo.files() { if text::contains(f.text, \"bad\") { emit(f.span, \"hit\"); } }",
    );
    rule["execution"] = serde_json::json!("repository");
    rule["patterns"] = serde_json::json!({});
    rule["code"]["capabilities"] = serde_json::json!(["text.v1", "repo.v1"]);
    assert_eq!(
        new_submission(root.path(), global.path(), rule)["exit_code"],
        0
    );

    let result = check(
        root.path(),
        global.path(),
        serde_json::json!({"changed": true, "base": "HEAD"}),
    );
    assert_eq!(result["exit_code"], 0, "{result}");
    assert_eq!(result["scope"]["partial"], true);
    assert_eq!(result["diagnostics"][0]["path"], "old.txt");
}

fn git(root: &std::path::Path, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(root)
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "git {:?} failed", args);
}
