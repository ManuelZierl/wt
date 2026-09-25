use serde_json::{json, Value};
use std::fs;
use std::process::Command;
use tempfile::{tempdir, TempDir};
use wt_core::{digest_text, dispatch};

struct Repo {
    root: TempDir,
    global: TempDir,
}
impl Repo {
    fn new() -> Self {
        let repo = Self {
            root: tempdir().unwrap(),
            global: tempdir().unwrap(),
        };
        fs::write(repo.root.path().join("source.txt"), "bad\n").unwrap();
        repo
    }
    fn run(&self, command: &str, mut options: Value) -> Value {
        options["root"] = json!(self.root.path());
        options["global_dir"] = json!(self.global.path());
        dispatch(command, &options).unwrap()
    }
    fn install(&self, mode: &str) -> Value {
        let mut submission: Value =
            serde_json::from_str(include_str!("../../../examples/shared-request-foo.json"))
                .unwrap();
        submission["id"] = json!("review-marker");
        submission["mode"] = json!(mode);
        submission["scope"] = json!({"include":["**/*.txt"]});
        submission["diagnostics"] =
            json!({"hit":{"kind":"review","message":"Review marker","help":"Inspect context"}});
        submission["patterns"] = json!({"bad":"bad"});
        submission["code"] = json!({"language":"wt-rule-1","capabilities":["text.v1"],"source":"for m in rx::find_all(file, \"bad\") { emit(m.span, \"hit\"); }"});
        submission["tests"] = json!({"schema_version":2,"cases":[
            {"name":"positive","files":[{"path":"a.txt","content":"bad"}],"expect":[{"path":"a.txt","code":"hit","kind":"review"}]},
            {"name":"negative","files":[{"path":"a.txt","content":"good"}],"expect":[]}
        ]});
        let result = self.run("new", json!({"submission":submission}));
        assert_eq!(result["exit_code"], 0, "{result}");
        result
    }
    fn check(&self) -> Value {
        self.run("check", json!({"no_cache":true}))
    }
    fn accept(&self, finding: &Value) -> Value {
        self.run("review",json!({"finding_id":finding["finding_id"],
            "decision":"acceptable","expect_evidence":finding["evidence_digest"],
            "rationale":"# Reviewed\n\nThis synthetic occurrence is deliberately retained in this exact context."}))
    }
}

#[test]
fn new_formats_and_migrates_prose_to_one_markdown_authority() {
    let repo = Repo::new();
    let created = repo.install("advisory");
    let package = repo.root.path().join(".wt/rules/review-marker");
    let manifest: Value =
        serde_json::from_slice(&fs::read(package.join("rule.json")).unwrap()).unwrap();
    assert_eq!(manifest["schema_version"], 3);
    assert_eq!(manifest["documentation"]["file"], "rule.md");
    for field in ["description", "rationale", "limitations"] {
        assert!(manifest.get(field).is_none());
    }
    assert!(fs::read_to_string(package.join("rule.md"))
        .unwrap()
        .contains("## Limitations"));
    let source = fs::read_to_string(package.join("check.wt")).unwrap();
    assert!(source.contains("\n    emit("));
    let shown = repo.run("show", json!({"id":"local/review-marker"}));
    assert_eq!(shown["digest"], created["digest"]);
    assert!(shown["rule"]["documentation"]["source"].is_string());
    let exported = shown["rule"].clone();
    let second = Repo::new();
    assert_eq!(
        second.run("new", json!({"submission":exported}))["exit_code"],
        0
    );
    assert_eq!(
        second.run("show", json!({"id":"local/review-marker"}))["rule"],
        shown["rule"]
    );
}

#[test]
fn fmt_is_explicit_idempotent_and_check_is_read_only() {
    let repo = Repo::new();
    repo.install("advisory");
    let path = repo.root.path().join(".wt/rules/review-marker/check.wt");
    let source = "for m in rx::find_all(file, \"bad\") { emit(m.span, \"hit\"); }";
    fs::write(&path, source).unwrap();
    assert_eq!(repo.run("fmt", json!({"check":true}))["exit_code"], 1);
    assert_eq!(fs::read_to_string(&path).unwrap(), source);
    assert_eq!(repo.check()["complete"], true);
    assert_eq!(fs::read_to_string(&path).unwrap(), source);
    assert_eq!(repo.run("fmt", json!({}))["exit_code"], 0);
    assert_eq!(repo.run("fmt", json!({"check":true}))["exit_code"], 0);
}

