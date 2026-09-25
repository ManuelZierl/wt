//! The Watchtower Rule Language runtime.
//!
//! Rhai is used as a pinned frontend. The unoptimized AST is validated against
//! WRL1 and interpreted by the restricted runtime adapter; no arbitrary Rhai
//! function is ever registered or executed. A separate complete WT IR is not
//! claimed here.

use anyhow::{anyhow, bail, Result};
use globset::{GlobBuilder, GlobMatcher};
use regex::{Regex, RegexBuilder};
use rhai::{
    ASTFlags, ASTNode, BinaryExpr, Engine, Expr, FnCallExpr, OptimizationLevel, Position, Stmt, AST,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as JsonValue};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::{Arc, Weak};

mod ast_match;
pub use ast_match::{
    supported_language_grammars as ast_supported_language_grammars,
    supported_languages as ast_supported_languages, AST_ENGINE,
};
use ast_match::{AstLanguage, AstMatch};
mod shared_text;
pub use shared_text::SharedText;
#[cfg(test)]
mod performance_tests;

const MAX_SOURCE: usize = 128 * 1024;
// Consumer budgets represent the original logical value cost, not the storage
// layout (which shrinks when payloads become shared).
const LOGICAL_VALUE_BYTES: usize = 104;
const MAX_FRONTEND_NODES: usize = 1_000_000;
const MAX_FILE_CEILING: usize = 64 * 1024 * 1024;
const MAX_STEPS: u64 = 1_000_000;
const MAX_REPOSITORY_STEPS: u64 = 10_000_000;
const MAX_MATCHES: usize = 10_000;
const MAX_SEQUENCE: usize = 100_000;
const MAX_REPOSITORY_FILES: usize = 10_000;
const MAX_REPOSITORY_SNAPSHOT_BYTES: usize = 100 * 1024 * 1024;
const MAX_DIAGNOSTICS: usize = 1_000;
const MAX_REGEX_SIZE: usize = 1024 * 1024;
const COMPILED_PATTERN_BYTES: usize = 1024 * 1024;
const COMPILED_SOURCE_MULTIPLIER: usize = 64;
const COMPILED_RESIDUAL_OVERHEAD: usize = 16 * 1024;
const PATTERN_REGISTRY_KEY_BYTES: usize = 64 * 1024;
const MAX_NATIVE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_REPOSITORY_NATIVE_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_RETAINED_BYTES: usize = 64 * 1024 * 1024;
const MAX_TEMPORARY_BYTES: usize = 64 * 1024 * 1024;

// These are cooperative runtime bounds. Process-level watchdogs and hard
// recovery belong to the caller's worker boundary, not this library API.

fn charge_consumer_budget(counter: &mut usize, bytes: usize) -> Result<()> {
    if bytes > MAX_TEMPORARY_BYTES.saturating_sub(*counter) {
        bail!("local temporary-value budget exceeded");
    }
    *counter += bytes;
    Ok(())
}

