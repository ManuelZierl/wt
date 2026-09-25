use serde_json::{json, Value};
use std::fs;
use tempfile::tempdir;
use wt_core::dispatch;

fn submission() -> Value {
    json!({"schema_version":1,"id":"marker","title":"Marker","mode":"advisory",
        "severity":"warning","execution":"file","scope":{"include":["**/*.txt"]},
        "patterns":{"marker":"bad"},"diagnostics":{"hit":{"kind":"review","message":"Review marker","help":"Inspect"}},
        "documentation":{"source":"# Intended constraint\nReview markers."},
        "code":{"language":"wt-rule-1","capabilities":["text.v1"],"source":"for m in rx::find_all(file, \"marker\") { emit(m.span, \"hit\"); }"}})
}

#[test]
fn explicit_coverage_is_enforced_independently_of_findings_and_allow_empty() {
    let root = tempdir().unwrap();
    let global = tempdir().unwrap();
    let common =
        json!({"root":root.path(),"global_dir":global.path(),"no_global":true,"detail":"full"});
    assert_eq!(
        dispatch(
            "new",
            &json!({"root":root.path(),"global_dir":global.path(),"submission":submission()})
        )
        .unwrap()["exit_code"],
        0
    );
    fs::write(root.path().join(".wt/config.json"), json!({"schema_version":1,"coverage":{"expectations":[{"rule_id":"local/marker","minimum_files":1,"reason":"Retain notes coverage"}]}}).to_string()).unwrap();
    let empty = dispatch(
        "check",
        &json!({"root":root.path(),"global_dir":global.path(),"no_global":true,"allow_empty":true,"detail":"full"}),
    )
    .unwrap();
    assert_eq!(empty["exit_code"], 2, "{empty}");
    assert_eq!(
        empty["coverage_expectations"][0]["status"],
        "not_applicable"
    );

    fs::write(root.path().join("note.txt"), "clean").unwrap();
    let full = dispatch("check", &common).unwrap();
    assert_eq!(full["exit_code"], 0, "{full}");
    assert_eq!(full["coverage_expectations"][0]["status"], "satisfied");
    assert_eq!(full["summary"]["raw_findings"], 0);

    fs::write(root.path().join("note.txt"), "ok").unwrap();
    fs::write(root.path().join("oversized.txt"), "long file").unwrap();
    fs::write(root.path().join(".wt/config.json"), json!({"schema_version":1,
        "scan":{"max_file_bytes":3},
        "coverage":{"expectations":[{"rule_id":"local/marker","minimum_files":1,"reason":"Retain notes coverage"}]}}).to_string()).unwrap();
    let gap = dispatch("check", &common).unwrap();
    assert_eq!(gap["exit_code"], 2, "{gap}");
    assert_eq!(gap["coverage_expectations"][0]["completed_files"], 1);
    assert_eq!(gap["coverage_expectations"][0]["source_gaps"], 1);
    assert_eq!(gap["coverage_expectations"][0]["status"], "incomplete");
    fs::remove_file(root.path().join("oversized.txt")).unwrap();

    let partial = dispatch("check", &json!({"root":root.path(),"global_dir":global.path(),"no_global":true,"rules":["local/marker"],"detail":"full"})).unwrap();
    assert_eq!(
        partial["coverage_expectations"][0]["status"],
        "not_evaluated_partial"
    );
    assert_eq!(partial["scope"]["partial"], true);

    fs::write(root.path().join(".wt/config.json"), json!({"schema_version":1,"coverage":{"expectations":[{"rule_id":"global/missing","minimum_files":1,"reason":"Global policy"}]}}).to_string()).unwrap();
    let omitted = dispatch("check", &common).unwrap();
    assert_eq!(omitted["exit_code"], 2, "{omitted}");
    assert_eq!(
        omitted["coverage_expectations"][0]["status"],
        "missing_rule"
    );
}