#[test]
fn decisions_hide_no_raw_matches_and_context_changes_reopen() {
    let repo = Repo::new();
    repo.install("enforced");
    let first = repo.check();
    assert_eq!(first["exit_code"], 1);
    let finding = first["diagnostics"][0].clone();
    let accepted = repo.accept(&finding);
    assert_eq!(accepted["exit_code"], 0, "{accepted}");
    let second = repo.check();
    assert_eq!(second["exit_code"], 0, "{second}");
    assert_eq!(second["reviewed"].as_array().unwrap().len(), 1);
    assert_eq!(second["summary"]["raw_findings"], 1);
    assert!(second["diagnostics"].as_array().unwrap().is_empty());
    // Same match and position, different surrounding behavior.
    fs::write(repo.root.path().join("source.txt"), "bad\nnow removable\n").unwrap();
    let changed = repo.check();
    assert_eq!(changed["exit_code"], 1);
    assert_eq!(
        changed["diagnostics"][0]["finding_id"],
        finding["finding_id"]
    );
    assert_eq!(
        changed["diagnostics"][0]["review_state"]["validity"],
        "stale"
    );
    assert_eq!(repo.accept(&finding)["exit_code"], 2);
}

#[test]
fn copied_occurrences_do_not_inherit_an_acceptance() {
    let repo = Repo::new();
    repo.install("enforced");
    let finding = repo.check()["diagnostics"][0].clone();
    assert_eq!(repo.accept(&finding)["exit_code"], 0);
    fs::write(repo.root.path().join("copy.txt"), "bad\n").unwrap();
    let checked = repo.check();
    assert_eq!(checked["exit_code"], 1);
    assert_eq!(checked["reviewed"].as_array().unwrap().len(), 1);
    assert_eq!(checked["diagnostics"].as_array().unwrap().len(), 1);
    assert_ne!(
        checked["diagnostics"][0]["finding_id"],
        finding["finding_id"]
    );
}

#[test]
fn interrupted_hidden_revision_fails_closed_without_erasing_raw_findings() {
    let repo = Repo::new();
    repo.install("enforced");
    let finding = repo.check()["diagnostics"][0].clone();
    assert_eq!(repo.accept(&finding)["exit_code"], 0);
    let occurrence = repo.root.path().join(".wt/reviews").join(
        finding["finding_id"]
            .as_str()
            .unwrap()
            .trim_start_matches("sha256:"),
    );
    fs::create_dir(occurrence.join(".pending-interrupted")).unwrap();
    let checked = repo.check();
    assert_eq!(checked["exit_code"], 2, "{checked}");
    assert_eq!(
        checked["diagnostics"][0]["finding_id"],
        finding["finding_id"]
    );
    assert!(checked["errors"]
        .to_string()
        .contains("invalid_review_state"));
    assert_eq!(repo.run("reviews", json!({}))["exit_code"], 2);
}

#[test]
fn inspect_uses_only_a_digest_verified_local_vcs_object_for_prior_source() {
    let repo = Repo::new();
    let git = |args: &[&str]| {
        let result = Command::new("git")
            .arg("-C")
            .arg(repo.root.path())
            .args(args)
            .status()
            .unwrap();
        assert!(result.success(), "git {args:?}");
    };
    git(&["init", "-q"]);
    git(&["add", "source.txt"]);
    git(&[
        "-c",
        "user.name=WT test",
        "-c",
        "user.email=wt@example.invalid",
        "commit",
        "-qm",
        "Initial source",
    ]);
    repo.install("advisory");
    let finding = repo.check()["diagnostics"][0].clone();
    assert_eq!(repo.accept(&finding)["exit_code"], 0);
    fs::write(
        repo.root.path().join("source.txt"),
        "bad\nchanged context\n",
    )
    .unwrap();
    let inspected = repo.run(
        "inspect",
        json!({"finding_id":finding["finding_id"],"output_version":3}),
    );
    assert_eq!(inspected["exit_code"], 0, "{inspected}");
    assert_eq!(
        inspected["data"]["previous_content"],
        "verified_local_vcs_object"
    );
    assert!(inspected["data"]["source_diff"]
        .as_str()
        .unwrap()
        .contains("+changed context"));
    assert_eq!(
        inspected["data"]["stale_reasons"],
        json!(["owner_file_changed"])
    );
}

