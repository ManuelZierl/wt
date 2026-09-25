use jsonschema::validator_for;
use serde_json::{json, Value};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use tempfile::tempdir;

fn run(root: &Path, global: &Path, args: &[&str], input: Option<&Value>) -> Value {
    let mut command = Command::new(env!("CARGO_BIN_EXE_wt"));
    command
        .args(args)
        .args(if args.contains(&"check") {
            &["--detail", "full"][..]
        } else {
            &[]
        })
        .args(["--format", "json", "--root"])
        .arg(root)
        .arg("--global-dir")
        .arg(global)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().unwrap();
    if let Some(input) = input {
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.to_string().as_bytes())
            .unwrap();
    }
    drop(child.stdin.take());
    let result = child.wait_with_output().unwrap();
    let parsed: Value =
        serde_json::from_slice(&result.stdout).unwrap_or_else(|e| panic!("{e}: {:?}", result));
    let schema: Value =
        serde_json::from_str(include_str!("../../../schemas/result.schema.json")).unwrap();
    let validator = validator_for(&schema).unwrap();
    let errors = validator
        .iter_errors(&parsed)
        .map(|e| e.to_string())
        .collect::<Vec<_>>();
    assert!(errors.is_empty(), "{errors:?}: {parsed}");
    assert_eq!(
        result.status.code().unwrap(),
        parsed["exit_code"].as_i64().unwrap() as i32
    );
    parsed
}

#[test]
fn cli_formats_markdown_packages_and_records_review_without_weakening_rule() {
    let root = tempdir().unwrap();
    let global = tempdir().unwrap();
    let submission = json!({"schema_version":1,"id":"review-marker","title":"Review marker", "severity":"warning","mode":"enforced",
    "documentation":{"source":"# Review marker\n\nThis is a review trigger, not a proven bug.\n"},
    "scope":{"include":["**/*.txt"]},"diagnostics":{"hit":{"kind":"review","message":"Review this marker","help":"Inspect its context"}},
    "code":{"language":"wt-rule-1","capabilities":["text.v1"],"source":"if file.text.contains(\"bad\") { emit(file.span, \"hit\"); }"},
    "tests":{"schema_version":1,"cases":[
        {"name":"positive","files":[{"path":"a.txt","content":"bad"}],"expect":[{"path":"a.txt","code":"hit","kind":"review"}]},
        {"name":"negative","files":[{"path":"a.txt","content":"good"}],"expect":[]}
    ]}});
    assert_eq!(
        run(
            root.path(),
            global.path(),
            &["new", "--stdin"],
            Some(&submission)
        )["exit_code"],
        0
    );
    fs_write(root.path(), "source.txt", "bad\n");
    fs_write(
        root.path(),
        "rationale.md",
        "# Accepted\n\nThis exact synthetic file is intentionally kept.",
    );
    let checked = run(root.path(), global.path(), &["check", "--no-cache"], None);
    assert_eq!(checked["exit_code"], 1);
    let finding = &checked["diagnostics"][0];
    let id = finding["finding_id"].as_str().unwrap();
    let evidence = finding["evidence_digest"].as_str().unwrap();
    let reason = root.path().join("rationale.md");
    let accepted = run(
        root.path(),
        global.path(),
        &[
            "review",
            id,
            "--decision",
            "acceptable",
            "--expect-evidence",
            evidence,
            "--reason-file",
            reason.to_str().unwrap(),
        ],
        None,
    );
    assert_eq!(accepted["exit_code"], 0, "{accepted}");
    let record: Value = serde_json::from_slice(
        &std::fs::read(Path::new(accepted["data"]["path"].as_str().unwrap()).join("decision.json"))
            .unwrap(),
    )
    .unwrap();
    let schema: Value =
        serde_json::from_str(include_str!("../../../schemas/review.schema.json")).unwrap();
    assert!(validator_for(&schema).unwrap().is_valid(&record));
    let checked = run(
        root.path(),
        global.path(),
        &["check", "--strict", "--no-cache"],
        None,
    );
    assert_eq!(checked["exit_code"], 0);
    assert_eq!(checked["summary"]["raw_findings"], 1);
    assert_eq!(checked["summary"]["reviewed_findings"], 1);
    let shown = run(
        root.path(),
        global.path(),
        &["show", "local/review-marker"],
        None,
    );
    assert!(shown["data"]["rule"]["documentation"]["source"].is_string());
    let formatted = root.path().join(".wt/rules/review-marker/check.wt");
    std::fs::write(
        &formatted,
        "if file.text.contains(\"bad\") { emit(file.span, \"hit\"); }",
    )
    .unwrap();
    assert_eq!(
        run(root.path(), global.path(), &["fmt", "--check"], None)["exit_code"],
        1
    );
    assert_eq!(
        run(root.path(), global.path(), &["fmt"], None)["exit_code"],
        0
    );
    assert_eq!(
        run(root.path(), global.path(), &["fmt", "--check"], None)["exit_code"],
        0
    );
    fs_write(root.path(), "source.txt", "bad\ncontext changed\n");
    assert_eq!(
        run(root.path(), global.path(), &["check"], None)["exit_code"],
        1
    );
    assert_eq!(
        run(root.path(), global.path(), &["reviews"], None)["data"]["reviews"][0]["validity"],
        "not_evaluated"
    );
}

fn fs_write(root: &Path, path: &str, content: &str) {
    std::fs::write(root.join(path), content).unwrap();
}