#[test]
fn non_current_config_schema_version_is_rejected() {
    let root = tempdir().unwrap();
    let global = tempdir().unwrap();
    fs::create_dir(root.path().join(".wt")).unwrap();
    fs::write(root.path().join(".wt/config.json"), r#"{"schema_version":2,"coverage":{"expectations":[{"rule_id":"local/marker","minimum_files":1,"reason":"required"}]}}"#).unwrap();
    let result = dispatch(
        "config",
        &json!({"root":root.path(),"global_dir":global.path()}),
    )
    .unwrap();
    assert_eq!(result["exit_code"], 2, "{result}");
}

#[test]
fn runtime_profile_merges_supplied_fields_and_rejects_unsupported_or_unsafe_values() {
    let root = tempdir().unwrap();
    let global = tempdir().unwrap();
    fs::create_dir(root.path().join(".wt")).unwrap();
    let local_path = root.path().join(".wt/config.json");
    fs::write(
        global.path().join("config.json"),
        json!({"schema_version":1,
        "runtime":{"file_steps":300,"file_native_bytes":100}, "optimizer":{"mode":"off"}})
        .to_string(),
    )
    .unwrap();
    fs::write(
        &local_path,
        json!({"schema_version":1,
        "runtime":{"file_steps":100}})
        .to_string(),
    )
    .unwrap();
    let options = json!({"root":root.path(),"global_dir":global.path()});
    let config = dispatch("config", &options).unwrap();
    assert_eq!(
        config["data"]["effective"]["runtime"]["file_steps"], 100,
        "{config}"
    );
    assert_eq!(
        config["data"]["effective"]["runtime"]["file_native_bytes"],
        100
    );
    assert_eq!(config["data"]["effective"]["optimizer"]["mode"], "off");
    assert!(config["data"]["origins"]
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["key"] == "runtime.file_steps" && entry["origin"] == "local"));
    assert!(config["data"]["origins"]
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["key"] == "runtime.file_native_bytes" && entry["origin"] == "global"));

    for invalid in [
        json!({"schema_version":2}),
        json!({"schema_version":1,"runtime":null}),
        json!({"schema_version":1,"runtime":{"file_steps":0}}),
        json!({"schema_version":1,"runtime":{"file_steps":1_000_001}}),
        json!({"schema_version":1,"runtime":{"worker_memory_bytes":1}}),
        json!({"schema_version":1,"runtime":{"worker_memory_bytes":536870913}}),
        json!({"schema_version":1,"runtime":{"total_memory_bytes":536870912,"parent_memory_bytes":536870912}}),
        json!({"schema_version":1,"runtime":{"unknown_budget":1}}),
        json!({"schema_version":1,"optimizer":{"mode":"experimental"}}),
    ] {
        fs::write(&local_path, invalid.to_string()).unwrap();
        let result = dispatch("config", &options).unwrap();
        assert_eq!(result["exit_code"], 2, "{invalid}: {result}");
    }
}

#[test]
fn preview_does_not_replace_package_and_inspect_reports_source_change() {
    let root = tempdir().unwrap();
    let global = tempdir().unwrap();
    let common = json!({"root":root.path(),"global_dir":global.path()});
    let created = dispatch(
        "new",
        &json!({"root":root.path(),"global_dir":global.path(),"submission":submission()}),
    )
    .unwrap();
    assert_eq!(created["exit_code"], 0, "{created}");
    assert!(!root.path().join(".wt/rules/.marker.lock").exists());
    let old_digest = created["data"]["digest"].as_str().unwrap();
    let mut replacement = submission();
    replacement["documentation"]["source"] = json!("# Intended constraint\nNew explanation.");
    let preview = dispatch("update", &json!({"root":root.path(),"global_dir":global.path(),"id":"local/marker","expect_hash":old_digest,"submission":replacement,"preview":true})).unwrap();
    assert_eq!(preview["exit_code"], 0, "{preview}");
    assert_eq!(preview["data"]["changes"]["documentation"], true);
    assert_ne!(preview["data"]["candidate_digest"], old_digest);
    let show = dispatch(
        "show",
        &json!({"root":root.path(),"global_dir":global.path(),"id":"local/marker"}),
    )
    .unwrap();
    assert_eq!(show["data"]["digest"], old_digest);

    fs::write(root.path().join("note.txt"), "bad here").unwrap();
    let checked = dispatch("check", &common).unwrap();
    let id = checked["diagnostics"][0]["finding_id"].as_str().unwrap();
    let detail = dispatch(
        "inspect",
        &json!({"root":root.path(),"global_dir":global.path(),"finding_id":id}),
    )
    .unwrap();
    assert_eq!(detail["exit_code"], 0, "{detail}");
    assert_eq!(detail["data"]["observation"], "present");
    assert!(detail["data"]["rule_markdown"]
        .as_str()
        .unwrap()
        .contains("Review markers"));
    assert_eq!(detail["data"]["finding"]["finding_id"], id);
}