#[derive(Clone)]
pub struct SourceFile {
    pub path: String,
    pub text: SharedText,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeLimits {
    pub max_file_bytes: usize,
    pub file_steps: u64,
    pub file_native_bytes: u64,
    pub repository_steps: u64,
    pub repository_native_bytes: u64,
}

impl Default for RuntimeLimits {
    fn default() -> Self {
        Self {
            max_file_bytes: MAX_FILE_CEILING,
            file_steps: MAX_STEPS,
            file_native_bytes: MAX_NATIVE_BYTES,
            repository_steps: MAX_REPOSITORY_STEPS,
            repository_native_bytes: MAX_REPOSITORY_NATIVE_BYTES,
        }
    }
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct RawDiagnostic {
    pub path: String,
    pub code: String,
    pub start_byte: usize,
    pub end_byte: usize,
}

const REGEX_ENGINE: &str = "regex-1.12.4-utf8-leftmost-first-nonoverlap";

#[derive(Clone)]
struct Pattern {
    regex: Arc<Regex>,
    expression: String,
    flags: PatternFlags,
    captures: HashSet<String>,
    canonical_key: String,
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
struct PatternFlags {
    case_insensitive: bool,
    multi_line: bool,
    dot_matches_new_line: bool,
    ignore_whitespace: bool,
    crlf: bool,
}

#[derive(Default)]
struct PatternRegistry {
    entries: HashMap<String, Weak<Regex>>,
    key_bytes: usize,
}

impl PatternRegistry {
    fn clean_dead(&mut self) {
        self.entries.retain(|_, regex| regex.strong_count() != 0);
        self.key_bytes = self.entries.keys().map(String::len).sum();
    }
}

thread_local! {
    static COMPILED_PATTERNS: RefCell<PatternRegistry> = RefCell::new(PatternRegistry::default());
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
/// A source location retained by the WT frontend without depending on Rhai's
/// position type in the public plan.
pub struct SourceLocation {
    pub line: Option<usize>,
    pub column: Option<usize>,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
/// Control-flow context recorded for a query demand. This is inspection data;
/// it is not a host-language predicate evaluator.
pub struct GuardContext {
    pub kind: String,
    pub source: SourceLocation,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
/// Expanded regex semantics used by a query's canonical identity.
pub struct PatternSemantics {
    pub case_insensitive: bool,
    pub multi_line: bool,
    pub dot_matches_new_line: bool,
    pub ignore_whitespace: bool,
    pub crlf: bool,
    pub engine: String,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
/// WT-owned native query description extracted from validated source.
pub struct QueryTemplate {
    pub call_site: usize,
    pub source: SourceLocation,
    pub guards: Vec<GuardContext>,
    pub namespace: String,
    pub operation: String,
    pub result_shape: String,
    pub input_symbolic_expression: String,
    pub pattern_alias: Option<String>,
    pub pattern_expression: Option<String>,
    pub pattern_flags: Option<PatternSemantics>,
    pub canonical_identity: String,
    pub resolved_string_constants: BTreeMap<String, String>,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
/// WT-owned inspection IR. The current residual executor still interprets the
/// validated Rhai AST; this IR is the stable query/planning boundary.
pub struct RuleIr {
    pub execution: String,
    pub queries: Vec<QueryTemplate>,
}

#[derive(Clone)]
struct DiagnosticDef;

#[derive(Clone, Hash, PartialEq, Eq)]
struct QueryKey {
    path: String,
    digest: [u8; 32],
    expression: String,
    flags: PatternFlags,
    start: usize,
    end: usize,
    independent_slice: bool,
    operation: String,
}

#[derive(Clone, Hash, PartialEq, Eq)]
struct TextKey {
    path: String,
    digest: [u8; 32],
    operation: String,
    arguments: Vec<[u8; 32]>,
}

#[derive(Clone)]
struct MatchValue {
    file: usize,
    start: usize,
    end: usize,
    text: SharedText,
    groups: Arc<HashMap<String, Option<SharedText>>>,
    /// Byte spans of captured metavariables, populated for `ast.v1` matches
    /// only (regex matches leave this empty). Backs `.node(name)`.
    node_spans: Arc<HashMap<String, (usize, usize)>>,
    reportable: bool,
}

#[derive(Clone)]
struct SpanValue {
    file: usize,
    start: usize,
    end: usize,
}

#[derive(Clone)]
struct FileValue {
    index: usize,
}

#[derive(Clone)]
struct LineValue {
    file: usize,
    start: usize,
    end: usize,
    text: SharedText,
}

#[derive(Clone)]
enum Value {
    Unit,
    Bool(bool),
    Int(i64),
    Text(SharedText),
    File(FileValue),
    Span(SpanValue),
    Match(MatchValue),
    Line(LineValue),
    Repo,
    Sequence(Vec<Value>),
}

struct QueryEntry {
    result: QueryResult,
}

enum QueryResult {
    Complete(Vec<MatchValue>),
    Exists(bool),
    Failed(String),
}

enum CaptureResult {
    Complete(Option<MatchValue>),
    Failed(String),
}

enum TextResult {
    Complete(Value),
    Failed(String),
}

#[derive(Clone, Hash, PartialEq, Eq)]
struct DerivedKey {
    path: String,
    digest: [u8; 32],
    operation: String,
    expression: String,
    flags: PatternFlags,
}

type AstTreeCache = HashMap<(String, [u8; 32], &'static str), Arc<Result<ast_match::Tree, String>>>;

#[derive(Clone, Hash, PartialEq, Eq)]
struct AstQueryKey {
    path: String,
    digest: [u8; 32],
    language: &'static str,
    pattern: String,
    start: usize,
    end: usize,
    independent_slice: bool,
}

/// Per-run shared native query storage.
pub struct QueryArena {
    optimized: bool,
    regex: HashMap<QueryKey, QueryEntry>,
    capture_text: HashMap<DerivedKey, CaptureResult>,
    ast: HashMap<AstQueryKey, QueryEntry>,
    parsed_trees: AstTreeCache,
    text: HashMap<TextKey, TextResult>,
    stats: ArenaStats,
    retained_bytes: usize,
    text_retained_bytes: usize,
    text_scope: Option<(String, [u8; 32])>,
}

#[derive(Default)]
struct ArenaStats {
    logical_requests: usize,
    physical_query_evaluations: usize,
    text_evaluations: usize,
    shared_result_hits: usize,
    parser_evaluations: usize,
    logical_steps: u64,
    native_bytes: u64,
    temporary_bytes: usize,
    residual_invocations: usize,
    residual_iterations: u64,
    peak_retained_bytes: usize,
    cache_key_bytes_hashed: u64,
}

impl QueryArena {
    pub fn new(optimized: bool) -> Self {
        Self {
            optimized,
            regex: HashMap::new(),
            capture_text: HashMap::new(),
            ast: HashMap::new(),
            parsed_trees: HashMap::new(),
            text: HashMap::new(),
            stats: ArenaStats::default(),
            retained_bytes: 0,
            text_retained_bytes: 0,
            text_scope: None,
        }
    }

    pub fn stats(&self) -> JsonValue {
        json!({
            "optimized": self.optimized,
            "logical_query_requests": self.stats.logical_requests,
            "physical_query_evaluations": self.stats.physical_query_evaluations + self.stats.text_evaluations + self.stats.parser_evaluations,
            "regex_evaluations": self.stats.physical_query_evaluations,
            "text_evaluations": self.stats.text_evaluations,
            "shared_result_hits": self.stats.shared_result_hits,
            "parser_evaluations": self.stats.parser_evaluations,
            "logical_steps": self.stats.logical_steps,
            "native_bytes": self.stats.native_bytes,
            "temporary_bytes": self.stats.temporary_bytes,
            "retained_memory_bytes": self.retained_bytes,
            "peak_retained_bytes": self.stats.peak_retained_bytes.max(self.retained_bytes),
            "residual_invocations": self.stats.residual_invocations,
            "residual_iterations": self.stats.residual_iterations,
            "cache_key_bytes_hashed": self.stats.cache_key_bytes_hashed,
        })
    }

    fn record_execution(
        &mut self,
        steps: u64,
        native_bytes: u64,
        temporary_bytes: usize,
        iterations: u64,
    ) {
        self.stats.logical_steps = self.stats.logical_steps.saturating_add(steps);
        self.stats.native_bytes = self.stats.native_bytes.saturating_add(native_bytes);
        self.stats.temporary_bytes = self.stats.temporary_bytes.saturating_add(temporary_bytes);
        self.stats.residual_invocations = self.stats.residual_invocations.saturating_add(1);
        self.stats.residual_iterations = self.stats.residual_iterations.saturating_add(iterations);
    }

    /// Release retained derived values for the completed file-local transaction.
    /// Counters intentionally remain cumulative for runner statistics.
    pub fn clear_file_results(&mut self) {
        self.stats.peak_retained_bytes = self.stats.peak_retained_bytes.max(self.retained_bytes);
        self.regex.clear();
        self.capture_text.clear();
        self.ast.clear();
        self.parsed_trees.clear();
        self.text.clear();
        self.retained_bytes = self.retained_bytes.saturating_sub(self.text_retained_bytes);
        self.text_retained_bytes = 0;
        self.text_scope = None;
        self.retained_bytes = 0;
    }

    fn text_query<F>(
        &mut self,
        key: TextKey,
        file_index: Option<usize>,
        output_bytes: usize,
        temporary_bytes: &mut usize,
        compute: F,
    ) -> Result<Value>
    where
        F: FnOnce() -> Result<Value>,
    {
        self.stats.logical_requests += 1;
        let cacheable = self.optimized && !key.path.is_empty();
        let scope = (key.path.clone(), key.digest);
        if cacheable && self.text_scope.as_ref() != Some(&scope) {
            self.stats.peak_retained_bytes =
                self.stats.peak_retained_bytes.max(self.retained_bytes);
            self.retained_bytes = self.retained_bytes.saturating_sub(self.text_retained_bytes);
            self.text.clear();
            self.text_retained_bytes = 0;
            self.text_scope = Some(scope);
        }
        if cacheable {
            if let Some(result) = self.text.get(&key) {
                self.stats.shared_result_hits += 1;
                return match result {
                    TextResult::Complete(value) => {
                        charge_consumer_budget(temporary_bytes, output_bytes)?;
                        let mut value = value.clone();
                        remap_line_files(&mut value, file_index);
                        Ok(value)
                    }
                    TextResult::Failed(error) => bail!("{error}"),
                };
            }
        }
        charge_consumer_budget(temporary_bytes, output_bytes)?;
        self.stats.text_evaluations += 1;
        let result = compute();
        match result {
            Ok(value) => {
                if cacheable {
                    let retained = value_size(&value);
                    if self.retained_bytes.saturating_add(retained) > MAX_RETAINED_BYTES {
                        let error = "retained query result budget exceeded".to_owned();
                        self.text.insert(key, TextResult::Failed(error.clone()));
                        bail!("{error}");
                    }
                    self.retained_bytes += retained;
                    self.text_retained_bytes += retained;
                    self.text.insert(key, TextResult::Complete(value.clone()));
                }
                Ok(value)
            }
            Err(error) => {
                if cacheable {
                    self.text.insert(key, TextResult::Failed(error.to_string()));
                }
                Err(error)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn find_all(
        &mut self,
        file_index: usize,
        file: &SourceFile,
        pattern: &Pattern,
        start: usize,
        end: usize,
        independent_slice: bool,
        operation: &str,
        temporary_bytes: &mut usize,
    ) -> Result<Vec<MatchValue>> {
        self.stats.logical_requests += 1;
        let key = QueryKey {
            path: file.path.clone(),
            digest: file.text.digest(&mut self.stats.cache_key_bytes_hashed),
            expression: pattern.expression.clone(),
            flags: pattern.flags,
            start,
            end,
            independent_slice,
            operation: operation.to_owned(),
        };
        if self.optimized {
            if let Some(entry) = self.regex.get(&key) {
                self.stats.shared_result_hits += 1;
                return match &entry.result {
                    QueryResult::Complete(matches) => {
                        charge_consumer_budget(
                            temporary_bytes,
                            matches.iter().map(match_value_size).sum::<usize>()
                                + matches.len() * LOGICAL_VALUE_BYTES,
                        )?;
                        Ok(matches
                            .iter()
                            .cloned()
                            .map(|mut matched| {
                                matched.file = file_index;
                                matched
                            })
                            .collect())
                    }
                    QueryResult::Exists(_) => bail!("invalid cached query result shape"),
                    QueryResult::Failed(error) => bail!("{error}"),
                };
            }
        }
        self.stats.physical_query_evaluations += 1;
        let haystack = file
            .text
            .get(start..end)
            .ok_or_else(|| anyhow!("invalid UTF-8 source region"))?;
        let result = (|| -> Result<Vec<MatchValue>> {
            let mut result = Vec::new();
            let mut pending_bytes = 0usize;
            for captures in pattern.regex.captures_iter(haystack) {
                if result.len() >= MAX_MATCHES {
                    bail!("regex result limit exceeded");
                }
                let whole = captures
                    .get(0)
                    .ok_or_else(|| anyhow!("regex returned a match without group 0"))?;
                let absolute_start = start + whole.start();
                let absolute_end = start + whole.end();
                let match_bytes = whole.as_str().len().saturating_add(64).saturating_add(
                    pattern
                        .captures
                        .iter()
                        .map(|name| {
                            name.len().saturating_add(24).saturating_add(
                                captures.name(name).map_or(0, |value| value.as_str().len()),
                            )
                        })
                        .sum::<usize>(),
                );
                pending_bytes = pending_bytes
                    .checked_add(match_bytes)
                    .ok_or_else(|| anyhow!("retained query result size overflow"))?;
                if self.retained_bytes.saturating_add(pending_bytes) > MAX_RETAINED_BYTES {
                    bail!("retained query result budget exceeded");
                }
                let mut groups = HashMap::new();
                for name in &pattern.captures {
                    groups.insert(
                        name.clone(),
                        captures.name(name).map(|capture| {
                            file.text
                                .slice(start + capture.start()..start + capture.end())
                        }),
                    );
                }
                result.push(MatchValue {
                    file: file_index,
                    start: absolute_start,
                    end: absolute_end,
                    text: file.text.slice(absolute_start..absolute_end),
                    groups: Arc::new(groups),
                    node_spans: Arc::new(HashMap::new()),
                    reportable: true,
                });
            }
            let bytes = result.iter().map(match_value_size).sum::<usize>();
            if bytes > MAX_RETAINED_BYTES
                || self.retained_bytes.saturating_add(bytes) > MAX_RETAINED_BYTES
            {
                bail!("retained query result budget exceeded");
            }
            Ok(result)
        })();
        if let Ok(matches) = &result {
            charge_consumer_budget(
                temporary_bytes,
                matches.iter().map(match_value_size).sum::<usize>()
                    + matches.len() * LOGICAL_VALUE_BYTES,
            )?;
        }
        if self.optimized {
            match &result {
                Ok(matches) => {
                    self.retained_bytes += matches.iter().map(match_value_size).sum::<usize>();
                    self.regex.insert(
                        key,
                        QueryEntry {
                            result: QueryResult::Complete(matches.clone()),
                        },
                    );
                }
                Err(error) => {
                    self.regex.insert(
                        key,
                        QueryEntry {
                            result: QueryResult::Failed(error.to_string()),
                        },
                    );
                }
            }
        }
        result
    }

    fn is_match(
        &mut self,
        file: &SourceFile,
        pattern: &Pattern,
        start: usize,
        end: usize,
        independent_slice: bool,
    ) -> Result<bool> {
        self.stats.logical_requests += 1;
        let key = QueryKey {
            path: file.path.clone(),
            digest: file.text.digest(&mut self.stats.cache_key_bytes_hashed),
            expression: pattern.expression.clone(),
            flags: pattern.flags,
            start,
            end,
            independent_slice,
            operation: "is_match".into(),
        };
        if self.optimized {
            if let Some(entry) = self.regex.get(&key) {
                self.stats.shared_result_hits += 1;
                return match &entry.result {
                    QueryResult::Exists(value) => Ok(*value),
                    QueryResult::Failed(error) => bail!("{error}"),
                    _ => bail!("invalid cached query result shape"),
                };
            }
        }
        self.stats.physical_query_evaluations += 1;
        let result = (|| -> Result<bool> {
            Ok(pattern.regex.is_match(
                file.text
                    .get(start..end)
                    .ok_or_else(|| anyhow!("invalid UTF-8 source region"))?,
            ))
        })();
        if self.optimized {
            match &result {
                Ok(value) => {
                    self.regex.insert(
                        key,
                        QueryEntry {
                            result: QueryResult::Exists(*value),
                        },
                    );
                }
                Err(error) => {
                    self.regex.insert(
                        key,
                        QueryEntry {
                            result: QueryResult::Failed(error.to_string()),
                        },
                    );
                }
            }
        }
        result
    }

    fn capture_text(
        &mut self,
        pattern: &Pattern,
        text: &SharedText,
        temporary_bytes: &mut usize,
    ) -> Result<Option<MatchValue>> {
        self.stats.logical_requests += 1;
        let key = DerivedKey {
            path: String::new(),
            digest: text.digest(&mut self.stats.cache_key_bytes_hashed),
            operation: "capture_text".into(),
            expression: pattern.expression.clone(),
            flags: pattern.flags,
        };
        if self.optimized {
            if let Some(entry) = self.capture_text.get(&key) {
                self.stats.shared_result_hits += 1;
                return match entry {
                    CaptureResult::Complete(value) => {
                        charge_consumer_budget(
                            temporary_bytes,
                            value.as_ref().map_or(0, match_value_size),
                        )?;
                        Ok(value.clone())
                    }
                    CaptureResult::Failed(error) => bail!("{error}"),
                };
            }
        }
        self.stats.physical_query_evaluations += 1;
        let result: Result<Option<MatchValue>> = (|| {
            let Some(captures) = pattern.regex.captures(text) else {
                return Ok(None);
            };
            let whole = captures
                .get(0)
                .ok_or_else(|| anyhow!("regex returned a match without group 0"))?;
            let bytes = whole.as_str().len().saturating_add(64).saturating_add(
                pattern
                    .captures
                    .iter()
                    .map(|name| {
                        name.len().saturating_add(24).saturating_add(
                            captures.name(name).map_or(0, |value| value.as_str().len()),
                        )
                    })
                    .sum::<usize>(),
            );
            if self.retained_bytes.saturating_add(bytes) > MAX_RETAINED_BYTES {
                bail!("retained query result budget exceeded");
            }
            let groups = pattern
                .captures
                .iter()
                .map(|name| {
                    (
                        name.clone(),
                        captures
                            .name(name)
                            .map(|value| text.slice(value.start()..value.end())),
                    )
                })
                .collect();
            Ok(Some(MatchValue {
                file: 0,
                start: 0,
                end: 0,
                text: text.slice(whole.start()..whole.end()),
                groups: Arc::new(groups),
                node_spans: Arc::new(HashMap::new()),
                reportable: false,
            }))
        })();
        if let Ok(value) = &result {
            charge_consumer_budget(temporary_bytes, value.as_ref().map_or(0, match_value_size))?;
        }
        if self.optimized {
            match &result {
                Ok(value) => {
                    let bytes = value.as_ref().map_or(0, match_value_size);
                    if self.retained_bytes.saturating_add(bytes) > MAX_RETAINED_BYTES {
                        let error = "retained query result budget exceeded".to_owned();
                        self.capture_text
                            .insert(key, CaptureResult::Failed(error.clone()));
                        return Err(anyhow!(error));
                    }
                    self.retained_bytes += bytes;
                    self.capture_text
                        .insert(key, CaptureResult::Complete(value.clone()));
                }
                Err(error) => {
                    self.capture_text
                        .insert(key, CaptureResult::Failed(error.to_string()));
                }
            }
        }
        result
    }

    /// Parse `file` under `language`, cached per (path, digest, language) for
    /// the current file-local transaction so repeated/nested `ast_match`
    /// calls in the same rule do not reparse. Any tree-sitter error node
    /// turns the parse into an analysis gap (never a silent no-match).
    fn ast_tree(
        &mut self,
        file: &SourceFile,
        digest: [u8; 32],
        language: AstLanguage,
    ) -> Arc<Result<ast_match::Tree, String>> {
        let key = (file.path.clone(), digest, language.name());
        if let Some(cached) = self.parsed_trees.get(&key) {
            return cached.clone();
        }
        let built = ast_match::parse_source(language, &file.text).and_then(|tree| {
            if ast_match::has_parse_error(&tree) {
                bail!(
                    "WT201 ast.v1 analysis gap: {:?} contains a parse error node under language {:?}",
                    file.path,
                    language.name()
                );
            }
            Ok(tree)
        });
        let entry = Arc::new(built.map_err(|error| error.to_string()));
        self.parsed_trees.insert(key, entry.clone());
        entry
    }

    #[allow(clippy::too_many_arguments)]
    fn ast_match(
        &mut self,
        file_index: usize,
        file: &SourceFile,
        language: AstLanguage,
        pattern_key: &str,
        pattern: &ast_grep_core::Pattern,
        start: usize,
        end: usize,
        independent_slice: bool,
        temporary_bytes: &mut usize,
    ) -> Result<Vec<MatchValue>> {
        self.stats.logical_requests += 1;
        let digest = file.text.digest(&mut self.stats.cache_key_bytes_hashed);
        let key = AstQueryKey {
            path: file.path.clone(),
            digest,
            language: language.name(),
            pattern: pattern_key.to_owned(),
            start,
            end,
            independent_slice,
        };
        if self.optimized {
            if let Some(entry) = self.ast.get(&key) {
                self.stats.shared_result_hits += 1;
                return match &entry.result {
                    QueryResult::Complete(matches) => {
                        charge_consumer_budget(
                            temporary_bytes,
                            matches.iter().map(match_value_size).sum::<usize>()
                                + matches.len() * LOGICAL_VALUE_BYTES,
                        )?;
                        Ok(matches
                            .iter()
                            .cloned()
                            .map(|mut matched| {
                                matched.file = file_index;
                                matched
                            })
                            .collect())
                    }
                    QueryResult::Exists(_) => bail!("invalid cached query result shape"),
                    QueryResult::Failed(error) => bail!("{error}"),
                };
            }
        }
        self.stats.parser_evaluations += 1;
        let tree_entry = self.ast_tree(file, digest, language);
        let result: Result<Vec<MatchValue>> = (|| {
            let tree = tree_entry
                .as_ref()
                .as_ref()
                .map_err(|error| anyhow!("{error}"))?;
            let scope = if independent_slice {
                Some((start, end))
            } else {
                None
            };
            let matches = ast_match::find_matches(tree, pattern, scope)?;
            let mut out = Vec::with_capacity(matches.len());
            let mut pending_bytes = 0usize;
            for AstMatch {
                start,
                end,
                captures,
            } in matches
            {
                let mut groups = HashMap::with_capacity(captures.len());
                let mut node_spans = HashMap::with_capacity(captures.len());
                let mut capture_bytes = 0usize;
                for (name, cstart, cend) in captures {
                    let text = file.text.slice(cstart..cend);
                    capture_bytes = capture_bytes
                        .saturating_add(name.len())
                        .saturating_add(24)
                        .saturating_add(text.len());
                    node_spans.insert(name.clone(), (cstart, cend));
                    groups.insert(name, Some(text));
                }
                let match_bytes = end
                    .saturating_sub(start)
                    .saturating_add(64)
                    .saturating_add(capture_bytes);
                pending_bytes = pending_bytes
                    .checked_add(match_bytes)
                    .ok_or_else(|| anyhow!("retained query result size overflow"))?;
                if self.retained_bytes.saturating_add(pending_bytes) > MAX_RETAINED_BYTES {
                    bail!("retained query result budget exceeded");
                }
                out.push(MatchValue {
                    file: file_index,
                    start,
                    end,
                    text: file.text.slice(start..end),
                    groups: Arc::new(groups),
                    node_spans: Arc::new(node_spans),
                    reportable: true,
                });
            }
            Ok(out)
        })();
        if let Ok(matches) = &result {
            charge_consumer_budget(
                temporary_bytes,
                matches.iter().map(match_value_size).sum::<usize>()
                    + matches.len() * LOGICAL_VALUE_BYTES,
            )?;
        }
        if self.optimized {
            match &result {
                Ok(matches) => {
                    self.retained_bytes += matches.iter().map(match_value_size).sum::<usize>();
                    self.ast.insert(
                        key,
                        QueryEntry {
                            result: QueryResult::Complete(matches.clone()),
                        },
                    );
                }
                Err(error) => {
                    self.ast.insert(
                        key,
                        QueryEntry {
                            result: QueryResult::Failed(error.to_string()),
                        },
                    );
                }
            }
        }
        result
    }
}

#[derive(Clone)]
pub struct Program {
    ast: AST,
    patterns: BTreeMap<String, Pattern>,
    diagnostics: BTreeMap<String, DiagnosticDef>,
    execution: Execution,
    path_globs: HashMap<String, GlobMatcher>,
    ast_patterns: BTreeMap<(AstLanguage, String), ast_grep_core::Pattern>,
    plan: JsonValue,
    rule_ir: RuleIr,
    compiled_residual_bytes: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Execution {
    File,
    Repository,
}

pub fn compile(manifest: &serde_json::Value, source: &str) -> Result<Program> {
    if source.len() > MAX_SOURCE {
        bail!("WT102 detector source exceeds 128 KiB");
    }
    if !source.is_char_boundary(source.len()) {
        bail!("WT100 detector source is not valid UTF-8");
    }
    let manifest_bytes = serde_json::to_vec(manifest).map_err(|error| {
        anyhow!("unable to serialize manifest for compiled reservation: {error}")
    })?;
    let object = manifest
        .as_object()
        .ok_or_else(|| anyhow!("WT100 manifest must be an object"))?;
    let code = object
        .get("code")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| anyhow!("WT100 manifest.code is required"))?;
    if code.get("language").and_then(JsonValue::as_str) != Some("wt-rule-1") {
        bail!("WT101 code.language must be wt-rule-1");
    }
    let execution = match object
        .get("execution")
        .and_then(JsonValue::as_str)
        .unwrap_or("file")
    {
        "file" => Execution::File,
        "repository" => Execution::Repository,
        other => bail!("WT100 unsupported execution mode {other:?}"),
    };
    let capabilities = code
        .get("capabilities")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| anyhow!("WT100 code.capabilities is required"))?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| anyhow!("WT100 capability names must be strings"))
        })
        .collect::<Result<HashSet<_>>>()?;
    for capability in &capabilities {
        if !matches!(
            capability.as_str(),
            "text.v1" | "path.v1" | "ast.v1" | "repo.v1"
        ) {
            bail!("WT104 unknown capability {capability}");
        }
    }
    let patterns = parse_patterns(object.get("patterns"))?;
    let diagnostics = parse_diagnostics(object.get("diagnostics"))?;
    let mut engine = Engine::new();
    engine.set_optimization_level(OptimizationLevel::None);
    engine.set_max_operations(MAX_STEPS);
    engine.set_max_variables(256);
    let ast = engine
        .compile(source)
        .map_err(|error| anyhow!("WT100 syntax error: {error}"))?;
    let mut ast_nodes = 0usize;
    ast.walk(&mut |_| {
        ast_nodes = ast_nodes.saturating_add(1);
        ast_nodes <= MAX_FRONTEND_NODES
    });
    if ast_nodes > MAX_FRONTEND_NODES {
        bail!("WT102 detector AST exceeds the frontend watchdog budget");
    }
    let mut host_scope = Scope::default();
    let (host_name, host_ty) = match execution {
        Execution::File => ("file", Ty::File),
        Execution::Repository => ("repo", Ty::Repo),
    };
    host_scope.bindings.insert(
        host_name.to_owned(),
        Binding {
            mutable: false,
            ty: host_ty,
            constant: None,
        },
    );
    reject_unsupported_literal_syntax(source)?;
    let mut validator = Validator {
        patterns: &patterns,
        diagnostics: &diagnostics,
        capabilities: &capabilities,
        execution,
        scopes: vec![host_scope],
        globs: HashSet::new(),
        steps: 0,
        ast_patterns: BTreeMap::new(),
    };
    validator.validate_ast(&ast)?;
    let mut path_globs = HashMap::new();
    for glob in validator.globs {
        let matcher = GlobBuilder::new(&glob)
            .literal_separator(true)
            .build()
            .map_err(|error| anyhow!("WT100 invalid path glob {glob:?}: {error}"))?
            .compile_matcher();
        path_globs.insert(glob, matcher);
    }
    let ast_patterns = validator.ast_patterns;
    let (rule_ir, plan) = make_plan(&ast, &patterns, execution);
    Ok(Program {
        ast,
        patterns,
        diagnostics,
        execution,
        path_globs,
        ast_patterns,
        plan,
        rule_ir,
        compiled_residual_bytes: manifest_bytes
            .len()
            .saturating_add(source.len().saturating_mul(COMPILED_SOURCE_MULTIPLIER))
            .saturating_add(COMPILED_RESIDUAL_OVERHEAD),
    })
}

impl Program {
    pub fn plan(&self) -> serde_json::Value {
        self.plan.clone()
    }

    pub fn rule_ir(&self) -> &RuleIr {
        &self.rule_ir
    }

    /// Return the conservative memory reservation for this compiled program.
    /// Pattern reservations are keyed by expanded semantics, not manifest alias.
    pub fn compiled_reservation(&self) -> (usize, Vec<(String, usize)>) {
        let mut keys = self
            .patterns
            .values()
            .map(|pattern| pattern.canonical_key.clone())
            .collect::<BTreeSet<_>>();
        keys.extend(
            self.ast_patterns
                .keys()
                .map(|(language, pattern)| format!("ast.v1:{}:{pattern}", language.name())),
        );
        (
            self.compiled_residual_bytes,
            keys.into_iter()
                .map(|key| (key, COMPILED_PATTERN_BYTES))
                .collect(),
        )
    }

    pub fn execute(
        &self,
        files: &[SourceFile],
        arena: &mut QueryArena,
    ) -> Result<Vec<RawDiagnostic>> {
        self.execute_with_limits(files, arena, RuntimeLimits::default())
    }

    pub fn execute_with_limits(
        &self,
        files: &[SourceFile],
        arena: &mut QueryArena,
        limits: RuntimeLimits,
    ) -> Result<Vec<RawDiagnostic>> {
        if files.is_empty() {
            bail!("no authorized source files supplied");
        }
        if limits.max_file_bytes == 0 || limits.max_file_bytes > MAX_FILE_CEILING {
            bail!("configured file limit exceeds the runtime ceiling");
        }
        for (name, value, ceiling) in [
            ("file_steps", limits.file_steps, MAX_STEPS),
            (
                "file_native_bytes",
                limits.file_native_bytes,
                MAX_NATIVE_BYTES,
            ),
            (
                "repository_steps",
                limits.repository_steps,
                MAX_REPOSITORY_STEPS,
            ),
            (
                "repository_native_bytes",
                limits.repository_native_bytes,
                MAX_REPOSITORY_NATIVE_BYTES,
            ),
        ] {
            if value == 0 || value > ceiling {
                bail!("configured {name} exceeds the runtime ceiling or is zero");
            }
        }
        for file in files {
            if file.path.is_empty()
                || file.path.starts_with('/')
                || file.path.contains('\\')
                || file
                    .path
                    .split('/')
                    .any(|part| part.is_empty() || part == "..")
            {
                bail!("unauthorized source path {:?}", file.path);
            }
            if file.text.len() > limits.max_file_bytes {
                bail!(
                    "source file {:?} exceeds configured limit of {} bytes",
                    file.path,
                    limits.max_file_bytes
                );
            }
        }
        if self.execution == Execution::Repository {
            if files.len() > MAX_REPOSITORY_FILES {
                bail!("repository input file limit exceeded");
            }
            let snapshot_bytes = files.iter().map(|file| file.text.len()).sum::<usize>();
            if snapshot_bytes > MAX_REPOSITORY_SNAPSHOT_BYTES {
                bail!("repository snapshot memory limit exceeded");
            }
        }
        let (max_steps, max_native_bytes) = match self.execution {
            Execution::File => (limits.file_steps, limits.file_native_bytes),
            Execution::Repository => (limits.repository_steps, limits.repository_native_bytes),
        };
        let mut ctx = EvalContext {
            program: self,
            files,
            arena,
            diagnostics: Vec::new(),
            steps: 0,
            iterations: 0,
            loop_depth: 0,
            native_bytes: 0,
            temporary_bytes: 0,
            max_steps,
            max_native_bytes,
            current_file: None,
        };
        let result = match self.execution {
            Execution::File => {
                if files.len() != 1 {
                    bail!("file execution requires exactly one source file");
                }
                let mut env = Env::default();
                env.define("file", Value::File(FileValue { index: 0 }), false);
                ctx.exec_block(self.ast.statements(), &mut env).map(|_| ())
            }
            Execution::Repository => {
                let mut env = Env::default();
                env.define("repo", Value::Repo, false);
                ctx.exec_block(self.ast.statements(), &mut env).map(|_| ())
            }
        };
        let stats = (
            ctx.steps,
            ctx.native_bytes,
            ctx.temporary_bytes,
            ctx.iterations,
        );
        let diagnostics = ctx.diagnostics;
        arena.record_execution(stats.0, stats.1, stats.2, stats.3);
        result.map(|_| diagnostics)
    }
}

fn parse_patterns(value: Option<&JsonValue>) -> Result<BTreeMap<String, Pattern>> {
    let mut patterns = BTreeMap::new();
    let Some(value) = value else {
        return Ok(patterns);
    };
    let object = value
        .as_object()
        .ok_or_else(|| anyhow!("WT100 patterns must be an object"))?;
    if object.len() > 128 {
        bail!("WT102 more than 128 patterns are not supported");
    }
    for (name, definition) in object {
        if !is_identifier(name) {
            bail!("WT100 invalid pattern ID {name:?}");
        }
        let (expression, flags) = if let Some(expression) = definition.as_str() {
            (expression.to_owned(), PatternFlags::default())
        } else {
            let definition = definition
                .as_object()
                .ok_or_else(|| anyhow!("WT100 pattern {name} must be a string or object"))?;
            let expression = definition
                .get("regex")
                .and_then(JsonValue::as_str)
                .ok_or_else(|| anyhow!("WT100 pattern {name}.regex is required"))?
                .to_owned();
            let flags = parse_flags(definition.get("flags"))?;
            (expression, flags)
        };
        if expression.len() > 8 * 1024 {
            bail!("WT102 pattern {name} exceeds 8 KiB");
        }
        let (regex, canonical_key) = intern_regex(&expression, flags)
            .map_err(|error| anyhow!("WT100 invalid regex {name}: {error}"))?;
        let captures = regex.capture_names().flatten().map(str::to_owned).collect();
        patterns.insert(
            name.clone(),
            Pattern {
                regex,
                expression,
                flags,
                captures,
                canonical_key,
            },
        );
    }
    Ok(patterns)
}

fn canonical_pattern_key(expression: &str, flags: PatternFlags) -> String {
    serde_json::to_string(&json!({
        "version": 1,
        "engine": REGEX_ENGINE,
        "expression": expression,
        "flags": {
            "case_insensitive": flags.case_insensitive,
            "multi_line": flags.multi_line,
            "dot_matches_new_line": flags.dot_matches_new_line,
            "ignore_whitespace": flags.ignore_whitespace,
            "crlf": flags.crlf,
        },
    }))
    .expect("canonical pattern key is serializable")
}

fn intern_regex(expression: &str, flags: PatternFlags) -> Result<(Arc<Regex>, String)> {
    let key = canonical_pattern_key(expression, flags);
    COMPILED_PATTERNS.with(|registry| {
        let mut registry = registry.borrow_mut();
        registry.clean_dead();
        if let Some(regex) = registry.entries.get(&key).and_then(Weak::upgrade) {
            return Ok((regex, key));
        }

        let mut builder = RegexBuilder::new(expression);
        builder
            .case_insensitive(flags.case_insensitive)
            .multi_line(flags.multi_line)
            .dot_matches_new_line(flags.dot_matches_new_line)
            .ignore_whitespace(flags.ignore_whitespace)
            .crlf(flags.crlf)
            .size_limit(MAX_REGEX_SIZE);
        let regex = Arc::new(builder.build()?);
        if key.len() <= PATTERN_REGISTRY_KEY_BYTES
            && registry.key_bytes.saturating_add(key.len()) <= PATTERN_REGISTRY_KEY_BYTES
        {
            registry.key_bytes = registry.key_bytes.saturating_add(key.len());
            registry.entries.insert(key.clone(), Arc::downgrade(&regex));
        }
        Ok((regex, key))
    })
}

fn parse_flags(value: Option<&JsonValue>) -> Result<PatternFlags> {
    let Some(value) = value else {
        return Ok(PatternFlags::default());
    };
    let object = value
        .as_object()
        .ok_or_else(|| anyhow!("WT100 pattern flags must be an object"))?;
    let mut flags = PatternFlags::default();
    for (name, value) in object {
        let enabled = value
            .as_bool()
            .ok_or_else(|| anyhow!("WT100 regex flag {name} must be boolean"))?;
        match name.as_str() {
            "case_insensitive" => flags.case_insensitive = enabled,
            "multi_line" => flags.multi_line = enabled,
            "dot_matches_new_line" => flags.dot_matches_new_line = enabled,
            "ignore_whitespace" => flags.ignore_whitespace = enabled,
            "crlf" => flags.crlf = enabled,
            _ => bail!("WT100 unknown regex flag {name}"),
        }
    }
    Ok(flags)
}

fn parse_diagnostics(value: Option<&JsonValue>) -> Result<BTreeMap<String, DiagnosticDef>> {
    let value = value.ok_or_else(|| anyhow!("WT100 diagnostics is required"))?;
    let object = value
        .as_object()
        .ok_or_else(|| anyhow!("WT100 diagnostics must be an object"))?;
    if object.is_empty() {
        bail!("WT100 diagnostics must not be empty");
    }
    let mut diagnostics = BTreeMap::new();
    for (code, definition) in object {
        if !is_identifier(code) && !code.contains('-') {
            bail!("WT100 invalid diagnostic ID {code:?}");
        }
        let definition = definition
            .as_object()
            .ok_or_else(|| anyhow!("WT100 diagnostic {code} must be an object"))?;
        let kind = definition.get("kind").and_then(JsonValue::as_str);
        if !matches!(kind, Some("violation") | Some("review")) {
            bail!("WT100 diagnostic {code} has invalid kind");
        }
        diagnostics.insert(code.clone(), DiagnosticDef);
    }
    Ok(diagnostics)
}

fn require_capability(capabilities: &HashSet<String>, capability: &str, what: &str) -> Result<()> {
    if capabilities.contains(capability) {
        Ok(())
    } else {
        bail!("WT104 {what} require capability {capability}")
    }
}

fn is_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) if first == '_' || first.is_ascii_alphabetic() => {
            chars.all(|character| character == '_' || character.is_ascii_alphanumeric())
        }
        _ => false,
    }
}

type ScopeKey = Vec<(Option<usize>, Option<usize>)>;
type ScopedConstants = HashMap<ScopeKey, BTreeMap<String, String>>;

fn make_plan(
    ast: &AST,
    patterns: &BTreeMap<String, Pattern>,
    execution: Execution,
) -> (RuleIr, JsonValue) {
    let mut calls = Vec::new();
    let mut queries = Vec::new();
    let mut scoped_constants = ScopedConstants::new();
    let mut call_site = 0usize;
    ast.walk(&mut |path: &[ASTNode<'_>]| {
        let scope = scope_key(path);
        let Some(node) = path.last() else {
            return true;
        };
        if let ASTNode::Stmt(Stmt::Var(data, flags, _)) = node {
            if flags.contains(ASTFlags::CONSTANT) {
                let (ident, expression, _) = &**data;
                let inherited = constants_for_scope(&scope, &scoped_constants);
                if let Some(value) = symbolic_string_constant(expression, &inherited) {
                    scoped_constants
                        .entry(scope.clone())
                        .or_default()
                        .insert(ident.name.to_string(), value);
                }
            }
        }
        let call = match node {
            ASTNode::Expr(Expr::FnCall(call, _)) | ASTNode::Expr(Expr::MethodCall(call, _)) => call,
            _ => return true,
        };
        if matches!(node, ASTNode::Expr(Expr::FnCall(..))) {
            let namespace = namespace_name(call);
            calls.push(json!({"name": call.name.to_string(), "namespace": namespace}));
        }
        let constants = constants_for_scope(&scope, &scoped_constants);
        if let Some(template) = query_template(path, call, patterns, &constants, call_site) {
            queries.push(template);
            call_site += 1;
        }
        true
    });
    let rule_ir = RuleIr {
        execution: match execution {
            Execution::File => "file".into(),
            Execution::Repository => "repository".into(),
        },
        queries,
    };
    let plan = json!({
        "language": "wt-rule-1",
        "execution": match execution { Execution::File => "file", Execution::Repository => "repository" },
        "patterns": patterns.iter().map(|(name, pattern)| (name.clone(), json!({
            "expression": pattern.expression,
            "flags": {
                "case_insensitive": pattern.flags.case_insensitive,
                "multi_line": pattern.flags.multi_line,
                "dot_matches_new_line": pattern.flags.dot_matches_new_line,
                "ignore_whitespace": pattern.flags.ignore_whitespace,
                "crlf": pattern.flags.crlf,
            }
        }))).collect::<BTreeMap<_, _>>(),
        "calls": calls,
        "ir": rule_ir,
        "frontend": "validated-rhai-ast-adapter",
        "residual_executor": "restricted-interpreter",
        "sharing": "canonical-query-arena",
    });
    (rule_ir, plan)
}

fn scope_key(path: &[ASTNode<'_>]) -> ScopeKey {
    path.iter()
        .filter_map(|node| match node {
            ASTNode::Stmt(Stmt::If(_, position)) => Some((position.line(), position.position())),
            ASTNode::Stmt(Stmt::For(_, position)) => Some((position.line(), position.position())),
            _ => None,
        })
        .collect()
}

fn constants_for_scope(
    scope: &[(Option<usize>, Option<usize>)],
    scoped_constants: &ScopedConstants,
) -> BTreeMap<String, String> {
    let mut constants = BTreeMap::new();
    for depth in 0..=scope.len() {
        if let Some(values) = scoped_constants.get(&scope[..depth]) {
            constants.extend(values.clone());
        }
    }
    constants
}

fn query_template(
    path: &[ASTNode<'_>],
    call: &FnCallExpr,
    patterns: &BTreeMap<String, Pattern>,
    constants: &BTreeMap<String, String>,
    call_site: usize,
) -> Option<QueryTemplate> {
    let method_receiver = method_receiver(path, call);
    let namespace = namespace_name(call);
    let (namespace, operation, result_shape, input) = if !namespace.is_empty() {
        let operation = match (namespace.as_str(), call.name.as_str()) {
            ("text", name) => format!("text.{name}"),
            ("path", "matches") => "path.matches".into(),
            ("rx", "is_match" | "find_all" | "find_in" | "capture_text") => {
                format!("regex.{}", call.name)
            }
            ("repo", "files") => "repo.files".into(),
            _ => return None,
        };
        let shape = result_shape(&operation);
        let input = symbolic_call_input(call, constants);
        (namespace, operation, shape, input)
    } else if let Some(receiver) = method_receiver {
        let operation = match call.name.as_str() {
            "contains" | "starts_with" | "ends_with" | "trim" | "split" | "replace"
            | "len_bytes" => format!("text.{}", call.name),
            "ast_match" => "ast.match".into(),
            "node" => "ast.node".into(),
            "group_text" => "regex.group_text".into(),
            "is_empty" => "sequence.is_empty".into(),
            _ => return None,
        };
        let shape = result_shape(&operation);
        let input = format!(
            "{}.{}({})",
            symbolic_expr(receiver, constants),
            call.name,
            call.args
                .iter()
                .map(|argument| symbolic_expr(argument, constants))
                .collect::<Vec<_>>()
                .join(", ")
        );
        (String::new(), operation, shape, input)
    } else {
        return None;
    };
    let source = source_location(call_position(call));
    let guards = path
        .iter()
        .filter_map(|node| match node {
            ASTNode::Stmt(Stmt::If(flow, _)) => Some(GuardContext {
                kind: "if".into(),
                source: source_location(flow.expr.position()),
            }),
            ASTNode::Stmt(Stmt::For(_, position)) => Some(GuardContext {
                kind: "for".into(),
                source: source_location(*position),
            }),
            _ => None,
        })
        .collect();
    let pattern_index = match operation.as_str() {
        "regex.capture_text" => Some(0),
        "regex.is_match" | "regex.find_all" | "regex.find_in" => Some(1),
        _ => None,
    };
    let pattern_alias = pattern_index
        .and_then(|index| call.args.get(index))
        .and_then(|argument| symbolic_string_constant(argument, constants));
    let pattern = pattern_alias.as_ref().and_then(|alias| patterns.get(alias));
    let pattern_expression = pattern.map(|pattern| pattern.expression.clone());
    let pattern_flags = pattern.map(|pattern| PatternSemantics {
        case_insensitive: pattern.flags.case_insensitive,
        multi_line: pattern.flags.multi_line,
        dot_matches_new_line: pattern.flags.dot_matches_new_line,
        ignore_whitespace: pattern.flags.ignore_whitespace,
        crlf: pattern.flags.crlf,
        engine: REGEX_ENGINE.into(),
    });
    let identity_input = if operation.starts_with("regex.") {
        match operation.as_str() {
            "regex.capture_text" => call.args.get(1),
            "regex.is_match" | "regex.find_all" | "regex.find_in" => call.args.first(),
            _ => None,
        }
        .map(|argument| symbolic_expr(argument, constants))
        .unwrap_or_default()
    } else {
        input.clone()
    };
    let canonical_identity = serde_json::to_string(&json!({
        "version": 1,
        "operation": operation,
        "result_shape": result_shape,
        "pattern": pattern_expression,
        "pattern_flags": pattern_flags,
        "input": identity_input,
    }))
    .expect("canonical query identity is serializable");
    Some(QueryTemplate {
        call_site,
        source,
        guards,
        namespace,
        operation,
        result_shape,
        input_symbolic_expression: input,
        pattern_alias,
        pattern_expression,
        pattern_flags,
        canonical_identity,
        resolved_string_constants: constants.clone(),
    })
}

fn result_shape(operation: &str) -> String {
    match operation {
        "regex.is_match" | "path.matches" | "text.contains" | "text.starts_with"
        | "text.ends_with" | "sequence.is_empty" => "bool".into(),
        "regex.find_all" | "regex.find_in" | "text.lines" | "text.split" | "ast.match"
        | "repo.files" => "sequence".into(),
        "regex.capture_text" | "ast.node" => "optional<match>".into(),
        "regex.group_text" => "optional<text>".into(),
        "text.len_bytes" => "int".into(),
        _ => "text".into(),
    }
}

fn source_location(position: Position) -> SourceLocation {
    SourceLocation {
        line: position.line(),
        column: position.position(),
    }
}

fn method_receiver<'a>(path: &'a [ASTNode<'a>], call: &FnCallExpr) -> Option<&'a Expr> {
    path.iter().rev().find_map(|node| match node {
        ASTNode::Expr(Expr::Dot(data, _, _)) => match &data.rhs {
            Expr::MethodCall(inner, _) if std::ptr::eq(inner.as_ref(), call) => Some(&data.lhs),
            _ => None,
        },
        _ => None,
    })
}

fn symbolic_call_input(call: &FnCallExpr, constants: &BTreeMap<String, String>) -> String {
    call.args
        .iter()
        .map(|argument| symbolic_expr(argument, constants))
        .collect::<Vec<_>>()
        .join(", ")
}

fn symbolic_string_constant(
    expression: &Expr,
    constants: &BTreeMap<String, String>,
) -> Option<String> {
    match expression {
        Expr::StringConstant(value, _) => Some(value.to_string()),
        Expr::Variable(data, ..) => constants.get(data.1.as_str()).cloned(),
        _ => None,
    }
}

fn symbolic_expr(expression: &Expr, constants: &BTreeMap<String, String>) -> String {
    if let Some(value) = symbolic_string_constant(expression, constants) {
        return format!("string:{value:?}");
    }
    match expression {
        Expr::Variable(data, ..) => data.1.to_string(),
        Expr::IntegerConstant(value, _) => value.to_string(),
        Expr::BoolConstant(value, _) => value.to_string(),
        Expr::Unit(_) => "()".into(),
        Expr::Property(data, _) => data.2.to_string(),
        Expr::Dot(data, _, _) => format!(
            "{}.{}",
            symbolic_expr(&data.lhs, constants),
            symbolic_expr(&data.rhs, constants)
        ),
        Expr::FnCall(call, _) | Expr::MethodCall(call, _) => {
            format!("{}({})", call.name, symbolic_call_input(call, constants))
        }
        _ => format!(
            "expr@{}",
            source_location(expression.position()).line.unwrap_or(0)
        ),
    }
}

fn namespace_name(call: &FnCallExpr) -> String {
    call.namespace
        .path
        .iter()
        .map(|part| part.name.as_str())
        .collect::<Vec<_>>()
        .join("::")
}

#[derive(Default)]
struct Scope {
    bindings: HashMap<String, Binding>,
}

#[derive(Clone)]
struct Binding {
    mutable: bool,
    ty: Ty,
    constant: Option<String>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
enum Ty {
    Unit,
    Bool,
    Int,
    Text,
    File,
    Span,
    Match,
    Line,
    Repo,
    Sequence(Box<Ty>),
    Optional(Box<Ty>),
}

impl Ty {
    fn optional(self) -> Self {
        Self::Optional(Box::new(self))
    }
    fn unwrap_optional(&self) -> &Ty {
        match self {
            Ty::Optional(inner) => inner,
            other => other,
        }
    }
}

struct Validator<'a> {
    patterns: &'a BTreeMap<String, Pattern>,
    diagnostics: &'a BTreeMap<String, DiagnosticDef>,
    capabilities: &'a HashSet<String>,
    execution: Execution,
    scopes: Vec<Scope>,
    globs: HashSet<String>,
    steps: usize,
    /// `ast.v1` patterns are static-literal arguments compiled eagerly here
    /// (once per distinct language+pattern), so a malformed pattern is a
    /// WT100 rule-compile error, never a runtime surprise.
    ast_patterns: BTreeMap<(AstLanguage, String), ast_grep_core::Pattern>,
}

impl Validator<'_> {
    fn error(
        &self,
        code: &str,
        message: impl std::fmt::Display,
        position: Position,
    ) -> anyhow::Error {
        let location = match (position.line(), position.position()) {
            (Some(line), Some(column)) => format!(" at {line}:{column}"),
            (Some(line), None) => format!(" at {line}:1"),
            _ => String::new(),
        };
        anyhow!("{code}{location}: {message}")
    }

    fn validate_ast(&mut self, ast: &AST) -> Result<()> {
        if ast.iter_functions().next().is_some() {
            bail!("WT102 user-defined functions are not part of wt-rule-1");
        }
        self.validate_block(ast.statements(), 0)
    }

    fn validate_block(&mut self, block: &[Stmt], loop_depth: usize) -> Result<()> {
        for stmt in block {
            self.steps += 1;
            if self.steps > MAX_STEPS as usize {
                bail!("WT102 detector AST exceeds the validation budget");
            }
            self.validate_stmt(stmt, loop_depth)?;
        }
        Ok(())
    }

    fn validate_stmt(&mut self, stmt: &Stmt, loop_depth: usize) -> Result<()> {
        match stmt {
            Stmt::Noop(_) => Ok(()),
            Stmt::Var(data, flags, pos) => {
                let (ident, expression, _) = &**data;
                self.check_binding_name(ident.name.as_str(), *pos)?;
                if flags.contains(ASTFlags::CONSTANT) && !is_literal(expression) {
                    return Err(self.error(
                        "WT102",
                        "const requires a literal",
                        expression.position(),
                    ));
                }
                let ty = self.validate_expr(expression)?;
                let constant = if flags.contains(ASTFlags::CONSTANT) {
                    match expression {
                        Expr::StringConstant(value, _) => Some(value.to_string()),
                        _ => None,
                    }
                } else {
                    None
                };
                self.check_binding_budget()?;
                self.current_scope_mut().bindings.insert(
                    ident.name.to_string(),
                    Binding {
                        mutable: !flags.contains(ASTFlags::CONSTANT),
                        ty,
                        constant,
                    },
                );
                Ok(())
            }
            Stmt::Assignment(data) => {
                let BinaryExpr { lhs, rhs } = &data.1;
                if data.0.is_op_assignment() {
                    return Err(self.error(
                        "WT105",
                        "compound assignment is forbidden",
                        data.0.position(),
                    ));
                }
                let name = variable_name(lhs).ok_or_else(|| {
                    self.error(
                        "WT105",
                        "only scalar local reassignment is allowed",
                        lhs.position(),
                    )
                })?;
                match self.lookup(name) {
                    Some(binding) if binding.mutable => {}
                    Some(_) => {
                        return Err(self.error("WT105", "binding is immutable", lhs.position()))
                    }
                    None => {
                        return Err(self.error(
                            "WT105",
                            "assignment requires an existing local",
                            lhs.position(),
                        ))
                    }
                }
                self.validate_expr(rhs).map(|_| ())
            }
            Stmt::If(flow, _) => {
                if self.validate_expr(&flow.expr)? != Ty::Bool {
                    return Err(self.error(
                        "WT105",
                        "if condition must be boolean",
                        flow.expr.position(),
                    ));
                }
                self.push_scope()?;
                self.validate_block(flow.body.statements(), loop_depth)?;
                self.scopes.pop();
                self.push_scope()?;
                self.validate_block(flow.branch.statements(), loop_depth)?;
                self.scopes.pop();
                Ok(())
            }
            Stmt::For(data, pos) => {
                if data.1.is_some() {
                    return Err(self.error(
                        "WT105",
                        "for loop counters are not part of wt-rule-1",
                        *pos,
                    ));
                }
                self.push_scope()?;
                self.check_binding_name(data.0.name.as_str(), *pos)?;
                let sequence_ty = self.validate_expr(&data.2.expr)?;
                let element_ty = match sequence_ty {
                    Ty::Sequence(inner) => *inner,
                    _ => {
                        return Err(self.error(
                            "WT105",
                            "for requires a finite WT sequence",
                            data.2.expr.position(),
                        ))
                    }
                };
                self.check_binding_budget()?;
                self.current_scope_mut().bindings.insert(
                    data.0.name.to_string(),
                    Binding {
                        mutable: false,
                        ty: element_ty,
                        constant: None,
                    },
                );
                self.validate_block(data.2.body.statements(), loop_depth + 1)?;
                self.scopes.pop();
                Ok(())
            }
            Stmt::BreakLoop(expression, _, pos) => {
                if expression.is_some() {
                    return Err(self.error("WT105", "break and continue take no value", *pos));
                }
                if loop_depth == 0 {
                    return Err(self.error("WT105", "break and continue require a loop", *pos));
                }
                Ok(())
            }
            Stmt::Return(expression, flags, pos) => {
                if flags.contains(ASTFlags::BREAK) || expression.is_some() {
                    return Err(self.error("WT102", "throw and valued return are forbidden", *pos));
                }
                Ok(())
            }
            Stmt::FnCall(call, _) => self.validate_call(call).map(|_| ()),
            Stmt::Expr(expression) => self.validate_expr(expression).map(|_| ()),
            Stmt::Block(block) => Err(self.error(
                "WT102",
                "standalone blocks are not part of wt-rule-1",
                block.position(),
            )),
            Stmt::While(_, pos) | Stmt::Do(_, _, pos) => Err(self.error(
                "WT102",
                "while, do, and loop are not part of wt-rule-1",
                *pos,
            )),
            Stmt::Switch(_, pos) => {
                Err(self.error("WT102", "switch is not part of wt-rule-1", *pos))
            }
            Stmt::TryCatch(_, pos) => {
                Err(self.error("WT102", "try/catch is not part of wt-rule-1", *pos))
            }
            Stmt::Import(_, pos) | Stmt::Export(_, pos) => {
                Err(self.error("WT102", "modules are not part of wt-rule-1", *pos))
            }
            Stmt::Share(_) => Err(anyhow!("WT102 closures are not part of wt-rule-1")),
            _ => Err(anyhow!(
                "WT102 unknown Rhai syntax node is not part of wt-rule-1"
            )),
        }
    }

    fn validate_expr(&mut self, expression: &Expr) -> Result<Ty> {
        match expression {
            Expr::BoolConstant(..) => Ok(Ty::Bool),
            Expr::IntegerConstant(..) => Ok(Ty::Int),
            Expr::StringConstant(..) => Ok(Ty::Text),
            Expr::Unit(_) => Ok(Ty::Unit),
            Expr::Variable(data, _, pos) => self
                .lookup(data.1.as_str())
                .map(|binding| binding.ty)
                .ok_or_else(|| self.error("WT105", format!("unknown binding {:?}", data.1), *pos)),
            Expr::DynamicConstant(_, pos) => {
                Err(self.error("WT102", "dynamic values are not part of wt-rule-1", *pos))
            }
            Expr::FloatConstant(_, pos) => Err(self.error(
                "WT102",
                "floating point values are not part of wt-rule-1",
                *pos,
            )),
            Expr::CharConstant(_, pos) => Err(self.error(
                "WT102",
                "character literals are not part of wt-rule-1",
                *pos,
            )),
            Expr::InterpolatedString(_, pos) => Err(self.error(
                "WT102",
                "interpolated strings are not part of wt-rule-1",
                *pos,
            )),
            Expr::Array(_, pos) | Expr::Map(_, pos) => Err(self.error(
                "WT102",
                "collection literals are not part of wt-rule-1",
                *pos,
            )),
            Expr::ThisPtr(pos) | Expr::Index(_, _, pos) | Expr::Coalesce(_, pos) => Err(self
                .error(
                    "WT102",
                    "this/indexing/coalescing are not part of wt-rule-1",
                    *pos,
                )),
            Expr::And(expressions, _) | Expr::Or(expressions, _) => {
                for expression in expressions.iter() {
                    let ty = self.validate_expr(expression)?;
                    if ty != Ty::Bool {
                        return Err(self.error(
                            "WT105",
                            "logical operands must be boolean",
                            expression.position(),
                        ));
                    }
                }
                Ok(Ty::Bool)
            }
            Expr::Dot(data, flags, pos) => {
                if !flags.is_empty() {
                    return Err(self.error(
                        "WT102",
                        "optional chaining is not part of wt-rule-1",
                        *pos,
                    ));
                }
                let receiver = self.validate_expr(&data.lhs)?;
                self.validate_dot_chain(receiver, &data.rhs)
            }
            Expr::Property(_, pos) => Err(self.error(
                "WT102",
                "standalone properties are not part of wt-rule-1",
                *pos,
            )),
            Expr::Stmt(block) => Err(self.error(
                "WT102",
                "expression-valued blocks are not part of wt-rule-1",
                block.position(),
            )),
            Expr::MethodCall(call, _) | Expr::FnCall(call, _) => self.validate_call(call),
            Expr::Custom(_, pos) => {
                Err(self.error("WT102", "custom syntax is not part of wt-rule-1", *pos))
            }
            _ => Err(anyhow!(
                "WT102 unknown Rhai expression node is not part of wt-rule-1"
            )),
        }
    }

    fn validate_dot_chain(&mut self, receiver: Ty, expression: &Expr) -> Result<Ty> {
        match expression {
            Expr::Property(property, _) => {
                self.validate_property(&receiver, property.2.as_str(), expression.position())
            }
            Expr::FnCall(call, _) | Expr::MethodCall(call, _) => {
                self.validate_method(&receiver, call)
            }
            Expr::Dot(data, _, _) => {
                let middle = self.validate_dot_chain(receiver, &data.lhs)?;
                self.validate_dot_chain(middle, &data.rhs)
            }
            _ => Err(self.error(
                "WT102",
                "computed property access is not part of wt-rule-1",
                expression.position(),
            )),
        }
    }

    fn validate_property(&self, receiver: &Ty, property: &str, pos: Position) -> Result<Ty> {
        let receiver = receiver.unwrap_optional();
        match receiver {
            Ty::File if property == "path" => {
                self.require_any_capability(&["text.v1", "path.v1"], "path APIs")?
            }
            Ty::File if matches!(property, "text" | "span") => {
                self.require_any_capability(&["text.v1", "ast.v1"], "file text/span APIs")?
            }
            Ty::Match => self.require_any_capability(&["text.v1", "ast.v1"], "match APIs")?,
            Ty::Line => self.require_text_surface()?,
            Ty::Sequence(_) => self.require_any_capability(
                &["text.v1", "path.v1", "repo.v1", "ast.v1"],
                "sequence APIs",
            )?,
            _ => {}
        }
        let ty = match (receiver, property) {
            (Ty::File, "path") => Ty::Text,
            (Ty::File, "text") => Ty::Text,
            (Ty::File, "span") => Ty::Span,
            (Ty::Match, "text") => Ty::Text,
            (Ty::Match, "span") => Ty::Span,
            (Ty::Sequence(_), "len") => Ty::Int,
            (Ty::Sequence(_), "is_empty") => Ty::Bool,
            (Ty::Line, "text") => Ty::Text,
            (Ty::Line, "span") => Ty::Span,
            _ => {
                return Err(self.error(
                    "WT104",
                    format!("property {property:?} is not available on {receiver:?}"),
                    pos,
                ))
            }
        };
        Ok(ty)
    }

    fn validate_method(&mut self, receiver: &Ty, call: &FnCallExpr) -> Result<Ty> {
        let receiver = receiver.unwrap_optional().clone();
        let name = call.name.as_str();
        if matches!(
            (&receiver, name),
            (Ty::File, "ast_match") | (Ty::Match, "ast_match")
        ) {
            let args = self.validate_arguments(call)?;
            if args.len() != 2 || args[0] != Ty::Text || args[1] != Ty::Text {
                return Err(self.error(
                    "WT104",
                    "ast_match requires (language, pattern) string literals",
                    call_position(call),
                ));
            }
            self.validate_ast_match_call(call, 0, 1)?;
            return Ok(Ty::Sequence(Box::new(Ty::Match)));
        }
        if matches!((&receiver, name), (Ty::Match, "node")) {
            let args = self.validate_arguments(call)?;
            if args.len() != 1 || args[0] != Ty::Text {
                return Err(self.error(
                    "WT104",
                    "node requires one text argument",
                    call_position(call),
                ));
            }
            self.require_capability("ast.v1", "ast.v1 node capture APIs")?;
            self.require_static_text(call, 0)?;
            return Ok(Ty::Match.optional());
        }
        let args = self.validate_arguments(call)?;
        match &receiver {
            Ty::Text => self.require_text_surface()?,
            Ty::Match => self.require_any_capability(&["text.v1", "ast.v1"], "match APIs")?,
            Ty::Repo => self.require_any_capability(&["text.v1", "repo.v1"], "repository APIs")?,
            Ty::Sequence(_) => self.require_any_capability(
                &["text.v1", "path.v1", "repo.v1", "ast.v1"],
                "sequence APIs",
            )?,
            _ => {}
        }
        match (receiver, name) {
            (Ty::Text, "contains" | "starts_with" | "ends_with")
                if args.len() == 1 && args[0] == Ty::Text =>
            {
                Ok(Ty::Bool)
            }
            (Ty::Text, "trim") if args.is_empty() => Ok(Ty::Text),
            (Ty::Text, "len_bytes") if args.is_empty() => Ok(Ty::Int),
            (Ty::Text, "split") if args.len() == 1 && args[0] == Ty::Text => {
                Ok(Ty::Sequence(Box::new(Ty::Text)))
            }
            (Ty::Text, "replace")
                if args.len() == 2 && args[0] == Ty::Text && args[1] == Ty::Text =>
            {
                Ok(Ty::Text)
            }
            (Ty::Sequence(_), "is_empty") if args.is_empty() => Ok(Ty::Bool),
            (Ty::Match, "group_text") if args.len() == 1 && args[0] == Ty::Text => {
                let name = self.require_static_text(call, 0)?;
                if !self
                    .patterns
                    .values()
                    .any(|pattern| pattern.captures.contains(&name))
                {
                    bail!("WT106 unknown capture name {name:?}")
                }
                Ok(Ty::Text.optional())
            }
            (Ty::Repo, "files") if args.is_empty() && self.execution == Execution::Repository => {
                Ok(Ty::Sequence(Box::new(Ty::File)))
            }
            _ => Err(self.error(
                "WT104",
                format!("method {name:?} has an invalid receiver, arity, or operand type"),
                call_position(call),
            )),
        }
    }

    /// Validate and eagerly compile a static (language, pattern) ast.v1
    /// argument pair, caching the compiled pattern by canonical key.
    fn validate_ast_match_call(
        &mut self,
        call: &FnCallExpr,
        language_index: usize,
        pattern_index: usize,
    ) -> Result<()> {
        self.require_capability("ast.v1", "ast_match")?;
        let language_text = self.require_static_text(call, language_index)?;
        let language = AstLanguage::parse(&language_text).ok_or_else(|| {
            self.error(
                "WT104",
                format!(
                    "unsupported ast.v1 language {language_text:?}; this build supports {}",
                    ast_match::supported_languages().join(", ")
                ),
                call.args
                    .get(language_index)
                    .map(Expr::position)
                    .unwrap_or(Position::START),
            )
        })?;
        let pattern_text = self.require_static_text(call, pattern_index)?;
        if !self
            .ast_patterns
            .contains_key(&(language, pattern_text.clone()))
        {
            let compiled = ast_match::compile_pattern(language, &pattern_text)?;
            self.ast_patterns.insert((language, pattern_text), compiled);
        }
        Ok(())
    }

    fn validate_call(&mut self, call: &FnCallExpr) -> Result<Ty> {
        let namespace = namespace_name(call);
        let name = call.name.as_str();
        let args = self.validate_arguments(call)?;
        match (namespace.as_str(), name) {
            ("", "emit") => {
                if args.len() != 2 || args[0] != Ty::Span {
                    return Err(self.error(
                        "WT104",
                        "emit requires (span, diagnostic_id)",
                        call_position(call),
                    ));
                }
                self.require_static_diagnostic(call, 1)?;
                Ok(Ty::Unit)
            }
            ("", "!") => {
                if args.len() == 1 && args[0] == Ty::Bool {
                    Ok(Ty::Bool)
                } else {
                    Err(self.error(
                        "WT105",
                        "! requires one boolean operand",
                        call_position(call),
                    ))
                }
            }
            ("", "+" | "-") if args.len() == 1 => {
                if args[0] == Ty::Int {
                    Ok(Ty::Int)
                } else {
                    Err(self.error(
                        "WT105",
                        "unary arithmetic requires an integer",
                        call_position(call),
                    ))
                }
            }
            ("", "+")
                if args.len() == 2
                    && ((compatible(&args[0], &Ty::Int) && compatible(&args[1], &Ty::Int))
                        || (compatible(&args[0], &Ty::Text)
                            && compatible(&args[1], &Ty::Text))) =>
            {
                Ok(if compatible(&args[0], &Ty::Text) {
                    Ty::Text
                } else {
                    Ty::Int
                })
            }
            ("", "-" | "*" | "/" | "%")
                if args.len() == 2
                    && compatible(&args[0], &Ty::Int)
                    && compatible(&args[1], &Ty::Int) =>
            {
                Ok(Ty::Int)
            }
            ("", "==" | "!=") if args.len() == 2 && equality_compatible(&args[0], &args[1]) => {
                Ok(Ty::Bool)
            }
            ("", "<" | "<=" | ">" | ">=")
                if args.len() == 2
                    && ((compatible(&args[0], &Ty::Int) && compatible(&args[1], &Ty::Int))
                        || (compatible(&args[0], &Ty::Text)
                            && compatible(&args[1], &Ty::Text))) =>
            {
                Ok(Ty::Bool)
            }
            ("text", "contains" | "starts_with" | "ends_with")
                if args.len() == 2
                    && compatible(&args[0], &Ty::Text)
                    && compatible(&args[1], &Ty::Text) =>
            {
                self.require_text_surface()?;
                Ok(Ty::Bool)
            }
            ("text", "trim") if args.len() == 1 && compatible(&args[0], &Ty::Text) => {
                self.require_text_surface()?;
                Ok(Ty::Text)
            }
            ("text", "len_bytes") if args.len() == 1 && compatible(&args[0], &Ty::Text) => {
                self.require_text_surface()?;
                Ok(Ty::Int)
            }
            ("text", "split")
                if args.len() == 2
                    && compatible(&args[0], &Ty::Text)
                    && compatible(&args[1], &Ty::Text) =>
            {
                self.require_text_surface()?;
                Ok(Ty::Sequence(Box::new(Ty::Text)))
            }
            ("text", "replace")
                if args.len() == 3
                    && compatible(&args[0], &Ty::Text)
                    && compatible(&args[1], &Ty::Text)
                    && compatible(&args[2], &Ty::Text) =>
            {
                self.require_text_surface()?;
                Ok(Ty::Text)
            }
            ("text", "lines") if args.len() == 1 && args[0] == Ty::File => {
                self.require_text_surface()?;
                Ok(Ty::Sequence(Box::new(Ty::Line)))
            }
            ("path", "matches") if args.len() == 2 && args[0] == Ty::Text => {
                self.require_any_capability(&["text.v1", "path.v1"], "path APIs")?;
                let glob = self.require_static_text(call, 1)?;
                validate_scope_glob(&glob, call.args[1].position())?;
                self.globs.insert(glob);
                Ok(Ty::Bool)
            }
            ("rx", "is_match") if args.len() == 2 && args[0] == Ty::File => {
                self.require_capability("text.v1", "regex APIs")?;
                self.require_static_pattern(call, 1)?;
                Ok(Ty::Bool)
            }
            ("rx", "find_all") if args.len() == 2 && args[0] == Ty::File => {
                self.require_capability("text.v1", "regex APIs")?;
                self.require_static_pattern(call, 1)?;
                Ok(Ty::Sequence(Box::new(Ty::Match)))
            }
            ("rx", "find_in") if args.len() == 2 && args[0] == Ty::Span => {
                self.require_capability("text.v1", "regex APIs")?;
                self.require_static_pattern(call, 1)?;
                Ok(Ty::Sequence(Box::new(Ty::Match)))
            }
            ("rx", "capture_text") if args.len() == 2 && compatible(&args[1], &Ty::Text) => {
                self.require_capability("text.v1", "regex APIs")?;
                self.require_static_pattern(call, 0)?;
                Ok(Ty::Match.optional())
            }
            _ => Err(self.error(
                "WT104",
                format!("unregistered call {namespace}::{name} or invalid arity/types"),
                call_position(call),
            )),
        }
    }

    fn validate_arguments(&mut self, call: &FnCallExpr) -> Result<Vec<Ty>> {
        call.args
            .iter()
            .map(|argument| self.validate_expr(argument))
            .collect()
    }

    fn require_text_surface(&self) -> Result<()> {
        self.require_capability("text.v1", "text APIs")
    }

    fn require_capability(&self, capability: &str, what: &str) -> Result<()> {
        require_capability(self.capabilities, capability, what)
    }

    fn require_any_capability(&self, capabilities: &[&str], what: &str) -> Result<()> {
        if capabilities
            .iter()
            .any(|capability| self.capabilities.contains(*capability))
        {
            Ok(())
        } else {
            bail!("WT104 {what} require one of {}", capabilities.join(", "))
        }
    }

    fn require_static_text(&self, call: &FnCallExpr, index: usize) -> Result<String> {
        static_string(
            call.args
                .get(index)
                .ok_or_else(|| anyhow!("missing static text argument"))?,
            self,
        )
        .ok_or_else(|| {
            self.error(
                "WT103",
                "identifier argument must be a string literal or const",
                call.args
                    .get(index)
                    .map(Expr::position)
                    .unwrap_or(Position::START),
            )
        })
    }

    fn require_static_pattern(&self, call: &FnCallExpr, index: usize) -> Result<()> {
        let pattern = self.require_static_text(call, index)?;
        if self.patterns.contains_key(&pattern) {
            Ok(())
        } else {
            Err(self.error(
                "WT106",
                format!("unknown pattern ID {pattern:?}"),
                call.args[index].position(),
            ))
        }
    }

    fn require_static_diagnostic(&self, call: &FnCallExpr, index: usize) -> Result<()> {
        let code = self.require_static_text(call, index)?;
        if self.diagnostics.contains_key(&code) {
            Ok(())
        } else {
            Err(self.error(
                "WT106",
                format!("unknown diagnostic ID {code:?}"),
                call.args[index].position(),
            ))
        }
    }

    fn check_binding_name(&self, name: &str, pos: Position) -> Result<()> {
        if matches!(name, "file" | "repo" | "emit" | "text" | "path" | "rx") {
            Err(self.error(
                "WT105",
                format!("reserved host name {name:?} cannot be shadowed"),
                pos,
            ))
        } else if !is_identifier(name) {
            Err(self.error("WT100", "invalid binding name", pos))
        } else {
            Ok(())
        }
    }

    fn current_scope_mut(&mut self) -> &mut Scope {
        self.scopes.last_mut().unwrap()
    }

    fn push_scope(&mut self) -> Result<()> {
        if self.scopes.len() >= 64 {
            bail!("WT102 detector nesting depth exceeds 64");
        }
        self.scopes.push(Scope::default());
        Ok(())
    }

    fn check_binding_budget(&self) -> Result<()> {
        if self
            .scopes
            .iter()
            .map(|scope| scope.bindings.len())
            .sum::<usize>()
            >= 256
        {
            bail!("WT102 detector local-binding limit exceeded");
        }
        Ok(())
    }
    fn lookup(&self, name: &str) -> Option<Binding> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.bindings.get(name).cloned())
    }
}

fn call_position(call: &FnCallExpr) -> Position {
    call.args
        .first()
        .map(Expr::position)
        .unwrap_or(Position::START)
}
fn variable_name(expression: &Expr) -> Option<&str> {
    match expression {
        Expr::Variable(data, ..) => {
            if data.2.is_empty() {
                Some(data.1.as_str())
            } else {
                None
            }
        }
        _ => None,
    }
}
fn is_literal(expression: &Expr) -> bool {
    matches!(
        expression,
        Expr::BoolConstant(..)
            | Expr::IntegerConstant(..)
            | Expr::StringConstant(..)
            | Expr::Unit(_)
    )
}
fn static_string(expression: &Expr, validator: &Validator<'_>) -> Option<String> {
    match expression {
        Expr::StringConstant(value, _) => Some(value.to_string()),
        Expr::Variable(data, ..) => {
            let name = data.1.as_str();
            validator.lookup(name).and_then(|binding| binding.constant)
        }
        _ => None,
    }
}

fn compatible(actual: &Ty, expected: &Ty) -> bool {
    actual == expected || matches!(actual, Ty::Optional(inner) if inner.as_ref() == expected)
}

fn equality_compatible(left: &Ty, right: &Ty) -> bool {
    if left == &Ty::Unit || right == &Ty::Unit {
        return matches!(left, Ty::Unit | Ty::Optional(_))
            && matches!(right, Ty::Unit | Ty::Optional(_));
    }
    let left = left.unwrap_optional();
    let right = right.unwrap_optional();
    left == right && matches!(left, Ty::Bool | Ty::Int | Ty::Text)
}

fn validate_scope_glob(glob: &str, pos: Position) -> Result<()> {
    if glob.is_empty()
        || glob.starts_with('/')
        || glob.contains('\\')
        || glob.split('/').any(|part| part == "..")
    {
        bail!("WT103 invalid root-relative path glob at {pos}");
    }
    Ok(())
}

fn reject_unsupported_literal_syntax(source: &str) -> Result<()> {
    let bytes = source.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'/') {
            index = source[index..]
                .find('\n')
                .map_or(bytes.len(), |offset| index + offset + 1);
            continue;
        }
        if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'*') {
            index = source[index + 2..]
                .find("*/")
                .map(|offset| index + offset + 4)
                .ok_or_else(|| anyhow!("WT100 unterminated comment"))?;
            continue;
        }
        if bytes[index] == b'`' {
            bail!("WT102 raw/interpolated strings are not part of wt-rule-1");
        }
        if bytes[index] == b'r'
            && bytes
                .get(index + 1)
                .is_some_and(|byte| *byte == b'"' || *byte == b'#')
        {
            bail!("WT102 raw strings are not part of wt-rule-1");
        }
        if bytes[index] == b'\'' {
            bail!("WT102 character literals are not part of wt-rule-1");
        }
        if bytes[index] == b'"' {
            index += 1;
            while index < bytes.len() {
                if bytes[index] == b'\\' {
                    index = index.saturating_add(2);
                    continue;
                }
                if bytes[index] == b'"' {
                    index += 1;
                    break;
                }
                index += 1;
            }
            continue;
        }
        index += 1;
    }
    Ok(())
}

#[derive(Default)]
struct Env {
    scopes: Vec<HashMap<String, (Value, bool)>>,
}
impl Env {
    fn define(&mut self, name: &str, value: Value, mutable: bool) {
        if self.scopes.is_empty() {
            self.scopes.push(HashMap::new());
        }
        self.scopes
            .last_mut()
            .unwrap()
            .insert(name.to_owned(), (value, mutable));
    }
    fn push(&mut self) {
        self.scopes.push(HashMap::new());
    }
    fn pop(&mut self) {
        self.scopes.pop();
    }
    fn get_ref(&self, name: &str) -> Option<&Value> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name).map(|(value, _)| value))
    }
    fn set(&mut self, name: &str, value: Value) -> Result<()> {
        for scope in self.scopes.iter_mut().rev() {
            if let Some((slot, mutable)) = scope.get_mut(name) {
                if !*mutable {
                    bail!("immutable binding {name}");
                }
                *slot = value;
                return Ok(());
            }
        }
        bail!("unknown binding {name}")
    }
}

