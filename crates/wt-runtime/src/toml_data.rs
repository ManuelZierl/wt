//! `toml.v1`: structured-data access to TOML documents, backed by `toml-span`
//! (a span-preserving parser), converted eagerly into an owned tree so the
//! runtime never carries a lifetime tied to the source file.
//!
//! Parser version is part of runtime semantics: reported by `wt capabilities`
//! (see `TOML_PARSER_CRATE`), same as ast.v1 grammar/engine versions. A file
//! that does not parse cleanly as TOML is an analysis gap (see `parse`),
//! never a silent no-match, matching the `ast.v1` parse-error-node contract.

use crate::shared_text::SharedText;
use anyhow::{anyhow, bail, Result};
use std::sync::Arc;
use toml_span::value::ValueInner;

/// Identity of the parser backing `toml.v1`, part of runtime semantics.
/// Reported by `wt capabilities` alongside the ast.v1 engine/grammar
/// versions, and folded into the review-evidence engine identity for rules
/// that declare `toml.v1`.
pub const TOML_PARSER_CRATE: &str = "toml-span-0.7.1";

/// Recursion ceiling for nested tables/arrays/inline-tables, guarding the
/// recursive converter below against a pathologically deep document.
pub const MAX_TOML_DEPTH: usize = 128;
/// Total node ceiling (every table, array, and scalar counts once) across one
/// parsed document, bounding the memory a single `file.toml()` call can
/// retain regardless of how deeply or widely it is nested.
pub const MAX_TOML_NODES: usize = 200_000;

/// One TOML value: a table, array, or scalar, with the byte span it occupies
/// in the original source. Cheap to clone (an `Arc` handle); shared with
/// every other handle derived from the same parse via `get`/`items`/`value`.
#[derive(Clone)]
pub struct TomlValue {
    pub file: usize,
    node: Arc<TomlValueNode>,
}

/// One `table` entry: a key (with its own span, independent of the value's)
/// and the value assigned to it.
#[derive(Clone)]
pub struct TomlEntry {
    pub file: usize,
    node: Arc<TomlEntryNode>,
}

struct TomlValueNode {
    span: (usize, usize),
    /// The exact source slice at `span`: the decoded string content for a
    /// `String` value (see `TomlKind::String`), the raw source text for
    /// every other kind (a table's `raw_text` is its whole `[section]` or
    /// `{ ... }` source, not a synthesized representation).
    raw_text: SharedText,
    kind: TomlKind,
}

enum TomlKind {
    Table(Vec<Arc<TomlEntryNode>>),
    Array(Vec<Arc<TomlValueNode>>),
    String(SharedText),
    // Integer/float/boolean scalars carry no separate payload: WRL1 exposes
    // them only through `.text` (their raw source representation, e.g. `"3"`,
    // `"1.5"`, `"true"`), backed by `raw_text` like every non-string kind.
    Integer,
    Float,
    Boolean,
}

struct TomlEntryNode {
    key: SharedText,
    key_span: (usize, usize),
    value: Arc<TomlValueNode>,
}