#[test]
fn watch_changes_and_rule_contract_changes_require_revalidation() {
    let repo = Repo::new();
    repo.install("enforced");
    let helper = repo.root.path().join("helper.rs");
    fs::write(&helper, "append only").unwrap();
    let finding = repo.check()["diagnostics"][0].clone();
    let recorded = repo.run(
        "review",
        json!({"finding_id":finding["finding_id"],"expect_evidence":finding["evidence_digest"],
        "decision":"acceptable","rationale":"Reviewed owning file and imported helper.",
        "watch":[format!("helper.rs={}",digest_text("append only"))]}),
    );
    assert_eq!(recorded["exit_code"], 0, "{recorded}");
    fs::write(&helper, "supports deletion").unwrap();
    assert_eq!(
        repo.check()["diagnostics"][0]["review_state"]["validity"],
        "stale"
    );
    fs::write(&helper, "append only").unwrap();
    fs::write(
        repo.root.path().join(".wt/rules/review-marker/rule.md"),
        "# Changed contract\n",
    )
    .unwrap();
    assert_eq!(
        repo.check()["diagnostics"][0]["review_state"]["validity"],
        "stale"
    );
}

#[test]
fn decision_history_is_append_only_and_requires_current_hash() {
    let repo = Repo::new();
    repo.install("enforced");
    let finding = repo.check()["diagnostics"][0].clone();
    let accepted = repo.accept(&finding);
    assert_eq!(accepted["exit_code"], 0);
    assert_eq!(repo.accept(&finding)["exit_code"], 2);
    let result=repo.run("review",json!({"finding_id":finding["finding_id"],"expect_evidence":finding["evidence_digest"],
        "expect_hash":accepted["record_digest"],"decision":"confirmed_issue","rationale":"New evidence confirms the concern."}));
    assert_eq!(result["exit_code"], 0, "{result}");
    assert_eq!(result["revision"], 2);
    assert!(std::path::Path::new(accepted["path"].as_str().unwrap())
        .join("rationale.md")
        .is_file());
    assert_eq!(repo.check()["exit_code"], 1);
    assert_eq!(
        repo.run("reviews", json!({}))["reviews"][0]["validity"],
        "not_evaluated"
    );
}

#[test]
fn missing_findings_are_not_reported_as_fixed_and_corrupt_state_fails_closed() {
    let repo = Repo::new();
    repo.install("enforced");
    let finding = repo.check()["diagnostics"][0].clone();
    let accepted = repo.accept(&finding);
    fs::write(repo.root.path().join("source.txt"), "clean\n").unwrap();
    assert_eq!(
        repo.check()["review_records"][0]["validity"],
        "not_observed"
    );
    fs::write(repo.root.path().join("source.txt"), "bad\n").unwrap();
    fs::write(
        std::path::Path::new(accepted["path"].as_str().unwrap()).join("decision.json"),
        "{broken",
    )
    .unwrap();
    let checked = repo.check();
    assert_eq!(checked["exit_code"], 2);
    assert_eq!(checked["complete"], false);
    assert_eq!(checked["diagnostics"].as_array().unwrap().len(), 1);
}

#[test]
fn fixture_tests_ignore_accepted_occurrence_state() {
    let repo = Repo::new();
    repo.install("enforced");
    let finding = repo.check()["diagnostics"][0].clone();
    assert_eq!(repo.accept(&finding)["exit_code"], 0);
    let tested = repo.run("test", json!({"id":"local/review-marker"}));
    assert_eq!(tested["exit_code"], 0);
    assert_eq!(tested["rules"][0]["cases"], 2);
}

#[test]
fn malformed_or_duplicate_prose_is_rejected_without_mutation() {
    let repo = Repo::new();
    repo.install("advisory");
    let mut submission = repo.run("show", json!({"id":"local/review-marker"}))["rule"].clone();
    submission["id"] = json!("duplicate-prose");
    submission["description"] = json!("duplicate");
    assert_eq!(
        repo.run("new", json!({"submission":submission}))["exit_code"],
        2
    );
    assert!(!repo.root.path().join(".wt/rules/duplicate-prose").exists());
}

#[cfg(unix)]
#[test]
fn review_and_documentation_symlinks_are_rejected() {
    use std::os::unix::fs::symlink;
    let repo = Repo::new();
    repo.install("enforced");
    symlink(repo.global.path(), repo.root.path().join(".wt/reviews")).unwrap();
    assert_eq!(repo.check()["exit_code"], 2);
    fs::remove_file(repo.root.path().join(".wt/reviews")).unwrap();
    let doc = repo.root.path().join(".wt/rules/review-marker/rule.md");
    fs::remove_file(&doc).unwrap();
    fs::write(repo.global.path().join("outside.md"), "# External").unwrap();
    symlink(repo.global.path().join("outside.md"), &doc).unwrap();
    assert_eq!(repo.check()["exit_code"], 2);
}