enum Flow {
    Normal,
    Break,
    Continue,
    Return,
}

struct EvalContext<'a> {
    program: &'a Program,
    files: &'a [SourceFile],
    arena: &'a mut QueryArena,
    diagnostics: Vec<RawDiagnostic>,
    steps: u64,
    iterations: u64,
    loop_depth: usize,
    native_bytes: u64,
    temporary_bytes: usize,
    max_steps: u64,
    max_native_bytes: u64,
    current_file: Option<usize>,
}
impl EvalContext<'_> {
    fn charge(&mut self) -> Result<()> {
        self.steps = self
            .steps
            .checked_add(1)
            .ok_or_else(|| anyhow!("step budget overflow"))?;
        if self.steps > self.max_steps {
            bail!("logical WRL step budget exceeded");
        }
        Ok(())
    }
    fn charge_native(&mut self, bytes: usize) -> Result<()> {
        self.native_bytes = self.native_bytes.saturating_add(bytes as u64);
        if self.native_bytes > self.max_native_bytes {
            bail!("logical native input-byte budget exceeded");
        }
        Ok(())
    }
    fn charge_temporary(&mut self, bytes: usize) -> Result<()> {
        charge_consumer_budget(&mut self.temporary_bytes, bytes)
    }
    fn exec_block(&mut self, block: &[Stmt], env: &mut Env) -> Result<Flow> {
        for stmt in block {
            if !matches!(stmt, Stmt::Block(_)) {
                self.charge()?;
            }
            match self.exec_stmt(stmt, env)? {
                Flow::Normal => {}
                flow => return Ok(flow),
            }
        }
        Ok(Flow::Normal)
    }
    fn exec_stmt(&mut self, stmt: &Stmt, env: &mut Env) -> Result<Flow> {
        match stmt {
            Stmt::Noop(_) => Ok(Flow::Normal),
            Stmt::Var(data, flags, _) => {
                let (ident, expr, _) = &**data;
                let value = self.eval_expr(expr, env)?;
                env.define(
                    ident.name.as_str(),
                    value,
                    !flags.contains(ASTFlags::CONSTANT),
                );
                Ok(Flow::Normal)
            }
            Stmt::Assignment(data) => {
                let name = variable_name(&data.1.lhs)
                    .ok_or_else(|| anyhow!("invalid assignment"))?
                    .to_owned();
                let value = self.eval_expr(&data.1.rhs, env)?;
                env.set(&name, value)?;
                Ok(Flow::Normal)
            }
            Stmt::Expr(expr) => {
                self.eval_expr(expr, env)?;
                Ok(Flow::Normal)
            }
            Stmt::FnCall(call, _) => {
                self.eval_call(call, env)?;
                Ok(Flow::Normal)
            }
            Stmt::If(flow, _) => {
                let condition = self.eval_expr(&flow.expr, env)?;
                match condition {
                    Value::Bool(true) => {
                        env.push();
                        let result = self.exec_block(flow.body.statements(), env)?;
                        env.pop();
                        Ok(result)
                    }
                    Value::Bool(false) => {
                        env.push();
                        let result = self.exec_block(flow.branch.statements(), env)?;
                        env.pop();
                        Ok(result)
                    }
                    _ => bail!("condition is not boolean"),
                }
            }
            Stmt::For(data, _) => {
                let sequence = self.eval_expr(&data.2.expr, env)?;
                let Value::Sequence(values) = sequence else {
                    bail!("for expression is not a WT sequence")
                };
                self.loop_depth += 1;
                for value in values {
                    self.charge()?;
                    self.iterations = self.iterations.saturating_add(1);
                    env.push();
                    env.define(data.0.name.as_str(), value, false);
                    let result = self.exec_block(data.2.body.statements(), env)?;
                    env.pop();
                    match result {
                        Flow::Normal | Flow::Continue => {}
                        Flow::Break => break,
                        Flow::Return => {
                            self.loop_depth -= 1;
                            return Ok(Flow::Return);
                        }
                    }
                }
                self.loop_depth -= 1;
                Ok(Flow::Normal)
            }
            Stmt::Block(block) => {
                env.push();
                let result = self.exec_block(block.statements(), env)?;
                env.pop();
                Ok(result)
            }
            Stmt::BreakLoop(_, flags, _) => {
                if flags.contains(ASTFlags::BREAK) {
                    Ok(Flow::Break)
                } else {
                    Ok(Flow::Continue)
                }
            }
            Stmt::Return(_, _, _) => Ok(Flow::Return),
            _ => bail!("unsupported statement reached after validation"),
        }
    }

    fn eval_expr(&mut self, expr: &Expr, env: &mut Env) -> Result<Value> {
        // Rhai's dot-chain and call container nodes are syntax scaffolding;
        // charge their WT properties/calls at the actual operation below.
        if !matches!(
            expr,
            Expr::Dot(..) | Expr::FnCall(..) | Expr::MethodCall(..) | Expr::Stmt(_)
        ) {
            self.charge()?;
        }
        match expr {
            Expr::BoolConstant(value, _) => Ok(Value::Bool(*value)),
            Expr::IntegerConstant(value, _) => Ok(Value::Int(*value)),
            Expr::StringConstant(value, _) => {
                let value = value.to_string();
                self.charge_temporary(value.len())?;
                Ok(Value::Text(value.into()))
            }
            Expr::Unit(_) => Ok(Value::Unit),
            Expr::Variable(data, ..) => {
                let name = data.1.as_str();
                let value = env
                    .get_ref(name)
                    .ok_or_else(|| anyhow!("unknown binding {name}"))?;
                self.charge_temporary(value_size(value))?;
                Ok(value.clone())
            }
            Expr::And(values, _) => {
                for value in values.iter() {
                    let result = self.eval_expr(value, env)?;
                    if !expect_bool(Some(&result))? {
                        return Ok(Value::Bool(false));
                    }
                }
                Ok(Value::Bool(true))
            }
            Expr::Or(values, _) => {
                for value in values.iter() {
                    let result = self.eval_expr(value, env)?;
                    if expect_bool(Some(&result))? {
                        return Ok(Value::Bool(true));
                    }
                }
                Ok(Value::Bool(false))
            }
            Expr::Dot(data, _, _) => {
                let left = self.eval_expr(&data.lhs, env)?;
                self.eval_dot(left, &data.rhs, env)
            }
            Expr::FnCall(call, _) | Expr::MethodCall(call, _) => self.eval_call(call, env),
            Expr::Stmt(block) => {
                env.push();
                self.exec_block(block.statements(), env)?;
                env.pop();
                Ok(Value::Unit)
            }
            _ => bail!("unsupported expression reached after validation"),
        }
    }

    fn eval_dot(&mut self, left: Value, rhs: &Expr, env: &mut Env) -> Result<Value> {
        match rhs {
            Expr::Property(data, _) => {
                self.charge()?;
                self.property(left, data.2.as_str())
            }
            Expr::FnCall(call, _) | Expr::MethodCall(call, _) => self.eval_method(left, call, env),
            Expr::Dot(data, _, _) => {
                let middle = self.eval_dot(left, &data.lhs, env)?;
                self.eval_dot(middle, &data.rhs, env)
            }
            _ => bail!("invalid property access"),
        }
    }
    fn property(&mut self, value: Value, name: &str) -> Result<Value> {
        match (value, name) {
            (Value::File(file), "path") => {
                self.current_file = Some(file.index);
                let path = &self.files[file.index].path;
                self.charge_temporary(path.len())?;
                Ok(Value::Text(path.to_owned().into()))
            }
            (Value::File(file), "text") => {
                self.current_file = Some(file.index);
                let text = &self.files[file.index].text;
                self.charge_temporary(text.len())?;
                Ok(Value::Text(text.to_owned()))
            }
            (Value::File(file), "span") => Ok(Value::Span(SpanValue {
                file: file.index,
                start: 0,
                end: self.files[file.index].text.len(),
            })),
            (Value::Match(value), "text") => Ok(Value::Text(value.text)),
            (Value::Match(value), "span") if value.reportable => Ok(Value::Span(SpanValue {
                file: value.file,
                start: value.start,
                end: value.end,
            })),
            (Value::Match(_), "span") => bail!("capture_text matches do not have reportable spans"),
            (Value::Sequence(values), "len") => Ok(Value::Int(values.len() as i64)),
            (Value::Sequence(values), "is_empty") => Ok(Value::Bool(values.is_empty())),
            (Value::Line(line), "text") => {
                self.charge_temporary(line.text.len())?;
                Ok(Value::Text(line.text))
            }
            (Value::Line(line), "span") => Ok(Value::Span(SpanValue {
                file: line.file,
                start: line.start,
                end: line.end,
            })),
            _ => bail!("unknown or unavailable WT property {name}"),
        }
    }

    fn eval_method(&mut self, receiver: Value, call: &FnCallExpr, env: &mut Env) -> Result<Value> {
        self.charge()?;
        self.charge_temporary((call.args.len() + 1) * LOGICAL_VALUE_BYTES)?;
        let mut args = Vec::with_capacity(call.args.len() + 1);
        args.push(receiver);
        for arg in &call.args {
            args.push(self.eval_expr(arg, env)?);
        }
        let namespace = if matches!(args.first(), Some(Value::Repo)) {
            "repo"
        } else {
            ""
        };
        self.eval_named_call(namespace, call.name.as_str(), args, env)
    }
    fn eval_call(&mut self, call: &FnCallExpr, env: &mut Env) -> Result<Value> {
        self.charge()?;
        let namespace = namespace_name(call);
        self.charge_temporary(call.args.len() * LOGICAL_VALUE_BYTES)?;
        let mut args = Vec::with_capacity(call.args.len());
        for arg in &call.args {
            args.push(self.eval_expr(arg, env)?);
        }
        if namespace.is_empty()
            && matches!(
                call.name.as_str(),
                "+" | "-" | "*" | "/" | "%" | "==" | "!=" | "<" | "<=" | ">" | ">=" | "!"
            )
        {
            return self.eval_operator(call.name.as_str(), args);
        }
        self.eval_named_call(&namespace, call.name.as_str(), args, env)
    }
    fn eval_named_call(
        &mut self,
        namespace: &str,
        name: &str,
        args: Vec<Value>,
        _env: &mut Env,
    ) -> Result<Value> {
        match (namespace, name) {
            ("", "emit") => {
                let span = expect_span(args.first())?;
                let code = expect_text(args.get(1))?;
                if !self.program.diagnostics.contains_key(code) {
                    bail!("unknown diagnostic {code}");
                }
                if self.diagnostics.len() >= MAX_DIAGNOSTICS {
                    bail!("diagnostic limit exceeded");
                }
                let file = self
                    .files
                    .get(span.file)
                    .ok_or_else(|| anyhow!("span is outside authorized files"))?;
                self.charge_temporary(file.path.len().saturating_add(code.len()))?;
                if span.end > file.text.len()
                    || span.start > span.end
                    || !file.text.is_char_boundary(span.start)
                    || !file.text.is_char_boundary(span.end)
                {
                    bail!("invalid reportable span");
                }
                self.diagnostics.push(RawDiagnostic {
                    path: file.path.clone(),
                    code: code.to_owned(),
                    start_byte: span.start,
                    end_byte: span.end,
                });
                Ok(Value::Unit)
            }
            (
                "",
                "contains" | "starts_with" | "ends_with" | "trim" | "split" | "replace"
                | "len_bytes",
            ) => self.text_call(name, args),
            ("", "lines") => self.text_call(name, args),
            ("", "is_empty") => match args.first() {
                Some(Value::Sequence(values)) => Ok(Value::Bool(values.is_empty())),
                _ => bail!("is_empty requires a WT sequence"),
            },
            ("", "ast_match") => {
                let (file_index, scope) = match args.first() {
                    Some(Value::File(file)) => (file.index, None),
                    Some(Value::Match(matched)) if matched.reportable => {
                        (matched.file, Some((matched.start, matched.end)))
                    }
                    Some(Value::Match(_)) => {
                        bail!("ast_match requires a match with a reportable span")
                    }
                    _ => bail!("ast_match requires a file or a match"),
                };
                let language_text = expect_text(args.get(1))?;
                let pattern_text = expect_text(args.get(2))?;
                let language = AstLanguage::parse(language_text)
                    .ok_or_else(|| anyhow!("unsupported ast.v1 language {language_text}"))?;
                let pattern = self
                    .program
                    .ast_patterns
                    .get(&(language, pattern_text.to_owned()))
                    .ok_or_else(|| anyhow!("uncompiled ast.v1 pattern"))?;
                let file = &self.files[file_index];
                self.charge_native(file.text.len())?;
                let (start, end, independent_slice) = match scope {
                    Some((start, end)) => (start, end, true),
                    None => (0, file.text.len(), false),
                };
                let matches = self.arena.ast_match(
                    file_index,
                    file,
                    language,
                    pattern_text,
                    pattern,
                    start,
                    end,
                    independent_slice,
                    &mut self.temporary_bytes,
                )?;
                Ok(Value::Sequence(
                    matches.into_iter().map(Value::Match).collect(),
                ))
            }
            ("", "node") => {
                let matched = match args.first() {
                    Some(Value::Match(matched)) => matched,
                    _ => bail!("node requires a match"),
                };
                match matched.node_spans.get(expect_text(args.get(1))?).copied() {
                    Some((start, end)) => {
                        let file = &self.files[matched.file];
                        self.charge_temporary(end.saturating_sub(start))?;
                        Ok(Value::Match(MatchValue {
                            file: matched.file,
                            start,
                            end,
                            text: file.text.slice(start..end),
                            groups: Arc::new(HashMap::new()),
                            node_spans: Arc::new(HashMap::new()),
                            reportable: true,
                        }))
                    }
                    None => Ok(Value::Unit),
                }
            }
            ("", "group_text") => {
                let matched = match args.first() {
                    Some(Value::Match(matched)) => matched,
                    _ => bail!("group_text requires a match"),
                };
                let name = expect_text(args.get(1))?;
                let value = matched.groups.get(name).and_then(Option::as_ref);
                self.charge_temporary(value.map_or(0, |value| value.len()))?;
                Ok(value.map_or(Value::Unit, |value| Value::Text(value.clone())))
            }
            (
                "text",
                "contains" | "starts_with" | "ends_with" | "trim" | "split" | "replace"
                | "len_bytes" | "lines",
            ) => self.text_call(name, args),
            ("path", "matches") => {
                let path = expect_text(args.first())?;
                let glob = expect_text(args.get(1))?;
                self.charge_native(path.len().saturating_add(glob.len()))?;
                let matcher = self
                    .program
                    .path_globs
                    .get(glob)
                    .ok_or_else(|| anyhow!("uncompiled path glob"))?;
                Ok(Value::Bool(matcher.is_match(path)))
            }
            ("rx", "is_match" | "find_all" | "find_in" | "capture_text") => {
                self.regex_call(name, args)
            }
            ("repo", "files") => {
                self.charge_temporary(self.files.len() * std::mem::size_of::<usize>())?;
                let mut indexes: Vec<_> = (0..self.files.len()).collect();
                indexes.sort_by(|left, right| self.files[*left].path.cmp(&self.files[*right].path));
                self.charge_temporary(indexes.len() * LOGICAL_VALUE_BYTES)?;
                Ok(Value::Sequence(
                    indexes
                        .into_iter()
                        .map(|index| Value::File(FileValue { index }))
                        .collect(),
                ))
            }
            _ => bail!("unregistered WT call {namespace}::{name}"),
        }
    }
    fn eval_operator(&mut self, name: &str, args: Vec<Value>) -> Result<Value> {
        if name == "!" {
            return Ok(Value::Bool(!expect_bool(args.first())?));
        }
        if matches!(name, "+" | "-") && args.len() == 1 {
            let value = expect_int(args.first())?;
            return if name == "+" {
                Ok(Value::Int(value))
            } else {
                Ok(Value::Int(
                    value
                        .checked_neg()
                        .ok_or_else(|| anyhow!("integer overflow"))?,
                ))
            };
        }
        if args.len() != 2 {
            bail!("operator {name} requires two operands");
        }
        let left = &args[0];
        let right = &args[1];
        match name {
            "+" => match (left, right) {
                (Value::Int(a), Value::Int(b)) => Ok(Value::Int(
                    a.checked_add(*b)
                        .ok_or_else(|| anyhow!("integer overflow"))?,
                )),
                (Value::Text(a), Value::Text(b)) => {
                    let bytes = a
                        .len()
                        .checked_add(b.len())
                        .ok_or_else(|| anyhow!("temporary string size overflow"))?;
                    self.charge_temporary(bytes)?;
                    let mut value = String::with_capacity(bytes);
                    value.push_str(a);
                    value.push_str(b);
                    Ok(Value::Text(value.into()))
                }
                _ => bail!("invalid operands to +"),
            },
            "-" | "*" | "/" | "%" => {
                let (a, b) = (expect_int(Some(left))?, expect_int(Some(right))?);
                let value = match name {
                    "-" => a.checked_sub(b),
                    "*" => a.checked_mul(b),
                    "/" if b != 0 => a.checked_div(b),
                    "%" if b != 0 => a.checked_rem(b),
                    _ => None,
                };
                Ok(Value::Int(value.ok_or_else(|| {
                    anyhow!("integer overflow or division by zero")
                })?))
            }
            "==" => Ok(Value::Bool(equal_values(left, right))),
            "!=" => Ok(Value::Bool(!equal_values(left, right))),
            "<" | "<=" | ">" | ">=" => {
                let result = match (left, right) {
                    (Value::Int(a), Value::Int(b)) => compare_int(name, *a, *b),
                    (Value::Text(a), Value::Text(b)) => compare_str(name, a, b),
                    _ => bail!("invalid comparison operands"),
                };
                Ok(Value::Bool(result))
            }
            _ => bail!("unsupported operator {name}"),
        }
    }
    fn text_call(&mut self, name: &str, args: Vec<Value>) -> Result<Value> {
        if name == "lines" {
            let file = expect_file(args.first())?;
            let source = &self.files[file.index].text;
            self.charge_native(source.len())?;
            let line_count = source.bytes().filter(|byte| *byte == b'\n').count()
                + usize::from(!source.is_empty() && !source.ends_with('\n'));
            if line_count > MAX_SEQUENCE {
                bail!("sequence limit exceeded");
            }
            let output_bytes = source
                .len()
                .saturating_add(line_count.saturating_mul(32))
                .saturating_add(line_count * LOGICAL_VALUE_BYTES);
            let key = text_key(
                &self.files[file.index].path,
                source,
                name,
                &[],
                &mut self.arena.stats.cache_key_bytes_hashed,
            );
            return self.arena.text_query(
                key,
                Some(file.index),
                output_bytes,
                &mut self.temporary_bytes,
                || {
                    let mut lines = Vec::with_capacity(line_count);
                    let mut start = 0;
                    for (index, character) in source.char_indices() {
                        if character == '\n' {
                            let mut end = index;
                            if end > start && source.as_bytes()[end - 1] == b'\r' {
                                end -= 1;
                            }
                            lines.push(Value::Line(LineValue {
                                file: file.index,
                                start,
                                end,
                                text: source.slice(start..end),
                            }));
                            start = index + 1;
                        }
                    }
                    if start < source.len() {
                        lines.push(Value::Line(LineValue {
                            file: file.index,
                            start,
                            end: source.len(),
                            text: source.slice(start..source.len()),
                        }));
                    }
                    Ok(Value::Sequence(lines))
                },
            );
        }
        let text = expect_shared_text(args.first())?;
        let operand_bytes = args.iter().map(value_size).sum::<usize>();
        self.charge_native(operand_bytes)?;
        let argument_texts = args
            .iter()
            .skip(1)
            .map(|value| expect_shared_text(Some(value)))
            .collect::<Result<Vec<_>>>()?;
        let path = self
            .current_file
            .map_or("", |index| self.files[index].path.as_str());
        let key = text_key(
            path,
            text,
            name,
            &argument_texts,
            &mut self.arena.stats.cache_key_bytes_hashed,
        );
        match name {
            "contains" => {
                let needle = expect_text(args.get(1))?;
                self.arena
                    .text_query(key, None, 0, &mut self.temporary_bytes, || {
                        Ok(Value::Bool(text.contains(needle)))
                    })
            }
            "starts_with" => {
                let needle = expect_text(args.get(1))?;
                self.arena
                    .text_query(key, None, 0, &mut self.temporary_bytes, || {
                        Ok(Value::Bool(text.starts_with(needle)))
                    })
            }
            "ends_with" => {
                let needle = expect_text(args.get(1))?;
                self.arena
                    .text_query(key, None, 0, &mut self.temporary_bytes, || {
                        Ok(Value::Bool(text.ends_with(needle)))
                    })
            }
            "trim" => {
                let trimmed = text.trim();
                self.arena
                    .text_query(key, None, trimmed.len(), &mut self.temporary_bytes, || {
                        Ok(Value::Text(trimmed.to_owned().into()))
                    })
            }
            "split" => {
                let separator = expect_text(args.get(1))?;
                if separator.is_empty() {
                    bail!("empty split separator");
                }
                let count = text.matches(separator).count() + 1;
                if count > MAX_SEQUENCE {
                    bail!("sequence limit exceeded");
                }
                let output_bytes = text.len().saturating_add(count * LOGICAL_VALUE_BYTES);
                self.arena
                    .text_query(key, None, output_bytes, &mut self.temporary_bytes, || {
                        let values = text
                            .split(separator)
                            .map(|part| Value::Text(part.to_owned().into()))
                            .collect::<Vec<_>>();
                        Ok(Value::Sequence(values))
                    })
            }
            "replace" => {
                let from = expect_text(args.get(1))?;
                let to = expect_text(args.get(2))?;
                if from.is_empty() {
                    bail!("empty replace needle");
                }
                let occurrences = text.matches(from).count();
                let value_len = text
                    .len()
                    .checked_add(occurrences.saturating_mul(to.len().saturating_sub(from.len())))
                    .ok_or_else(|| anyhow!("temporary string size overflow"))?;
                self.arena
                    .text_query(key, None, value_len, &mut self.temporary_bytes, || {
                        Ok(Value::Text(text.replace(from, to).into()))
                    })
            }
            "len_bytes" => self
                .arena
                .text_query(key, None, 0, &mut self.temporary_bytes, || {
                    Ok(Value::Int(text.len() as i64))
                }),
            _ => bail!("unknown text API"),
        }
    }
    fn regex_call(&mut self, name: &str, args: Vec<Value>) -> Result<Value> {
        match name {
            "is_match" => {
                let file = expect_file(args.first())?;
                let pattern_name = expect_text(args.get(1))?;
                let pattern = self
                    .program
                    .patterns
                    .get(pattern_name)
                    .ok_or_else(|| anyhow!("unknown pattern {pattern_name}"))?;
                let text = &self.files[file.index];
                self.charge_native(text.text.len())?;
                Ok(Value::Bool(self.arena.is_match(
                    text,
                    pattern,
                    0,
                    text.text.len(),
                    false,
                )?))
            }
            "find_all" => {
                let file = expect_file(args.first())?;
                let pattern_name = expect_text(args.get(1))?;
                let pattern = self
                    .program
                    .patterns
                    .get(pattern_name)
                    .ok_or_else(|| anyhow!("unknown pattern {pattern_name}"))?;
                let text = &self.files[file.index];
                self.charge_native(text.text.len())?;
                let matches = self.arena.find_all(
                    file.index,
                    text,
                    pattern,
                    0,
                    text.text.len(),
                    false,
                    "find_all",
                    &mut self.temporary_bytes,
                )?;
                Ok(Value::Sequence(
                    matches.into_iter().map(Value::Match).collect(),
                ))
            }
            "find_in" => {
                let span = expect_span(args.first())?;
                let pattern_name = expect_text(args.get(1))?;
                let pattern = self
                    .program
                    .patterns
                    .get(pattern_name)
                    .ok_or_else(|| anyhow!("unknown pattern {pattern_name}"))?;
                let text = &self.files[span.file];
                self.charge_native(span.end.saturating_sub(span.start))?;
                let matches = self.arena.find_all(
                    span.file,
                    text,
                    pattern,
                    span.start,
                    span.end,
                    true,
                    "find_in",
                    &mut self.temporary_bytes,
                )?;
                Ok(Value::Sequence(
                    matches.into_iter().map(Value::Match).collect(),
                ))
            }
            "capture_text" => {
                let pattern_name = expect_text(args.first())?;
                let pattern = self
                    .program
                    .patterns
                    .get(pattern_name)
                    .ok_or_else(|| anyhow!("unknown pattern {pattern_name}"))?;
                let text = expect_shared_text(args.get(1))?;
                self.charge_native(text.len())?;
                let matched = self
                    .arena
                    .capture_text(pattern, text, &mut self.temporary_bytes)?;
                Ok(matched.map(Value::Match).unwrap_or(Value::Unit))
            }
            _ => bail!("unknown regex API"),
        }
    }
}

