use anyhow::{bail, Result};
use serde_json::{json, Value};

/// Convert the internal complete result into an advertised wire protocol.
/// Never project policy that the older format cannot express.
pub fn project(mut value: Value, version: u64, detail: &str) -> Result<Value> {
    if value["command"] == "capabilities" || value.get("$schema").is_some() {
        return Ok(value);
    }
    if version == 2 {
        match value["command"].as_str() {
            Some("inspect" | "guide") => bail!("result protocol 2 cannot represent this revision-3 command; use --output-version 3"),
            Some("update") => {
                if value["preview"] == true {
                    bail!("result protocol 2 cannot represent update preview; use --output-version 3")
                }
                for key in ["changes", "previous_digest", "candidate_digest"] {
                    value.as_object_mut().unwrap().remove(key);
                }
            }
            _ => {}
        }
        if value["command"] == "config" {
            if value["expectations"]
                .as_array()
                .is_some_and(|items| !items.is_empty())
            {
                bail!("result protocol 2 cannot represent coverage expectations; use --output-version 3")
            }
            project_legacy_profile(&mut value)?;
            value.as_object_mut().unwrap().remove("configured");
            value.as_object_mut().unwrap().remove("effective");
            value.as_object_mut().unwrap().remove("coverage");
            value.as_object_mut().unwrap().remove("expectations");
            value["schema_version"] = json!(2);
        }
        if value["command"] == "explain" {
            if value["coverage_expectations"]
                .as_array()
                .is_some_and(|items| !items.is_empty())
            {
                bail!("result protocol 2 cannot represent coverage expectations; use --output-version 3")
            }
            value
                .as_object_mut()
                .unwrap()
                .remove("coverage_expectations");
        }
        if value["command"] == "check" {
            if detail != "full" && value["scope"].is_object() {
                bail!("result protocol 2 requires --detail full")
            }
            if value["preview"] == true {
                bail!(
                    "result protocol 2 cannot represent candidate preview; use --output-version 3"
                )
            }
            if value["coverage_expectations"]
                .as_array()
                .is_some_and(|items| !items.is_empty())
            {
                bail!("result protocol 2 cannot represent coverage expectations; use --output-version 3")
            }
            value
                .as_object_mut()
                .unwrap()
                .remove("coverage_expectations");
            if let Some(stats) = value.get_mut("stats").and_then(Value::as_object_mut) {
                stats.remove("measurement");
                if let Some(runtime) = stats.get_mut("runtime").and_then(Value::as_object_mut) {
                    for key in [
                        "compilation",
                        "transport",
                        "admitted_jobs",
                        "worker_reservation_bytes",
                        "parent_reservation_bytes",
                        "total_scheduling_bytes",
                        "peak_retained_bytes",
                        "residual_iterations",
                    ] {
                        runtime.remove(key);
                    }
                }
            }
            if let Some(policy) = value
                .get_mut("effective_policy")
                .and_then(Value::as_object_mut)
            {
                policy.remove("coverage");
                policy.remove("expectations");
                policy.insert("schema_version".to_owned(), json!(2));
            }
            if let Some(policy) = value.get_mut("effective_policy") {
                project_legacy_profile(policy)?;
            }
        }
        return Ok(value);
    }
    if version != 3 {
        bail!("unsupported result protocol {version}; installed versions are 2 and 3")
    }
    let command = value["command"].as_str().unwrap_or("command").to_owned();
    value["schema_version"] = json!(3);
    value["detail"] = json!(detail);
    if command == "check" && value.get("complete").is_some() {
        let full = detail == "full";
        let suppressed_count = value["suppressed"].as_array().map_or(0, Vec::len);
        let summary = &mut value["summary"];
        summary["raw_occurrences"] = summary["raw_findings"].clone();
        summary["reviewed_occurrences"] = summary["reviewed_findings"].clone();
        summary["actionable_occurrences"] = summary["actionable_findings"].clone();
        summary["suppressed_occurrences"] = json!(suppressed_count);
        value["inventory"] =
            json!({"reviewed":full,"suppressed":full,"files":full,"review_records":full});
        if !full {
            for key in ["reviewed", "suppressed", "files", "review_records"] {
                value.as_object_mut().unwrap().remove(key);
            }
        }
    } else {
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
        object.insert("notices".to_owned(), json!([]));
        object.insert("data".to_owned(), Value::Object(payload));
    }
    Ok(value)
}

fn project_legacy_profile(value: &mut Value) -> Result<()> {
    let Some(object) = value.as_object_mut() else {
        return Ok(());
    };
    let explicit_optimizer =
        object
            .get("origins")
            .and_then(Value::as_array)
            .is_some_and(|origins| {
                origins
                    .iter()
                    .rev()
                    .find(|entry| entry["key"] == "optimizer.mode")
                    .is_some_and(|entry| entry["origin"] == "cli")
            });
    if object
        .get("runtime")
        .is_some_and(|runtime| *runtime != json!(crate::config::RuntimeConfig::default()))
        || (!explicit_optimizer
            && object.get("optimizer").is_some_and(|optimizer| {
                *optimizer != json!(crate::config::OptimizerConfig::default())
            }))
    {
        bail!("result protocol 2 cannot represent a changed runtime or optimizer profile; use --output-version 3")
    }
    object.remove("runtime");
    object.remove("optimizer");
    if let Some(origins) = object.get_mut("origins").and_then(Value::as_array_mut) {
        origins.retain(|entry| {
            !entry["key"]
                .as_str()
                .is_some_and(|key| key.starts_with("runtime.") || key.starts_with("optimizer."))
        });
    }
    Ok(())
}
