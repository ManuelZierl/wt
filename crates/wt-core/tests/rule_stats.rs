use serde_json::{json, Value};
use std::fs;
use tempfile::TempDir;
use wt_core::dispatch;

struct Repo {
    root: TempDir,
    global: TempDir,
}
impl Repo {
    fn new() -> Self {
        Self {
            root: TempDir::new().unwrap(),
            global: TempDir::new().unwrap(),
        }
    }
    fn run(&self, command: &str, mut options: Value) -> Value {
        options["root"] = json!(self.root.path());
        options["global_dir"] = json!(self.global.path());
        dispatch(command, &options).unwrap()
    }
    /// Install an enforced rule whose `pattern` regex decides which
    /// occurrences it reports over `*.txt` files, using a `review`
    /// diagnostic so acceptances are legal.
    fn install(&self, id: &str, pattern: &str) -> Value {
        let submission = json!({
            "schema_version": 1,
            "id": id,
            "title": id,
            "documentation": {"source": format!("# {id}\n\n## Description\n\nMatches {pattern}.\n\n## Rationale\n\nTest fixture.\n\n## Limitations\n\nNone.\n")},
            "mode": "enforced",
            "severity": "warning",
            "execution": "file",
            "scope": {"include": ["**/*.txt"]},
            "patterns": {"needle": pattern},
            "diagnostics": {"hit": {"kind": "review", "message": "Review marker", "help": "Inspect context"}},
            "code": {"language": "wt-rule-1", "capabilities": ["text.v1"],
                "source": "for m in rx::find_all(file, \"needle\") { emit(m.span, \"hit\"); }"},
            "tests": {"schema_version": 1, "cases": [
                {"name": "positive", "files": [{"path": "fixture.txt", "content": pattern}], "expect": [{"path": "fixture.txt", "code": "hit", "kind": "review"}]},
                {"name": "negative", "files": [{"path": "fixture.txt", "content": "unrelated"}], "expect": []}
            ]}
        });
        let result = self.run("new", json!({"submission": submission}));
        assert_eq!(result["exit_code"], 0, "{result}");
        result
    }
    fn check(&self) -> Value {
        self.run("check", json!({"no_cache": true, "detail": "full"}))
    }
    fn stats(&self) -> Value {
        self.run("stats", json!({"no_cache": true}))
    }
    fn decide(&self, finding: &Value, decision: &str) -> Value {
        let result = self.run(
            "review",
            json!({"finding_id": finding["finding_id"], "decision": decision,
                "expect_evidence": finding["evidence_digest"],
                "rationale": "# Reviewed\n\nRecorded for the wt stats test fixtures."}),
        );
        assert_eq!(result["exit_code"], 0, "{result}");
        result
    }
    fn rule<'a>(stats: &'a Value, id: &str) -> &'a Value {
        stats["data"]["rules"]
            .as_array()
            .unwrap()
            .iter()
            .find(|rule| rule["id"] == id)
            .unwrap_or_else(|| panic!("rule {id} missing from stats output: {stats}"))
    }
}

#[test]
fn a_rule_with_no_findings_and_no_decisions_is_dead() {
    let repo = Repo::new();
    repo.install("dead-rule", "zzz-never-matches-zzz");
    fs::write(repo.root.path().join("source.txt"), "clean text\n").unwrap();
    let stats = repo.stats();
    assert_eq!(stats["exit_code"], 0, "{stats}");
    assert_eq!(stats["data"]["complete"], true);
    let rule = Repo::rule(&stats, "local/dead-rule");
    assert_eq!(rule["signal"], "dead");
    assert_eq!(rule["mode"], "enforced");
    assert_eq!(rule["severity"], "warning");
    assert_eq!(rule["raw_findings"], 0);
    assert_eq!(rule["stale_decisions"], 0);
    assert_eq!(rule["review_decisions"]["acceptable"], 0);
    assert_eq!(rule["review_decisions"]["confirmed_issue"], 0);
    assert_eq!(rule["review_decisions"]["accepted_risk"], 0);
    assert_eq!(stats["data"]["signals"]["dead"], 1);
}

#[test]
fn most_findings_accepted_as_acceptable_is_noisy() {
    let repo = Repo::new();
    repo.install("noisy-rule", "bad");
    fs::write(repo.root.path().join("source.txt"), "bad\n").unwrap();
    let finding = repo.check()["diagnostics"][0].clone();
    repo.decide(&finding, "acceptable");
    let stats = repo.stats();
    let rule = Repo::rule(&stats, "local/noisy-rule");
    assert_eq!(rule["signal"], "noisy");
    assert_eq!(rule["raw_findings"], 1);
    assert_eq!(rule["review_decisions"]["acceptable"], 1);
    assert_eq!(stats["data"]["signals"]["noisy"], 1);
}

