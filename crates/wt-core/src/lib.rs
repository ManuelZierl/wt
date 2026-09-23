//! The application-independent Watchtower command implementation.
//!
//! The CLI is deliberately a thin adapter.  It parses command-line syntax and
//! supplies an options object; this crate owns validation, filesystem policy,
//! execution, and the schema-v2 result envelopes.

mod cache;
mod config;
mod digest;
mod discovery;
mod fixtures;
mod rule;
mod runner;
mod selection;
mod waivers;
pub mod worker;

use anyhow::{anyhow, Result};
use serde::de::{DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde::Deserializer;
use serde_json::Value;
use std::collections::HashSet;
use std::fmt;

pub use digest::{digest_bytes, digest_package, digest_text};

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
    match runner::dispatch(command, options) {
        Ok(value) => Ok(value),
        Err(error) => Ok(serde_json::json!({
            "schema_version": 2,
            "command": command,
            "status": "error",
            "exit_code": 2,
            "error": error.to_string()
        })),
    }
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
    fn dispatch_always_returns_schema_v2_for_command_errors() {
        let result = dispatch("not-a-command", &serde_json::json!({})).unwrap();
        assert_eq!(result["schema_version"], 2);
        assert_eq!(result["exit_code"], 2);
    }
}