#[test]
fn repository_review_evidence_includes_additions_to_the_authorized_input_set() {
    let repo = Repo::new();
    repo.install("enforced");
    let shown = repo.run("show", json!({"id":"local/review-marker"}));
    let mut rule = shown["rule"].clone();
    rule["execution"] = json!("repository");
    rule["code"]["source"]=json!("for source_file in repo.files() { for m in rx::find_all(source_file, \"bad\") { emit(m.span, \"hit\"); } }");
    let updated = repo.run(
        "update",
        json!({"id":"local/review-marker","expect_hash":shown["digest"],"submission":rule}),
    );
    assert_eq!(updated["exit_code"], 0, "{updated}");
    let finding = repo.check()["diagnostics"][0].clone();
    assert_eq!(repo.accept(&finding)["exit_code"], 0);
    assert_eq!(repo.check()["exit_code"], 0);
    fs::write(
        repo.root.path().join("additional.txt"),
        "clean input, but a different repository context\n",
    )
    .unwrap();
    let checked = repo.check();
    assert_eq!(checked["exit_code"], 1);
    assert_eq!(
        checked["diagnostics"][0]["review_state"]["validity"],
        "stale"
    );
}

#[test]
fn unknown_watched_evidence_and_stale_hash_never_create_acceptances() {
    let repo = Repo::new();
    repo.install("enforced");
    let finding = repo.check()["diagnostics"][0].clone();
    let result=repo.run("review",json!({"finding_id":finding["finding_id"],"expect_evidence":finding["evidence_digest"],
        "decision":"acceptable","rationale":"Requires missing evidence.","watch":[format!("missing.txt={}",digest_text("expected"))]}));
    assert_eq!(result["exit_code"], 2);
    assert_eq!(repo.check()["exit_code"], 1);
    assert!(repo.run("reviews", json!({}))["reviews"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[test]
fn malformed_watch_option_cannot_create_an_unwatched_acceptance() {
    let repo = Repo::new();
    repo.install("enforced");
    fs::write(repo.root.path().join("helper.rs"), "reviewed dependency").unwrap();
    let finding = repo.check()["diagnostics"][0].clone();
    for watch in [
        json!(format!("helper.rs={}", digest_text("reviewed dependency"))),
        Value::Null,
        json!({"helper.rs": digest_text("reviewed dependency")}),
    ] {
        let result = repo.run(
            "review",
            json!({"finding_id":finding["finding_id"],
                "expect_evidence":finding["evidence_digest"],"decision":"acceptable",
                "rationale":"Reviewed the helper as supporting evidence.","watch":watch}),
        );
        assert_eq!(result["exit_code"], 2, "{result}");
        assert!(result["error"].as_str().unwrap().contains("watch"));
    }
    assert!(repo.run("reviews", json!({}))["reviews"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(repo.check()["exit_code"], 1);
}

#[test]
fn modified_earlier_rationale_invalidates_the_current_acceptance() {
    let repo = Repo::new();
    repo.install("enforced");
    let finding = repo.check()["diagnostics"][0].clone();
    let first = repo.accept(&finding);
    let second = repo.run(
        "review",
        json!({"finding_id":finding["finding_id"],
        "expect_evidence":finding["evidence_digest"],"expect_hash":first["record_digest"],
        "decision":"accepted_risk","rationale":"Reviewed again; risk accepted."}),
    );
    assert_eq!(second["exit_code"], 0, "{second}");
    assert_eq!(repo.check()["exit_code"], 0);

    fs::write(
        std::path::Path::new(first["path"].as_str().unwrap()).join("rationale.md"),
        "Changed earlier review rationale.",
    )
    .unwrap();
    let checked = repo.check();
    assert_eq!(checked["exit_code"], 2, "{checked}");
    assert_eq!(checked["complete"], false);
    assert_eq!(checked["diagnostics"].as_array().unwrap().len(), 1);
    assert_eq!(repo.run("reviews", json!({}))["exit_code"], 2);
}

#[test]
fn altered_history_links_fail_closed() {
    for break_first in [false, true] {
        let repo = Repo::new();
        repo.install("enforced");
        let finding = repo.check()["diagnostics"][0].clone();
        let first = repo.accept(&finding);
        let second = repo.run(
            "review",
            json!({"finding_id":finding["finding_id"],
            "expect_evidence":finding["evidence_digest"],"expect_hash":first["record_digest"],
            "decision":"acceptable","rationale":"Reviewed a second time."}),
        );
        assert_eq!(second["exit_code"], 0, "{second}");
        let path = std::path::Path::new(
            if break_first {
                &first["path"]
            } else {
                &second["path"]
            }
            .as_str()
            .unwrap(),
        )
        .join("decision.json");
        let mut record: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        record["previous"] = json!(digest_text("unrelated revision"));
        fs::write(&path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
        let checked = repo.check();
        assert_eq!(checked["exit_code"], 2, "{checked}");
        assert_eq!(checked["diagnostics"].as_array().unwrap().len(), 1);
    }
}