#[test]
fn a_confirmed_issue_makes_a_rule_useful_even_if_mostly_accepted_elsewhere() {
    let repo = Repo::new();
    repo.install("useful-rule", "bad");
    fs::write(repo.root.path().join("a.txt"), "bad\n").unwrap();
    fs::write(repo.root.path().join("b.txt"), "bad also\n").unwrap();
    fs::write(repo.root.path().join("c.txt"), "bad too\n").unwrap();
    let checked = repo.check();
    let diagnostics = checked["diagnostics"].as_array().unwrap().clone();
    assert_eq!(diagnostics.len(), 3, "{checked}");
    // Two occurrences are accepted as fine, but one is a real, confirmed issue.
    repo.decide(&diagnostics[0], "acceptable");
    repo.decide(&diagnostics[1], "acceptable");
    repo.decide(&diagnostics[2], "confirmed_issue");
    let stats = repo.stats();
    let rule = Repo::rule(&stats, "local/useful-rule");
    assert_eq!(rule["signal"], "useful");
    assert_eq!(rule["review_decisions"]["acceptable"], 2);
    assert_eq!(rule["review_decisions"]["confirmed_issue"], 1);
    assert_eq!(stats["data"]["signals"]["useful"], 1);
    assert_eq!(stats["data"]["signals"]["noisy"], 0);
}

#[test]
fn stale_decisions_are_counted_separately_from_their_outcome() {
    let repo = Repo::new();
    repo.install("stale-rule", "bad");
    fs::write(repo.root.path().join("source.txt"), "bad\n").unwrap();
    let finding = repo.check()["diagnostics"][0].clone();
    repo.decide(&finding, "acceptable");
    assert_eq!(
        Repo::rule(&repo.stats(), "local/stale-rule")["stale_decisions"],
        0
    );
    // Same match, changed surrounding context: the acceptance goes stale.
    fs::write(repo.root.path().join("source.txt"), "bad\nnew context\n").unwrap();
    let stats = repo.stats();
    let rule = Repo::rule(&stats, "local/stale-rule");
    assert_eq!(rule["stale_decisions"], 1);
    assert_eq!(rule["review_decisions"]["acceptable"], 1);
    // A stale acceptance no longer hides the finding, so it is still noisy,
    // not dead: the check that ran as part of stats reports it as raw again.
    assert_eq!(rule["raw_findings"], 1);
    assert_eq!(rule["signal"], "noisy");
}

#[test]
fn an_incomplete_check_is_reported_as_unknown_not_dead() {
    let repo = Repo::new();
    repo.install("incomplete-rule", "bad");
    fs::write(repo.root.path().join("source.txt"), "bad\n").unwrap();
    let finding = repo.check()["diagnostics"][0].clone();
    let accepted = repo.decide(&finding, "acceptable");
    // Corrupt the stored decision so the review store fails closed: the
    // underlying check becomes incomplete without any rule actually going quiet.
    fs::write(
        std::path::Path::new(accepted["data"]["path"].as_str().unwrap()).join("decision.json"),
        "{broken",
    )
    .unwrap();
    let stats = repo.stats();
    assert_eq!(stats["exit_code"], 2, "{stats}");
    assert_eq!(stats["data"]["complete"], false);
    let rule = Repo::rule(&stats, "local/incomplete-rule");
    assert_eq!(rule["signal"], "unknown");
    assert_ne!(rule["signal"], "dead");
    assert_eq!(rule["raw_findings"], Value::Null);
    assert_eq!(rule["stale_decisions"], Value::Null);
    assert!(!stats["data"]["errors"].as_array().unwrap().is_empty());
}

#[test]
fn a_disabled_rule_is_reported_as_disabled_not_dead() {
    let repo = Repo::new();
    repo.install("disabled-rule", "bad");
    let set = repo.run(
        "set-mode",
        json!({"id": "local/disabled-rule", "mode": "disabled", "reason": "Temporarily retired."}),
    );
    assert_eq!(set["exit_code"], 0, "{set}");
    let stats = repo.stats();
    let rule = Repo::rule(&stats, "local/disabled-rule");
    assert_eq!(rule["signal"], "disabled");
    assert_eq!(rule["mode"], "disabled");
    assert_eq!(stats["data"]["signals"]["disabled"], 1);
}

#[test]
fn problem_signals_are_sorted_first_in_stats_output() {
    let repo = Repo::new();
    repo.install("a-useful-rule", "bad");
    repo.install("b-dead-rule", "zzz-never-matches-zzz");
    fs::write(repo.root.path().join("source.txt"), "bad\n").unwrap();
    let finding = repo.check()["diagnostics"][0].clone();
    repo.decide(&finding, "confirmed_issue");
    let stats = repo.stats();
    let ids = stats["data"]["rules"]
        .as_array()
        .unwrap()
        .iter()
        .map(|rule| rule["id"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(ids, vec!["local/b-dead-rule", "local/a-useful-rule"]);
}
