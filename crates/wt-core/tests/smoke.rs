use std::fs;
use tempfile::tempdir;
use wt_core::dispatch;

#[test]
fn create_and_check_rule() {
    let root = tempdir().unwrap();
    let global = tempdir().unwrap();
    let options =
        serde_json::json!({"root": root.path(), "global_dir": global.path(), "global": false});
    assert_eq!(dispatch("init", &options).unwrap()["exit_code"], 0);
    fs::write(root.path().join("source.txt"), "bad").unwrap();
    let mut submission = serde_json::json!({
        "schema_version": 2, "id": "bad-text", "title": "Bad text", "description": "finds bad text",
        "rationale": "test", "mode": "enforced", "severity": "error", "execution": "file",
        "scope": {"include": ["**/*.txt"]}, "patterns": {"bad": "bad"},
        "diagnostics": {"hit": {"kind": "violation", "message": "bad", "help": "fix"}},
        "limitations": [], "code": {"language": "wt-rule-1", "capabilities": ["regex.v1"], "source": "for m in rx::find_all(file, \"bad\") { emit(m.span, \"hit\"); }"}
    });
    submission["mode"] = serde_json::json!("advisory");
    let create = dispatch("new", &serde_json::json!({"root": root.path(), "global_dir": global.path(), "submission": submission})).unwrap();
    assert_eq!(create["exit_code"], 0, "{create}");
    let result = dispatch(
        "check",
        &serde_json::json!({"root": root.path(), "global_dir": global.path(), "no_global": true}),
    )
    .unwrap();
    assert_eq!(result["exit_code"], 0, "{result}");
    assert_eq!(result["diagnostics"].as_array().unwrap().len(), 1);
}
