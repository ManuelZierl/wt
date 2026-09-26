use serde_json::{json, Value};

/// Project the internal complete result into the advertised wire protocol
/// (`CONTRACT_VERSION`). Ordinary command results are wrapped in an envelope
/// carrying `stages`, `notices`, `errors`, and command-specific `data`;
/// `check` results and standalone documents (`capabilities`, `schema`) keep
/// their own top-level shape.
pub fn project(mut value: Value, detail: &str) -> Value {
    if value["command"] == "capabilities" || value.get("$schema").is_some() {
        return value;
    }
    let command = value["command"].as_str().unwrap_or("command").to_owned();
    value["schema_version"] = json!(crate::CONTRACT_VERSION);
    value["detail"] = json!(detail);
    if command == "check" && value.get("complete").is_some() {
        let full = detail == "full";
        let summary = &mut value["summary"];
        summary["raw_occurrences"] = summary["raw_findings"].clone();
        summary["reviewed_occurrences"] = summary["reviewed_findings"].clone();
        summary["actionable_occurrences"] = summary["actionable_findings"].clone();
        value["inventory"] = json!({"reviewed":full,"files":full,"review_records":full});
        if !full {
            for key in ["reviewed", "files", "review_records"] {
                value.as_object_mut().unwrap().remove(key);
            }
        }
        return value;
    }
    let object = value.as_object_mut().unwrap();
    let data = ["schema_version", "command", "status", "exit_code", "detail"];
    let mut payload = serde_json::Map::new();
    let keys = object
        .keys()
        .filter(|key| !data.contains(&key.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    for key in keys {
        payload.insert(key.clone(), object.remove(&key).unwrap());
    }
    if let Some(error) = payload.remove("error") {
        object.insert("errors".to_owned(), json!([{"error":error}]));
    } else {
        object.insert("errors".to_owned(), json!([]));
    }
    // A command may report its own advisory notices (e.g. `validate`'s
    // capability hints) inline in its payload; promote them to the shared
    // envelope field instead of leaving them nested under `data`. Absent
    // that, the envelope carries an empty array, same as before.
    let notices = payload.remove("notices").unwrap_or_else(|| json!([]));
    let stages = match command.as_str() {
        "new" | "update" => {
            let mut stages = vec!["validated", "formatted"];
            if payload.get("tests").and_then(Value::as_str) == Some("passed") {
                stages.push("fixtures_executed");
            }
            stages
        }
        "validate" => vec!["validated"],
        "test" => vec!["fixtures_executed"],
        "review" => vec!["fresh_check", "evidence_verified", "decision_written"],
        "inspect" => vec!["fresh_check", "evidence_evaluated"],
        _ => vec![],
    };
    if command == "validate" {
        payload.remove("plan");
        if let Some(rules) = payload.get_mut("rules").and_then(Value::as_array_mut) {
            for rule in rules {
                rule.as_object_mut().map(|rule| rule.remove("plan"));
            }
        }
    }
    object.insert("stages".to_owned(), json!(stages));
    object.insert("notices".to_owned(), notices);
    object.insert("data".to_owned(), Value::Object(payload));
    value
}