impl TomlValue {
    /// The value's kind name: `"table"`, `"array"`, `"string"`, `"integer"`,
    /// `"float"`, or `"boolean"`. `toml-span` does not parse TOML datetimes
    /// (see `parse`), so that kind never appears here.
    pub fn kind_name(&self) -> &'static str {
        match &self.node.kind {
            TomlKind::Table(_) => "table",
            TomlKind::Array(_) => "array",
            TomlKind::String(_) => "string",
            TomlKind::Integer => "integer",
            TomlKind::Float => "float",
            TomlKind::Boolean => "boolean",
        }
    }

    /// String content for a `string` value; the raw source text at `span`
    /// for every other kind.
    pub fn text(&self) -> SharedText {
        match &self.node.kind {
            TomlKind::String(content) => content.clone(),
            _ => self.node.raw_text.clone(),
        }
    }

    pub fn span(&self) -> (usize, usize) {
        self.node.span
    }

    /// The same value, restamped for a different file index. The parsed tree
    /// itself is file-index-agnostic (shared across cache hits); only the
    /// handle's `file` field distinguishes which authorized file a span
    /// belongs to, same idea as `MatchValue`'s cache-hit `file` remap.
    pub fn with_file(&self, file: usize) -> TomlValue {
        TomlValue {
            file,
            node: Arc::clone(&self.node),
        }
    }

    /// The direct child value named `key`, or `None` when this is not a
    /// table or has no such key. Not recursive: a caller chains `.get(...)`
    /// for a dotted path, same as chaining `.get` on a nested `serde_json`
    /// object.
    pub fn get(&self, key: &str) -> Option<TomlValue> {
        let TomlKind::Table(entries) = &self.node.kind else {
            return None;
        };
        entries
            .iter()
            .find(|entry| entry.key == key)
            .map(|entry| TomlValue {
                file: self.file,
                node: Arc::clone(&entry.value),
            })
    }

    /// Every entry of this table, in source order (by key span), or an empty
    /// sequence when this is not a table.
    pub fn entries(&self) -> Vec<TomlEntry> {
        let TomlKind::Table(entries) = &self.node.kind else {
            return Vec::new();
        };
        entries
            .iter()
            .map(|entry| TomlEntry {
                file: self.file,
                node: Arc::clone(entry),
            })
            .collect()
    }

    /// Every element of this array, in source order, or an empty sequence
    /// when this is not an array.
    pub fn items(&self) -> Vec<TomlValue> {
        let TomlKind::Array(items) = &self.node.kind else {
            return Vec::new();
        };
        items
            .iter()
            .map(|item| TomlValue {
                file: self.file,
                node: Arc::clone(item),
            })
            .collect()
    }
}

impl TomlEntry {
    pub fn key(&self) -> SharedText {
        self.node.key.clone()
    }

    pub fn key_span(&self) -> (usize, usize) {
        self.node.key_span
    }

    pub fn value(&self) -> TomlValue {
        TomlValue {
            file: self.file,
            node: Arc::clone(&self.node.value),
        }
    }
}

/// Parse `source` as TOML, eagerly converting the whole document into an
/// owned tree decoupled from `source`'s lifetime, stamped with `file` (the
/// authorized-file index every span reported from it refers back to). Any
/// parse failure — including a TOML datetime literal, which this parser does
/// not support — is returned as an error; the caller (see
/// `QueryArena::toml_tree`) turns that into a `WT201` analysis gap for the
/// file, never a silent no-match or partial tree.
pub fn parse(file: usize, source: &str) -> Result<TomlValue> {
    let root =
        toml_span::parse(source).map_err(|error| anyhow!("{}", format_parse_error(&error)))?;
    let mut nodes = 0usize;
    let node = convert(source, &root, 0, &mut nodes)?;
    Ok(TomlValue { file, node })
}

fn format_parse_error(error: &toml_span::Error) -> String {
    format!(
        "does not parse cleanly as TOML at byte {}: {error}",
        error.span.start
    )
}

fn convert(
    source: &str,
    value: &toml_span::value::Value<'_>,
    depth: usize,
    nodes: &mut usize,
) -> Result<Arc<TomlValueNode>> {
    if depth > MAX_TOML_DEPTH {
        bail!("toml.v1 document nesting exceeds {MAX_TOML_DEPTH} levels");
    }
    *nodes = nodes
        .checked_add(1)
        .ok_or_else(|| anyhow!("toml.v1 document node count overflow"))?;
    if *nodes > MAX_TOML_NODES {
        bail!("toml.v1 document exceeds {MAX_TOML_NODES} nodes");
    }
    let span = (value.span.start, value.span.end);
    let raw_text = slice_span(source, span)?;
    let kind = match value.as_ref() {
        ValueInner::Table(table) => {
            let mut entries = Vec::with_capacity(table.len());
            for (key, child) in table.iter() {
                let key_span = (key.span.start, key.span.end);
                let converted = convert(source, child, depth + 1, nodes)?;
                entries.push(Arc::new(TomlEntryNode {
                    key: SharedText::from(key.name.as_ref()),
                    key_span,
                    value: converted,
                }));
            }
            // `toml-span` groups entries in a `BTreeMap<Key, _>` sorted by
            // key name; re-sort by key span so entries()/iteration order
            // follows the source, which is what a diagnostic-authoring rule
            // expects and matches ast.v1/text.v1 sequences (source order).
            entries.sort_by_key(|entry| entry.key_span.0);
            TomlKind::Table(entries)
        }
        ValueInner::Array(items) => {
            let mut converted = Vec::with_capacity(items.len());
            for item in items {
                converted.push(convert(source, item, depth + 1, nodes)?);
            }
            TomlKind::Array(converted)
        }
        ValueInner::String(text) => TomlKind::String(SharedText::from(text.as_ref())),
        ValueInner::Integer(_) => TomlKind::Integer,
        ValueInner::Float(_) => TomlKind::Float,
        ValueInner::Boolean(_) => TomlKind::Boolean,
    };
    Ok(Arc::new(TomlValueNode {
        span,
        raw_text,
        kind,
    }))
}