fn expect_shared_text(value: Option<&Value>) -> Result<&SharedText> {
    match value {
        Some(Value::Text(value)) => Ok(value),
        _ => bail!("expected text"),
    }
}

fn expect_text(value: Option<&Value>) -> Result<&str> {
    match value {
        Some(Value::Text(value)) => Ok(value),
        _ => bail!("expected text"),
    }
}

fn text_key(
    path: &str,
    text: &SharedText,
    operation: &str,
    arguments: &[&SharedText],
    bytes_hashed: &mut u64,
) -> TextKey {
    TextKey {
        path: path.to_owned(),
        digest: text.digest(bytes_hashed),
        operation: operation.to_owned(),
        arguments: arguments
            .iter()
            .map(|argument| argument.digest(bytes_hashed))
            .collect(),
    }
}

fn remap_line_files(value: &mut Value, file_index: Option<usize>) {
    let Some(file_index) = file_index else {
        return;
    };
    if let Value::Sequence(values) = value {
        for value in values {
            if let Value::Line(line) = value {
                line.file = file_index;
            }
        }
    }
}

fn value_size(value: &Value) -> usize {
    match value {
        Value::Text(text) => text.len(),
        Value::Sequence(values) => values.iter().map(value_size).sum::<usize>() + values.len() * 16,
        Value::Match(matched) => match_value_size(matched),
        Value::Line(line) => line.text.len() + 32,
        _ => 32,
    }
}

