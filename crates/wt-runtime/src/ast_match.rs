//! `ast.v1`: structural matching backed by ast-grep's Rust engine
//! (`ast-grep-core` + `ast-grep-language`), linked directly (no subprocess).
//!
//! Grammar and engine versions are part of runtime semantics: they are
//! reported by `wt capabilities` and, because the whole compiled crate
//! (including this file and `Cargo.lock`) is folded into the cache/evidence
//! digest (see `wt-core::cache::compiled_semantic_identity`), a grammar
//! update automatically reopens affected review decisions.

use anyhow::{anyhow, bail, Result};
use ast_grep_core::meta_var::MetaVariable;
use ast_grep_core::tree_sitter::StrDoc;
use ast_grep_core::{AstGrep, Pattern};
use ast_grep_language::SupportLang;

/// Identity of the structural matching engine itself, independent of any
/// single grammar. Exposed by `wt capabilities`.
pub const AST_ENGINE: &str = "ast-grep-core-0.45.3";

pub const MAX_PATTERN_BYTES: usize = 8 * 1024;
pub const MAX_AST_MATCHES: usize = 10_000;

/// The languages `ast.v1` can target. Each is gated behind its own Cargo
/// feature (`lang-python`, `lang-javascript`, `lang-typescript`, `lang-rust`)
/// so a build can drop grammars it does not need; all four are default-on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AstLanguage {
    Python,
    JavaScript,
    TypeScript,
    Tsx,
    Rust,
}

impl AstLanguage {
    /// Parse a rule's static language literal. Returns `None` both for
    /// languages wt never supports and for ones compiled out of this build;
    /// callers turn that into a validation error, never a panic.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            #[cfg(feature = "lang-python")]
            "python" => Some(Self::Python),
            #[cfg(feature = "lang-javascript")]
            "javascript" => Some(Self::JavaScript),
            #[cfg(feature = "lang-typescript")]
            "typescript" => Some(Self::TypeScript),
            #[cfg(feature = "lang-typescript")]
            "tsx" => Some(Self::Tsx),
            #[cfg(feature = "lang-rust")]
            "rust" => Some(Self::Rust),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Python => "python",
            Self::JavaScript => "javascript",
            Self::TypeScript => "typescript",
            Self::Tsx => "tsx",
            Self::Rust => "rust",
        }
    }

    /// The grammar crate identity behind this language, part of runtime
    /// semantics per the ast.v1 contract.
    pub fn grammar_version(self) -> &'static str {
        match self {
            Self::Python => "tree-sitter-python-0.25.0",
            Self::JavaScript => "tree-sitter-javascript-0.25.0",
            Self::TypeScript | Self::Tsx => "tree-sitter-typescript-0.23.2",
            Self::Rust => "tree-sitter-rust-0.24.2",
        }
    }

    fn support(self) -> SupportLang {
        match self {
            Self::Python => SupportLang::Python,
            Self::JavaScript => SupportLang::JavaScript,
            Self::TypeScript => SupportLang::TypeScript,
            Self::Tsx => SupportLang::Tsx,
            Self::Rust => SupportLang::Rust,
        }
    }

    const ALL: &'static [Self] = &[
        Self::Python,
        Self::JavaScript,
        Self::TypeScript,
        Self::Tsx,
        Self::Rust,
    ];
}

/// Languages this build actually supports, sorted, for `wt capabilities`.
pub fn supported_languages() -> Vec<&'static str> {
    let mut names: Vec<&'static str> = AstLanguage::ALL
        .iter()
        .filter(|lang| AstLanguage::parse(lang.name()).is_some())
        .map(|lang| lang.name())
        .collect();
    names.sort_unstable();
    names
}

/// (language name, grammar crate identity) for every language this build
/// supports, sorted by language name, for `wt capabilities`. Grammar
/// versions are part of ast.v1 runtime semantics: a grammar update can
/// change matches, so this is reported and (via the compiled-source +
/// `Cargo.lock` cache/evidence digest) reopens affected review decisions.
pub fn supported_language_grammars() -> Vec<(&'static str, &'static str)> {
    let mut grammars: Vec<(&'static str, &'static str)> = AstLanguage::ALL
        .iter()
        .filter_map(|lang| {
            AstLanguage::parse(lang.name()).map(|lang| (lang.name(), lang.grammar_version()))
        })
        .collect();
    grammars.sort_unstable();
    grammars
}

pub type Tree = AstGrep<StrDoc<SupportLang>>;