/// Slice `source` at a parser-reported byte span, defensively: `toml-span`
/// always reports spans on UTF-8 boundaries for valid input, but this never
/// turns a parser bug into a panic (the assert inside `SharedText::slice`).
fn slice_span(source: &str, span: (usize, usize)) -> Result<SharedText> {
    if span.1 < span.0
        || span.1 > source.len()
        || !source.is_char_boundary(span.0)
        || !source.is_char_boundary(span.1)
    {
        bail!("toml.v1 parser produced an out-of-range span");
    }
    Ok(SharedText::from(source).slice(span.0..span.1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tables_arrays_and_scalars_with_spans() {
        let source =
            "name = \"kea-core\"\ncount = 3\nratio = 1.5\nenabled = true\ntags = [\"a\", \"b\"]\n";
        let value = parse(0, source).unwrap();
        assert_eq!(value.kind_name(), "table");
        let name = value.get("name").unwrap();
        assert_eq!(name.kind_name(), "string");
        assert_eq!(name.text(), "kea-core");
        let count = value.get("count").unwrap();
        assert_eq!(count.kind_name(), "integer");
        assert_eq!(count.text(), "3");
        let ratio = value.get("ratio").unwrap();
        assert_eq!(ratio.kind_name(), "float");
        let enabled = value.get("enabled").unwrap();
        assert_eq!(enabled.kind_name(), "boolean");
        let tags = value.get("tags").unwrap();
        assert_eq!(tags.kind_name(), "array");
        let items = tags.items();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].text(), "a");
        assert!(value.get("missing").is_none());
    }

    #[test]
    fn dotted_keys_and_inline_tables_get_correct_key_and_value_spans() {
        let source = "[dependencies]\nforbidden.workspace = true\nforbidden.path = \"..\"\nalias = { package = \"forbidden-crate\", version = \"1\" }\n";
        let value = parse(0, source).unwrap();
        let deps = value.get("dependencies").unwrap();
        let forbidden = deps.get("forbidden").unwrap();
        let path = forbidden.get("path").unwrap();
        assert_eq!(path.kind_name(), "string");
        assert_eq!(path.text(), "..");
        let (start, end) = path.span();
        // A string value's span covers its decoded content, not the
        // surrounding quote characters.
        assert_eq!(&source[start..end], "..");
        let alias = deps.get("alias").unwrap();
        assert_eq!(alias.kind_name(), "table");
        let package = alias.get("package").unwrap();
        assert_eq!(package.text(), "forbidden-crate");
        let entries = alias.entries();
        assert_eq!(entries.len(), 2);
        let package_entry = entries
            .iter()
            .find(|entry| entry.key() == "package")
            .unwrap();
        let (key_start, key_end) = package_entry.key_span();
        assert_eq!(&source[key_start..key_end], "package");
    }

    #[test]
    fn rejects_a_toml_datetime_literal_as_a_parse_error() {
        let source = "a = 1979-05-27T07:32:00Z\n";
        assert!(parse(0, source).is_err());
    }

    #[test]
    fn rejects_malformed_toml_as_a_parse_error() {
        let source = "[dependencies\nfoo = \"1\"\n";
        assert!(parse(0, source).is_err());
    }
}