fn match_value_size(value: &MatchValue) -> usize {
    value.text.len()
        + value
            .groups
            .iter()
            .map(|(name, value)| name.len() + 24 + value.as_ref().map_or(0, |value| value.len()))
            .sum::<usize>()
        + value.node_spans.len() * 32
        + 64
}
fn expect_bool(value: Option<&Value>) -> Result<bool> {
    match value {
        Some(Value::Bool(value)) => Ok(*value),
        _ => bail!("expected boolean"),
    }
}
fn expect_int(value: Option<&Value>) -> Result<i64> {
    match value {
        Some(Value::Int(value)) => Ok(*value),
        _ => bail!("expected integer"),
    }
}
fn expect_file(value: Option<&Value>) -> Result<FileValue> {
    match value {
        Some(Value::File(value)) => Ok(value.clone()),
        _ => bail!("expected file handle"),
    }
}
fn expect_span(value: Option<&Value>) -> Result<SpanValue> {
    match value {
        Some(Value::Span(value)) => Ok(value.clone()),
        _ => bail!("expected source span"),
    }
}
fn equal_values(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Unit, Value::Unit) => true,
        (Value::Bool(a), Value::Bool(b)) => a == b,
        (Value::Int(a), Value::Int(b)) => a == b,
        (Value::Text(a), Value::Text(b)) => a == b,
        _ => false,
    }
}
fn compare_int(op: &str, a: i64, b: i64) -> bool {
    match op {
        "<" => a < b,
        "<=" => a <= b,
        ">" => a > b,
        ">=" => a >= b,
        _ => false,
    }
}
fn compare_str(op: &str, a: &str, b: &str) -> bool {
    match op {
        "<" => a < b,
        "<=" => a <= b,
        ">" => a > b,
        ">=" => a >= b,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(patterns: JsonValue, diagnostics: JsonValue, capabilities: &[&str]) -> JsonValue {
        json!({"execution":"file","patterns":patterns,"diagnostics":diagnostics,"code":{"language":"wt-rule-1","capabilities":capabilities}})
    }

    #[test]
    fn rejects_arbitrary_rhai_and_dead_forbidden_syntax() {
        let manifest = manifest(json!({}), json!({"x":{"kind":"violation"}}), &[]);
        let error = compile(&manifest, "if false { while true { break; } }")
            .err()
            .expect("forbidden syntax must fail")
            .to_string();
        assert!(error.contains("WT102"));
    }

    #[test]
    fn reports_source_bound_matches() {
        let manifest = manifest(
            json!({"call":"request\\([^\\r\\n]*\\)"}),
            json!({"hit":{"kind":"violation"}}),
            &["text.v1"],
        );
        let program = compile(
            &manifest,
            "for m in rx::find_all(file, \"call\") { emit(m.span, \"hit\"); }",
        )
        .unwrap();
        let mut arena = QueryArena::new(true);
        let diagnostics = program
            .execute(
                &[SourceFile {
                    path: "src/a.rs".into(),
                    text: "request(1)".into(),
                }],
                &mut arena,
            )
            .unwrap();
        assert_eq!(
            diagnostics,
            vec![RawDiagnostic {
                path: "src/a.rs".into(),
                code: "hit".into(),
                start_byte: 0,
                end_byte: 10
            }]
        );
    }

    #[test]
    fn identical_queries_share_but_findings_do_not() {
        let first = manifest(
            json!({"call":"request\\([^\\r\\n]*\\)"}),
            json!({"a":{"kind":"violation"}}),
            &["text.v1"],
        );
        let second = manifest(
            json!({"other":"request\\([^\\r\\n]*\\)"}),
            json!({"b":{"kind":"violation"}}),
            &["text.v1"],
        );
        let a = compile(
            &first,
            "for m in rx::find_all(file, \"call\") { emit(m.span, \"a\"); }",
        )
        .unwrap();
        let b = compile(
            &second,
            "for m in rx::find_all(file, \"other\") { emit(m.span, \"b\"); }",
        )
        .unwrap();
        let files = [SourceFile {
            path: "x".into(),
            text: "request(foo)".into(),
        }];
        let mut arena = QueryArena::new(true);
        let mut result = a.execute(&files, &mut arena).unwrap();
        result.extend(b.execute(&files, &mut arena).unwrap());
        assert_eq!(result.len(), 2);
        assert_eq!(arena.stats()["physical_query_evaluations"], 1);
        assert_eq!(arena.stats()["shared_result_hits"], 1);
    }

    #[test]
    fn text_and_short_circuit_are_deterministic() {
        let manifest = manifest(
            json!({}),
            json!({"missing":{"kind":"violation"}}),
            &["text.v1"],
        );
        let program = compile(
            &manifest,
            "if !text::contains(file.text, \"needle\") { emit(file.span, \"missing\"); }",
        )
        .unwrap();
        let mut arena = QueryArena::new(true);
        let result = program
            .execute(
                &[SourceFile {
                    path: "x".into(),
                    text: "body".into(),
                }],
                &mut arena,
            )
            .unwrap();
        assert_eq!(result[0].start_byte, 0);
    }

    #[test]
    fn repository_rules_receive_the_authorized_snapshot() {
        let mut value = manifest(
            json!({}),
            json!({"hit":{"kind":"violation"}}),
            &["text.v1", "repo.v1"],
        );
        value["execution"] = json!("repository");
        let program = compile(&value, "for f in repo.files() { if text::contains(f.text, \"bad\") { emit(f.span, \"hit\"); } }").unwrap();
        let mut arena = QueryArena::new(true);
        let result = program
            .execute(
                &[
                    SourceFile {
                        path: "a".into(),
                        text: "ok".into(),
                    },
                    SourceFile {
                        path: "b".into(),
                        text: "bad".into(),
                    },
                ],
                &mut arena,
            )
            .unwrap();
        assert_eq!(result[0].path, "b");
    }

    #[test]
    fn lines_preserve_source_spans_and_crlf() {
        let value = manifest(json!({}), json!({"hit":{"kind":"violation"}}), &["text.v1"]);
        let program = compile(&value, "for line in text::lines(file) { if line.text == \"bad\" { emit(line.span, \"hit\"); } }").unwrap();
        let mut arena = QueryArena::new(true);
        let result = program
            .execute(
                &[SourceFile {
                    path: "x".into(),
                    text: "ok\r\nbad\r\n".into(),
                }],
                &mut arena,
            )
            .unwrap();
        assert_eq!((result[0].start_byte, result[0].end_byte), (4, 7));
    }

    #[test]
    fn ast_helper_ignores_strings_and_comments() {
        let value = manifest(json!({}), json!({"hit":{"kind":"violation"}}), &["ast.v1"]);
        let program = compile(
            &value,
            r#"for m in file.ast_match("tsx", "<input type=\"number\" />") { emit(m.span, "hit"); }"#,
        )
        .unwrap();
        let mut arena = QueryArena::new(true);
        let source = "const s = \"<input type=\\\"number\\\" />\"; // <input type=\\\"number\\\" />\nconst node = <input type=\"number\" />;";
        let result = program
            .execute(
                &[SourceFile {
                    path: "x.tsx".into(),
                    text: source.into(),
                }],
                &mut arena,
            )
            .unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(
            &source[result[0].start_byte..result[0].end_byte],
            "<input type=\"number\" />"
        );
    }

    #[test]
    fn ast_match_supports_nested_search_and_metavariable_capture() {
        // The N+1 example from the task: match a `for` loop, then search
        // inside its captured body for an ORM query call.
        let value = manifest(
            json!({}),
            json!({"n-plus-one":{"kind":"review"}}),
            &["ast.v1"],
        );
        let program = compile(
            &value,
            r#"
            for m in file.ast_match("python", "for $X in $ITER:\n    $$$BODY") {
                for q in m.node("BODY").ast_match("python", "$QS.objects.$METHOD($$$)") {
                    emit(q.span, "n-plus-one");
                }
            }
        "#,
        )
        .unwrap();
        let source = "for user in users:\n    posts = user.objects.filter(author=user)\n";
        let mut arena = QueryArena::new(true);
        let result = program
            .execute(
                &[SourceFile {
                    path: "x.py".into(),
                    text: source.into(),
                }],
                &mut arena,
            )
            .unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].code, "n-plus-one");
        assert_eq!(
            &source[result[0].start_byte..result[0].end_byte],
            "user.objects.filter(author=user)"
        );
    }

    /// Compile an example package's `rule.json` + `check.wt` and assert every
    /// case in its `tests.json` produces exactly the expected diagnostic
    /// codes. Shared by every `examples/*` fixture-suite test.
    fn run_example_fixture_suite(example_dir: &str) -> Program {
        let root =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../{example_dir}"));
        let manifest: JsonValue =
            serde_json::from_str(&std::fs::read_to_string(root.join("rule.json")).unwrap())
                .unwrap();
        let source = std::fs::read_to_string(root.join("check.wt")).unwrap();
        let program = compile(&manifest, &source).unwrap();
        let cases: JsonValue =
            serde_json::from_str(&std::fs::read_to_string(root.join("tests.json")).unwrap())
                .unwrap();
        for case in cases["cases"].as_array().unwrap() {
            let file = &case["files"][0];
            let fixture = root.join(file["fixture"].as_str().unwrap());
            let text = std::fs::read_to_string(fixture).unwrap();
            let mut expected = case["expect"]
                .as_array()
                .unwrap()
                .iter()
                .map(|finding| finding["code"].as_str().unwrap().to_owned())
                .collect::<Vec<_>>();
            expected.sort();
            let mut actual = program
                .execute(
                    &[SourceFile {
                        path: file["path"].as_str().unwrap().into(),
                        text: text.into(),
                    }],
                    &mut QueryArena::new(true),
                )
                .unwrap()
                .into_iter()
                .map(|finding| finding.code)
                .collect::<Vec<_>>();
            actual.sort();
            assert_eq!(
                actual, expected,
                "{example_dir} fixture case {}",
                case["name"]
            );
        }
        program
    }

    #[test]
    fn orm_query_in_loop_fixture_suite_matches_expected_codes() {
        run_example_fixture_suite("examples/orm-query-in-loop");
    }

    #[test]
    fn raw_sql_in_loop_fixture_suite_matches_expected_codes() {
        run_example_fixture_suite("examples/raw-sql-in-loop");
    }

    #[test]
    fn decimal_fixture_suite_matches_expected_codes() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/template-number-decimal-step");
        let program = run_example_fixture_suite("examples/template-number-decimal-step");
        let invalid = std::fs::read_to_string(root.join("fixtures/invalid-tsx.tsx")).unwrap();
        assert!(program
            .execute(
                &[SourceFile {
                    path: "frontend/src/invalid.tsx".into(),
                    text: invalid.into(),
                }],
                &mut QueryArena::new(true),
            )
            .is_err());
    }

    #[test]
    fn text_v1_supplies_core_matching_paths_and_repository_calls() {
        let mut value = manifest(
            json!({"hit":"bad"}),
            json!({"hit":{"kind":"violation"}}),
            &["text.v1"],
        );
        value["execution"] = json!("repository");
        let program = compile(&value, "for f in repo.files() { if rx::is_match(f, \"hit\") && path::matches(f.path, \"src/**\") { emit(f.span, \"hit\"); } }").unwrap();
        let mut arena = QueryArena::new(true);
        let result = program
            .execute(
                &[SourceFile {
                    path: "src/a".into(),
                    text: "bad".into(),
                }],
                &mut arena,
            )
            .unwrap();
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn validator_rejects_backend_constructs_and_bad_types_everywhere() {
        let value = manifest(json!({}), json!({"x":{"kind":"violation"}}), &["text.v1"]);
        for source in [
            "let x = { emit(file.span, \"x\"); };",
            "if false { let x = r#\"raw\"#; }",
            "if false { file.not_documented; }",
            "for x in file.text { emit(file.span, \"x\"); }",
            "if false { file.text.contains(); }",
            "if false && file.text.contains(1) { emit(file.span, \"x\"); }",
            "if 1 { emit(file.span, \"x\"); }",
        ] {
            assert!(
                compile(&value, source).is_err(),
                "accepted invalid WRL: {source}"
            );
        }
        let error = match compile(&value, "while true { emit(file.span, \"x\"); }") {
            Ok(_) => panic!("forbidden syntax was accepted"),
            Err(error) => error,
        };
        let error = error.to_string();
        assert!(error.starts_with("WT102"));
        assert!(error.contains("at "));
    }

    #[test]
    fn narrow_capabilities_authorize_only_their_own_surface() {
        let diagnostic = json!({"x":{"kind":"violation"}});
        let path = manifest(json!({}), diagnostic.clone(), &["path.v1"]);
        assert!(compile(&path, "path::matches(file.path, \"**\");").is_ok());
        assert!(compile(&path, "rx::is_match(file, \"missing\");").is_err());
        let repo = manifest(json!({}), diagnostic, &["repo.v1"]);
        assert!(compile(&repo, "text::contains(file.text, \"x\");").is_err());
    }

    #[test]
    fn unsupported_ast_language_fails_validation_with_a_clear_error() {
        let value = manifest(json!({}), json!({"x":{"kind":"violation"}}), &["ast.v1"]);
        let error = compile(
            &value,
            r#"for m in file.ast_match("cobol", "MOVE $X TO $Y") { emit(m.span, "x"); }"#,
        )
        .err()
        .expect("unsupported language must fail validation")
        .to_string();
        assert!(error.contains("WT104"));
        assert!(error.contains("cobol"));
        assert!(
            error.contains("python"),
            "lists supported languages: {error}"
        );
    }

    #[test]
    fn ast_match_requires_ast_v1_capability() {
        let value = manifest(json!({}), json!({"x":{"kind":"violation"}}), &["text.v1"]);
        let error = compile(
            &value,
            r#"for m in file.ast_match("python", "$X") { emit(m.span, "x"); }"#,
        )
        .err()
        .expect("ast_match without ast.v1 must fail validation")
        .to_string();
        assert!(error.contains("WT104"));
    }

    #[test]
    fn invalid_ast_pattern_fails_validation_not_execution() {
        let value = manifest(json!({}), json!({"x":{"kind":"violation"}}), &["ast.v1"]);
        let error = compile(
            &value,
            r#"for m in file.ast_match("python", "(((") { emit(m.span, "x"); }"#,
        )
        .err()
        .expect("a pattern that cannot parse must fail at compile time")
        .to_string();
        assert!(error.contains("WT100"));
    }

    #[test]
    fn constants_are_lexical_and_short_circuit_preserves_optional_guards() {
        let value = manifest(
            json!({"p":"bad"}),
            json!({"x":{"kind":"violation"}}),
            &["text.v1"],
        );
        assert!(compile(
            &value,
            "if true { const p = \"p\"; } rx::is_match(file, p);"
        )
        .is_err());
        let program = compile(
            &value,
            "let a = text::trim(\"\"); if false && a.not_a_property { emit(file.span, \"x\"); }",
        )
        .is_err();
        assert!(program);
        let guarded = compile(
            &manifest(json!({}), json!({"x":{"kind":"violation"}}), &["ast.v1"]),
            r#"for m in file.ast_match("tsx", "<input />") { let missing = m.node("missing"); if missing != () && missing.text == "z" { emit(missing.span, "x"); } }"#,
        )
        .unwrap();
        assert!(guarded
            .execute(
                &[SourceFile {
                    path: "x.tsx".into(),
                    text: "const x = <input />;".into()
                }],
                &mut QueryArena::new(true)
            )
            .unwrap()
            .is_empty());
    }

    #[test]
    fn capture_text_span_is_a_runtime_error_and_zero_width_is_complete() {
        let value = manifest(
            json!({"empty":""}),
            json!({"x":{"kind":"violation"}}),
            &["text.v1"],
        );
        let program = compile(
            &value,
            "let m = rx::capture_text(\"empty\", \"abc\"); if m != () { emit(m.span, \"x\"); }",
        )
        .unwrap();
        assert!(program
            .execute(
                &[SourceFile {
                    path: "x".into(),
                    text: "abc".into()
                }],
                &mut QueryArena::new(true)
            )
            .is_err());
        let value = manifest(
            json!({"empty":""}),
            json!({"x":{"kind":"violation"}}),
            &["text.v1"],
        );
        let program = compile(
            &value,
            "for m in rx::find_all(file, \"empty\") { emit(m.span, \"x\"); }",
        )
        .unwrap();
        let result = program
            .execute(
                &[SourceFile {
                    path: "x".into(),
                    text: "ab".into(),
                }],
                &mut QueryArena::new(true),
            )
            .unwrap();
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn query_identity_and_clear_are_observable_without_fake_snapshot_io() {
        let a = manifest(
            json!({"p":"a"}),
            json!({"x":{"kind":"violation"}}),
            &["text.v1"],
        );
        let b = manifest(
            json!({"p":"b"}),
            json!({"x":{"kind":"violation"}}),
            &["text.v1"],
        );
        let first = compile(&a, "rx::is_match(file, \"p\");").unwrap();
        let second = compile(&b, "rx::is_match(file, \"p\");").unwrap();
        let files = [SourceFile {
            path: "x".into(),
            text: "a".into(),
        }];
        let mut arena = QueryArena::new(true);
        first.execute(&files, &mut arena).unwrap();
        second.execute(&files, &mut arena).unwrap();
        assert_eq!(arena.stats()["physical_query_evaluations"], 2);
        assert!(arena.stats().get("snapshot_reads").is_none());
        assert!(arena.stats()["logical_steps"].as_u64().unwrap() > 0);
        assert!(arena.stats().get("native_bytes").is_some());
        assert!(arena.stats().get("temporary_bytes").is_some());
        assert_eq!(arena.stats()["residual_invocations"], 2);
        arena.clear_file_results();
        assert_eq!(arena.stats()["physical_query_evaluations"], 2);
    }

    #[test]
    fn runtime_file_limits_are_configurable_below_the_64_mib_ceiling() {
        let value = manifest(json!({}), json!({"x":{"kind":"violation"}}), &["text.v1"]);
        let program = compile(&value, "return;").unwrap();
        let file = SourceFile {
            path: "large.ts".into(),
            text: "x".repeat(5 * 1024 * 1024).into(),
        };
        assert!(program
            .execute(std::slice::from_ref(&file), &mut QueryArena::new(true))
            .is_ok());
        assert!(program
            .execute_with_limits(
                &[file],
                &mut QueryArena::new(true),
                RuntimeLimits {
                    max_file_bytes: 4 * 1024 * 1024,
                    ..RuntimeLimits::default()
                },
            )
            .is_err());
    }

    #[test]
    fn execution_kind_selects_logical_step_budget() {
        let body = (0..3)
            .map(|_| "text::len_bytes(part);")
            .collect::<Vec<_>>()
            .join(" ");
        let file_rule = manifest(json!({}), json!({"x":{"kind":"violation"}}), &["text.v1"]);
        let file_program = compile(
            &file_rule,
            &format!("for part in text::split(file.text, \",\") {{ {body} }}"),
        )
        .unwrap();
        let mut repository_rule = file_rule;
        repository_rule["execution"] = json!("repository");
        let repository_program = compile(
            &repository_rule,
            &format!(
                "for f in repo.files() {{ for part in text::split(f.text, \",\") {{ {body} }} }}"
            ),
        )
        .unwrap();
        let file = SourceFile {
            path: "x".into(),
            text: "x,".repeat(99_999).into(),
        };
        assert!(file_program
            .execute(std::slice::from_ref(&file), &mut QueryArena::new(true))
            .is_err());
        let result = repository_program.execute(&[file], &mut QueryArena::new(true));
        assert!(result.is_ok(), "{}", result.unwrap_err());
    }

    #[test]
    fn text_operations_share_file_local_results_and_charge_each_consumer() {
        let value = manifest(json!({}), json!({"x":{"kind":"violation"}}), &["text.v1"]);
        let program = compile(
            &value,
            r#"
                text::contains(file.text, "needle");
                text::contains(file.text, "needle");
                text::trim(file.text);
                text::trim(file.text);
                text::split(file.text, ",");
                text::split(file.text, ",");
                text::replace(file.text, "needle", "hit");
                text::replace(file.text, "needle", "hit");
                text::lines(file);
                text::lines(file);
                text::len_bytes(file.text);
                text::len_bytes(file.text);
            "#,
        )
        .unwrap();
        let mut arena = QueryArena::new(true);
        program
            .execute(
                &[SourceFile {
                    path: "x".into(),
                    text: "needle,body\n".into(),
                }],
                &mut arena,
            )
            .unwrap();
        assert!(arena.stats()["shared_result_hits"].as_u64().unwrap() >= 6);
        assert!(arena.stats()["native_bytes"].as_u64().unwrap() > 0);
    }

    #[test]
    fn method_and_function_aliases_have_identical_wrl_costs() {
        let manifest = manifest(json!({}), json!({"hit":{"kind":"violation"}}), &["text.v1"]);
        let files = [SourceFile {
            path: "x.txt".into(),
            text: "abc".into(),
        }];
        for source in [
            "let found = file.text.contains(\"b\");",
            "let found = text::contains(file.text, \"b\");",
        ] {
            let program = compile(&manifest, source).unwrap();
            let mut arena = QueryArena::new(true);
            program.execute(&files, &mut arena).unwrap();
            assert_eq!(arena.stats()["native_bytes"], 4);
            assert_eq!(arena.stats()["logical_steps"], 5);
            program.execute(&files, &mut arena).unwrap();
            assert_eq!(arena.stats()["native_bytes"], 8);
            assert_eq!(arena.stats()["logical_steps"], 10);
        }
    }

    #[test]
    fn exponential_concatenation_hits_the_temporary_budget_before_allocation() {
        let value = manifest(
            json!({"x":"x"}),
            json!({"x":{"kind":"violation"}}),
            &["text.v1"],
        );
        let program = compile(
            &value,
            "let value = file.text; for m in rx::find_all(file, \"x\") { value = value + value; }",
        )
        .unwrap();
        let source = SourceFile {
            path: "large.ts".into(),
            text: "x".repeat(40).into(),
        };
        let mut arena = QueryArena::new(true);
        assert!(program.execute(&[source], &mut arena).is_err());
        assert!(arena.stats()["temporary_bytes"].as_u64().unwrap() <= 64 * 1024 * 1024);
    }

    #[test]
    fn rule_ir_records_guards_canonical_aliases_operations_and_only_used_patterns() {
        let value = manifest(
            json!({
                "alias_a": "request\\([^\\r\\n]*\\)",
                "alias_b": "request\\([^\\r\\n]*\\)",
                "unused": "never"
            }),
            json!({"x":{"kind":"violation"}}),
            &["text.v1", "ast.v1"],
        );
        let program = compile(
            &value,
            r#"
                const selected = "alias_a";
                if text::contains(file.text, "ast") {
                    for m in file.ast_match("tsx", "<input/>") {
                        m.node("type");
                    }
                }
                for m in rx::find_all(file, selected) { emit(m.span, "x"); }
                for m in rx::find_all(file, "alias_b") { emit(m.span, "x"); }
                rx::is_match(file, "alias_a");
            "#,
        )
        .unwrap();
        let queries = &program.rule_ir().queries;
        assert!(queries.iter().any(|query| {
            query.operation == "ast.match"
                && query.guards.iter().any(|guard| guard.kind == "if")
                && query.guards.iter().any(|guard| guard.kind == "for")
        }));
        assert!(queries.iter().any(|query| query.operation == "ast.node"));
        let finds = queries
            .iter()
            .filter(|query| query.operation == "regex.find_all")
            .collect::<Vec<_>>();
        assert_eq!(finds.len(), 2);
        assert_eq!(finds[0].canonical_identity, finds[1].canonical_identity);
        assert_ne!(
            finds[0].pattern_alias, finds[1].pattern_alias,
            "local aliases remain visible while canonical identity is shared"
        );
        let existence = queries
            .iter()
            .find(|query| query.operation == "regex.is_match")
            .unwrap();
        assert_ne!(finds[0].canonical_identity, existence.canonical_identity);
        assert!(queries
            .iter()
            .all(|query| query.pattern_expression.as_deref() != Some("never")));
        assert!(queries.iter().any(|query| {
            query.pattern_alias.as_deref() == Some("alias_a")
                && query.resolved_string_constants.get("selected") == Some(&"alias_a".into())
        }));
        assert!(program.plan()["ir"]["queries"].is_array());
    }

    #[test]
    fn compiled_patterns_are_interned_by_expanded_semantics() {
        let definitions = json!({
            "left": "same",
            "right": {"regex": "same", "flags": {"case_insensitive": false}},
            "different": {"regex": "same", "flags": {"case_insensitive": true}}
        });
        let patterns = parse_patterns(Some(&definitions)).unwrap();
        assert!(Arc::ptr_eq(
            &patterns["left"].regex,
            &patterns["right"].regex
        ));
        assert!(!Arc::ptr_eq(
            &patterns["left"].regex,
            &patterns["different"].regex
        ));

        let program = compile(
            &manifest(definitions, json!({"x":{"kind":"violation"}}), &["text.v1"]),
            "rx::is_match(file, \"left\");",
        )
        .unwrap();
        let (_, reservations) = program.compiled_reservation();
        assert_eq!(reservations.len(), 2);
        assert!(reservations.iter().all(|(_, bytes)| *bytes == 1024 * 1024));
    }
}