/// Compile a static pattern literal against a language, eagerly, at rule
/// compile time. Errors here are rule-authoring mistakes (WT100), not
/// runtime/analysis conditions.
pub fn compile_pattern(language: AstLanguage, pattern: &str) -> Result<Pattern> {
    if pattern.is_empty() {
        bail!("WT100 ast.v1 pattern must not be empty");
    }
    if pattern.len() > MAX_PATTERN_BYTES {
        bail!("WT102 ast.v1 pattern exceeds {MAX_PATTERN_BYTES} bytes");
    }
    let compiled = Pattern::try_new(pattern, language.support()).map_err(|error| {
        anyhow!(
            "WT100 invalid ast.v1 pattern for language {:?}: {error}",
            language.name()
        )
    })?;
    if compiled.has_error() {
        bail!(
            "WT100 ast.v1 pattern for language {:?} does not parse cleanly in that grammar",
            language.name()
        );
    }
    Ok(compiled)
}

/// Compile a contextual pattern eagerly, at rule compile time: `context` is a
/// standalone snippet that must parse without error nodes in `language`, and
/// `selector` is the tree-sitter node kind inside it that becomes the actual
/// pattern (ast-grep's "contextual pattern"). This is how `ast.v1` matches a
/// node kind that cannot stand alone as a bare pattern — a Rust `match` arm, a
/// struct field, an attribute, a function parameter — by writing it inside a
/// minimal enclosing shape and naming the node kind to extract. Errors here
/// are rule-authoring mistakes (WT100), not runtime/analysis conditions.
pub fn compile_context_pattern(
    language: AstLanguage,
    context: &str,
    selector: &str,
) -> Result<Pattern> {
    if context.is_empty() || selector.is_empty() {
        bail!("WT100 ast.v1 contextual pattern requires a non-empty context and selector");
    }
    if context.len() > MAX_PATTERN_BYTES || selector.len() > MAX_PATTERN_BYTES {
        bail!("WT102 ast.v1 contextual pattern exceeds {MAX_PATTERN_BYTES} bytes");
    }
    let compiled = Pattern::contextual(context, selector, language.support()).map_err(|error| {
        anyhow!(
            "WT100 invalid ast.v1 contextual pattern for language {:?}, selector {selector:?}: {error}",
            language.name()
        )
    })?;
    if compiled.has_error() {
        bail!(
            "WT100 ast.v1 contextual pattern for language {:?} does not parse cleanly in that grammar",
            language.name()
        );
    }
    Ok(compiled)
}

/// Parse a source file. Never panics: tree-sitter recovers from syntax
/// errors instead of failing outright, so this only errs for degenerate
/// conditions (e.g. no parser assigned); ordinary syntax errors surface
/// later via `has_parse_error`, as an analysis gap, not here.
pub fn parse_source(language: AstLanguage, source: &str) -> Result<Tree> {
    AstGrep::try_new(source, language.support())
        .map_err(|error| anyhow!("ast.v1 parser did not produce a tree: {error}"))
}

/// True when the parsed tree contains any tree-sitter error/missing node.
/// Per the ast.v1 contract, any error node means the file is an analysis gap
/// for ast.v1 rules on that file, never a silent no-match.
pub fn has_parse_error(tree: &Tree) -> bool {
    tree.root().get_inner_node().has_error()
}

/// One structural match: the whole match's byte span, plus the byte span of
/// every named metavariable capture (`$X` and `$$$X`; a multi-capture spans
/// from the first to the last captured node).
pub struct AstMatch {
    pub start: usize,
    pub end: usize,
    pub captures: Vec<(String, usize, usize)>,
}

/// Search a tree for `pattern`, optionally scoped to the smallest node that
/// fully contains `scope` (a byte range from an earlier match/capture). This
/// is how nested search ("X inside Y") is expressed: no separate relational
/// operator, the caller re-searches within a captured span.
pub fn find_matches(
    tree: &Tree,
    pattern: &Pattern,
    scope: Option<(usize, usize)>,
) -> Result<Vec<AstMatch>> {
    let root = tree.root();
    let search_root = match scope {
        None => root,
        Some((start, end)) => {
            let inner = root.get_inner_node();
            match inner.descendant_for_byte_range(start, end) {
                Some(descendant) => tree.adopt(descendant),
                None => return Ok(Vec::new()),
            }
        }
    };
    let mut results = Vec::new();
    for node_match in search_root.find_all(pattern) {
        if results.len() >= MAX_AST_MATCHES {
            bail!("ast.v1 match limit exceeded");
        }
        let range = node_match.range();
        let mut captures = Vec::new();
        for variable in node_match.get_env().get_matched_variables() {
            match variable {
                MetaVariable::Capture(name, _) => {
                    if let Some(node) = node_match.get_env().get_match(&name) {
                        let node_range = node.range();
                        captures.push((name, node_range.start, node_range.end));
                    }
                }
                MetaVariable::MultiCapture(name) => {
                    let nodes = node_match.get_env().get_multiple_matches(&name);
                    if let (Some(first), Some(last)) = (nodes.first(), nodes.last()) {
                        captures.push((name, first.range().start, last.range().end));
                    }
                }
                MetaVariable::Dropped(_) | MetaVariable::Multiple => {}
            }
        }
        results.push(AstMatch {
            start: range.start,
            end: range.end,
            captures,
        });
    }
    Ok(results)
}
