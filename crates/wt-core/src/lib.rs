//! The application-independent Watchtower command implementation.
//!
//! The CLI is deliberately a thin adapter.  It parses command-line syntax and
//! supplies an options object; this crate owns validation, filesystem policy,
//! execution, and the result envelopes.

mod cache;
mod config;
mod coordination;
mod digest;
mod discovery;
mod engine_identity;
mod fixtures;
mod formatting;
mod inspection;
mod protocol;
mod reviews;
mod rule;
mod runner;
mod selection;
pub mod worker;

use anyhow::{anyhow, Result};
use serde::de::{DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde::Deserializer;
use serde_json::Value;
use std::collections::HashSet;
use std::fmt;

pub use digest::{digest_bytes, digest_package, digest_text};

/// The single contract version shared by every wt document kind: rule and
/// submission packages, configuration, fixtures/tests, review records, and
/// the result protocol. wt has no external users, so there is no reason to
/// version these independently; a breaking change to any one of them bumps
/// this constant for all of them.
pub const CONTRACT_VERSION: u64 = 1;

/// Return the installed rule/submission schema. wt accepts exactly
/// `CONTRACT_VERSION`; any other requested version is an error.
pub fn package_schema(name: &str, version: u64) -> Result<Value> {
    let source = match name {
        "rule" => include_str!("../../../schemas/rule.schema.json"),
        "submission" => include_str!("../../../schemas/submission.schema.json"),
        _ => return Err(anyhow!("unknown package schema {name}")),
    };
    if version != CONTRACT_VERSION {
        return Err(anyhow!("unsupported {name} schema version {version}"));
    }
    parse_json(source)
}

/// Parse strict JSON, including rejecting duplicate object keys at every level.
pub fn parse_json(input: &str) -> Result<Value> {
    let mut deserializer = serde_json::Deserializer::from_str(input);
    let value = DuplicateAware.deserialize(&mut deserializer)?;
    deserializer.end()?;
    Ok(value)
}

struct DuplicateAware;

impl<'de> DeserializeSeed<'de> for DuplicateAware {
    type Value = Value;

    fn deserialize<D>(self, deserializer: D) -> std::result::Result<Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(ValueVisitor)
    }
}

struct ValueVisitor;

impl<'de> Visitor<'de> for ValueVisitor {
    type Value = Value;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON value")
    }

    fn visit_unit<E>(self) -> std::result::Result<Value, E>
    where
        E: serde::de::Error,
    {
        Ok(Value::Null)
    }

    fn visit_bool<E>(self, value: bool) -> std::result::Result<Value, E>
    where
        E: serde::de::Error,
    {
        Ok(Value::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> std::result::Result<Value, E>
    where
        E: serde::de::Error,
    {
        Ok(Value::Number(value.into()))
    }

    fn visit_u64<E>(self, value: u64) -> std::result::Result<Value, E>
    where
        E: serde::de::Error,
    {
        Ok(Value::Number(value.into()))
    }

    fn visit_f64<E>(self, value: f64) -> std::result::Result<Value, E>
    where
        E: serde::de::Error,
    {
        serde_json::Number::from_f64(value)
            .map(Value::Number)
            .ok_or_else(|| E::custom("non-finite JSON number"))
    }

    fn visit_str<E>(self, value: &str) -> std::result::Result<Value, E>
    where
        E: serde::de::Error,
    {
        Ok(Value::String(value.to_owned()))
    }

    fn visit_string<E>(self, value: String) -> std::result::Result<Value, E>
    where
        E: serde::de::Error,
    {
        Ok(Value::String(value))
    }

    fn visit_seq<A>(self, mut access: A) -> std::result::Result<Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(value) = access.next_element_seed(DuplicateAware)? {
            values.push(value);
        }
        Ok(Value::Array(values))
    }

    fn visit_map<A>(self, mut access: A) -> std::result::Result<Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut values = serde_json::Map::new();
        let mut keys = HashSet::new();
        while let Some(key) = access.next_key::<String>()? {
            if !keys.insert(key.clone()) {
                return Err(serde::de::Error::custom(format!(
                    "duplicate JSON object key {key:?}"
                )));
            }
            values.insert(key, access.next_value_seed(DuplicateAware)?);
        }
        Ok(Value::Object(values))
    }
}

/// Dispatch one already-parsed CLI command options object.
pub fn dispatch_with_worker(
    command: &str,
    options: &Value,
    executable: &std::path::Path,
) -> Result<Value> {
    let mut options = options.clone();
    options["_worker_executable"] = serde_json::json!(executable);
    dispatch(command, &options)
}

pub fn dispatch(command: &str, options: &Value) -> Result<Value> {
    if !options.is_object() {
        return Err(anyhow!("options must be a JSON object"));
    }
    let detail = options
        .get("detail")
        .and_then(Value::as_str)
        .unwrap_or("summary");
    let result = match runner::dispatch(command, options) {
        Ok(value) => value,
        Err(error) => serde_json::json!({
            "schema_version": CONTRACT_VERSION,
            "command": command,
            "status": "error",
            "exit_code": 2,
            "error": error.to_string()
        }),
    };
    Ok(protocol::project(result, detail))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_json_keys_are_rejected() {
        assert!(parse_json(r#"{"a": 1, "a": 2}"#).is_err());
        assert!(parse_json(r#"{"outer": [{"a": 1, "a": 2}]}"#).is_err());
    }

    #[test]
    fn dispatch_always_stamps_the_contract_version_for_command_errors() {
        let result = dispatch("not-a-command", &serde_json::json!({})).unwrap();
        assert_eq!(result["schema_version"], CONTRACT_VERSION);
        assert_eq!(result["exit_code"], 2);
    }
}
