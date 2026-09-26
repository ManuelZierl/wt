//! The review-evidence engine identity (`engine_digest`).
//!
//! This is deliberately narrower than `cache::compiled_semantic_identity`,
//! which hashes the whole compiled wt-runtime source plus `Cargo.lock` and
//! the toolchain file, and exists purely to invalidate detection/fixture
//! *caches* on any code change (correct: a stale cache must never survive a
//! rebuild). Binding review evidence to that same all-of-the-compiler hash
//! would mean every wt release -- even a version bump that touches nothing
//! semantic -- reopens every review decision in every user repository.
//!
//! Instead, review evidence binds to an explicit, maintainer-controlled
//! semantic identity with two independent knobs:
//!
//!   - [`WRL1_SEMANTICS_EPOCH`]: bump this by hand when a wt-runtime source
//!     change (see [`RUNTIME_SOURCE_FINGERPRINT`] and its tripwire test)
//!     could make an existing rule produce different findings or spans.
//!     Bumping it reopens every review decision everywhere -- that is the
//!     point, and it is the only way this identity changes without a
//!     dependency version changing too.
//!   - the exact pinned versions, read from this build's `Cargo.lock`, of
//!     the dependencies that actually implement matching/scoping semantics.
//!     A dependency bump (say, a tree-sitter grammar update) changes the
//!     identity automatically, with no epoch bump and no code review of
//!     wt-runtime required to notice it happened.
//!
//! Capabilities are folded in per rule (see [`review_engine_identity`]): the
//! base identity below covers every capability that existed in the wt 0.0.1
//! release (`text.v1`, `path.v1`, `repo.v1`, `ast.v1` -- none of which need a
//! component beyond the base list, since `repo.v1`/`path.v1` reuse `globset`
//! and `text.v1` reuses `regex`, already tracked). A capability added later
//! that depends on its own semantics-relevant crate folds that crate's
//! pinned version in too, but only for rules that declare it; see
//! [`CAPABILITY_COMPONENTS`].
//!
//! Finally, for continuity across the v0.0.2 release: wt 0.0.1 shipped epoch
//! 1 with exactly [`V0_0_1_COMPONENT_VERSIONS`] and no capability
//! extensions. As long as that stays true, the emitted identity for base
//! capabilities is the frozen v0.0.1 constant itself, not a freshly computed
//! digest -- so the 14 existing Kea decisions (and everyone else's) stay
//! current. See [`legacy_alias_applies`].

use crate::digest::semantic_digest;
use std::sync::OnceLock;

/// Bump by hand when a wt-runtime source change could change what an
/// existing rule finds or where it reports a span. This reopens every
/// review decision in every user repository -- a dependency version change
/// does not need this; it changes the identity by itself (see
/// [`base_component_versions`]).
pub const WRL1_SEMANTICS_EPOCH: u64 = 1;

/// Hash of the wt-runtime source files that implement rule semantics,
/// checked against a fresh computation by
/// `runtime_source_fingerprint_tripwire` in `cargo test`. Kept next to the
/// epoch deliberately: a mismatch is the prompt to decide whether the epoch
/// needs to move.
///
/// Update by running `cargo test -p wt-core runtime_source_fingerprint` and
/// pasting the "actual" value the failure prints.
#[cfg(test)]
const RUNTIME_SOURCE_FINGERPRINT: &str =
    "sha256:794114ba05019153ecaa94ee78addb3ad71ed386d82b49e3009c2b230fe5b1a6";

/// wt-runtime source files that implement rule semantics: the interpreter
/// and its builtins (`lib.rs`), `ast.v1`'s matcher/grammar mapping
/// (`ast_match.rs`), `toml.v1`'s parser/tree mapping (`toml_data.rs`), and
/// the shared text/digest representation all of them operate on
/// (`shared_text.rs`). `performance_tests.rs` is excluded: it is
/// a `#[cfg(test)]`-only module (see `crates/wt-runtime/src/lib.rs`) that
/// cannot affect what a real build reports. Only ever read by the tripwire
/// test below, so this (and the fingerprint above) stay test-only.
#[cfg(test)]
const RUNTIME_SOURCE_FILES: &[(&str, &str)] = &[
    ("lib.rs", include_str!("../../wt-runtime/src/lib.rs")),
    (
        "ast_match.rs",
        include_str!("../../wt-runtime/src/ast_match.rs"),
    ),
    (
        "toml_data.rs",
        include_str!("../../wt-runtime/src/toml_data.rs"),
    ),
    (
        "shared_text.rs",
        include_str!("../../wt-runtime/src/shared_text.rs"),
    ),
];

#[cfg(test)]
fn runtime_source_fingerprint() -> String {
    let mut parts = Vec::with_capacity(RUNTIME_SOURCE_FILES.len() * 2);
    for (name, contents) in RUNTIME_SOURCE_FILES {
        parts.push(name.as_bytes());
        parts.push(contents.as_bytes());
    }
    semantic_digest("WT-RUNTIME-SOURCE-1", &parts)
}

/// This build's `Cargo.lock`, parsed once for the pinned versions of the
/// dependencies that matter to review-evidence identity.
const CARGO_LOCK: &str = include_str!("../../../Cargo.lock");

/// Dependencies whose pinned version affects matching/scoping semantics for
/// every rule, regardless of declared capability:
///   - `rhai`: the WRL1 interpreter every rule executes under.
///   - `regex`/`regex-syntax`/`regex-automata`: `text.v1`'s pattern language
///     and match spans (`regex-syntax` parses patterns, `regex-automata` is
///     the matching engine regex is built on).
///   - `globset`: `path.v1` path predicates and every rule's `scope` globs.
///   - `ast-grep-core`/`ast-grep-language`/`tree-sitter`: `ast.v1`'s
///     matching engine and the parser core every grammar builds on.
///
/// `repo.v1` needs no component of its own: it is repository-scoped
/// iteration built on the same `text.v1`/`path.v1`/`globset` primitives
/// already listed.
const BASE_COMPONENTS: &[&str] = &[
    "rhai",
    "regex",
    "regex-syntax",
    "regex-automata",
    "globset",
    "ast-grep-core",
    "ast-grep-language",
    "tree-sitter",
];

/// Per-language `ast.v1` grammar crates (see `crates/wt-runtime/Cargo.toml`,
/// each gated by its own feature). Looked up by name rather than assumed
/// present, so a build without a given language feature simply omits it --
/// it cannot produce `ast.v1` findings for that language either.
const AST_GRAMMAR_COMPONENTS: &[&str] = &[
    "tree-sitter-python",
    "tree-sitter-javascript",
    "tree-sitter-typescript",
    "tree-sitter-rust",
];

/// Extension point: a capability beyond the v0.0.1 set that depends on its
/// own semantics-relevant crate lists it here, mapped to the crate name(s)
/// whose pinned version should fold into the identity for rules that
/// declare that capability. Rules that don't declare it are unaffected --
/// they keep the base identity.
///
/// `toml.v1` is backed by `toml-span` (see `crates/wt-runtime/src/toml_data.rs`
/// and its `TOML_PARSER_CRATE` constant); a parser version bump changes
/// `toml.v1` matching/span semantics, so it folds in here. `smallvec` is
/// `toml-span`'s own only dependency, but it was already present in this
/// workspace's `Cargo.lock` before `toml.v1` existed (pulled in transitively
/// elsewhere) and is a generic small-vector container, not TOML-parsing
/// semantics -- it is deliberately left out.
const CAPABILITY_COMPONENTS: &[(&str, &[&str])] = &[("toml.v1", &["toml-span"])];

/// The frozen wt 0.0.1 release identity. Verified against the `v0.0.1` tag:
/// `wt check --format json --show-reviewed --root <kea>` built from that tag
/// emits this exact value for every one of Kea's findings.
const V0_0_1_ENGINE_DIGEST: &str =
    "sha256:c79157972c88f5a70affdc5436051cd91907306f3ba997fd11a11486e9a0be63";

/// Component versions pinned by the v0.0.1 release's `Cargo.lock`, in the
/// same order `base_component_versions` produces them. v0.0.1 predates
/// capability extensions, so this is exactly the base list.
const V0_0_1_COMPONENT_VERSIONS: &[&str] = &[
    "1.26.1", // rhai
    "1.12.4", // regex
    "0.8.11", // regex-syntax
    "0.4.18", // regex-automata
    "0.4.20", // globset
    "0.45.3", // ast-grep-core
    "0.45.3", // ast-grep-language
    "0.27.0", // tree-sitter
    "0.25.0", // tree-sitter-python
    "0.25.0", // tree-sitter-javascript
    "0.23.2", // tree-sitter-typescript
    "0.24.2", // tree-sitter-rust
];

/// Compute the review engine identity for a rule declaring `capabilities`.
///
/// Rules whose capabilities are all in the v0.0.1 set get the base identity
/// (the frozen v0.0.1 constant, or a fresh digest once the epoch or a
/// component version has moved beyond it). A rule declaring a capability
/// with an entry in [`CAPABILITY_COMPONENTS`] gets that capability's pinned
/// component(s) folded in as well, so it reopens independently of rules that
/// don't declare it.
pub fn review_engine_identity(capabilities: &[String]) -> String {
    let mut extra: Vec<String> = capabilities
        .iter()
        .filter_map(|capability| {
            CAPABILITY_COMPONENTS
                .iter()
                .find(|(cap, _)| *cap == capability)
                .map(|(cap, crates)| {
                    crates
                        .iter()
                        .map(|crate_name| {
                            let version = lockfile_version(CARGO_LOCK, crate_name)
                                .unwrap_or_else(|| "unknown".to_owned());
                            format!("{cap}={crate_name}@{version}")
                        })
                        .collect::<Vec<_>>()
                })
        })
        .flatten()
        .collect();
    if extra.is_empty() {
        return base_review_engine_identity().to_owned();
    }
    // Sort/dedup so identity depends on which capabilities are declared, not
    // the order code.capabilities happens to list them in.
    extra.sort();
    extra.dedup();
    let base = base_review_engine_identity();
    let mut parts: Vec<&[u8]> = vec![base.as_bytes()];
    let extra_bytes: Vec<Vec<u8>> = extra.iter().map(|part| part.as_bytes().to_vec()).collect();
    parts.extend(extra_bytes.iter().map(Vec::as_slice));
    semantic_digest("WT-ENGINE-2-CAPABILITY", &parts)
}

fn base_review_engine_identity() -> &'static str {
    static IDENTITY: OnceLock<String> = OnceLock::new();
    IDENTITY.get_or_init(|| {
        let versions = base_component_versions();
        if legacy_alias_applies(&versions) {
            V0_0_1_ENGINE_DIGEST.to_owned()
        } else {
            let epoch = WRL1_SEMANTICS_EPOCH.to_be_bytes();
            let mut parts: Vec<&[u8]> = vec![&epoch];
            let named: Vec<Vec<u8>> = base_component_names()
                .zip(versions.iter())
                .map(|(name, version)| format!("{name}@{version}").into_bytes())
                .collect();
            parts.extend(named.iter().map(Vec::as_slice));
            semantic_digest("WT-ENGINE-2", &parts)
        }
    })
}

fn base_component_names() -> impl Iterator<Item = &'static str> {
    BASE_COMPONENTS
        .iter()
        .chain(AST_GRAMMAR_COMPONENTS.iter())
        .copied()
}

/// Resolve every base/grammar component's pinned version from this build's
/// `Cargo.lock`, in `base_component_names()` order. A missing optional
/// grammar crate (feature disabled) is simply omitted.
fn base_component_versions() -> Vec<String> {
    base_component_names()
        .filter_map(|name| lockfile_version(CARGO_LOCK, name))
        .collect()
}

fn legacy_alias_applies(versions: &[String]) -> bool {
    WRL1_SEMANTICS_EPOCH == 1
        && versions.len() == V0_0_1_COMPONENT_VERSIONS.len()
        && versions
            .iter()
            .zip(V0_0_1_COMPONENT_VERSIONS.iter())
            .all(|(actual, expected)| actual == expected)
}

/// Look up `crate_name`'s pinned `version` in a `Cargo.lock` file. This is a
/// narrow, purpose-built parser for `[[package]]` stanzas
/// (`name = "..."` / `version = "..."`), not a general TOML parser --
/// `Cargo.lock`'s format is stable and simple enough not to need one here.
/// Returns the first match; none of the tracked crate names are expected to
/// resolve to more than one version in this workspace.
fn lockfile_version(lockfile: &str, crate_name: &str) -> Option<String> {
    let mut lines = lockfile.lines().peekable();
    while let Some(line) = lines.next() {
        if line.trim() != "[[package]]" {
            continue;
        }
        let mut name = None;
        let mut version = None;
        while let Some(next) = lines.peek() {
            let trimmed = next.trim();
            if trimmed.is_empty() || trimmed == "[[package]]" {
                break;
            }
            lines.next();
            if let Some(value) = trimmed.strip_prefix("name = \"") {
                name = value.strip_suffix('"');
            } else if let Some(value) = trimmed.strip_prefix("version = \"") {
                version = value.strip_suffix('"');
            }
        }
        if name == Some(crate_name) {
            return version.map(str::to_owned);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_source_fingerprint_tripwire() {
        let actual = runtime_source_fingerprint();
        assert_eq!(
            actual, RUNTIME_SOURCE_FINGERPRINT,
            "wt-runtime source changed. If rule semantics changed (existing \
             rules could produce different findings/spans), bump \
             WRL1_SEMANTICS_EPOCH -- this reopens review decisions in user \
             repos. If semantics are unchanged (refactor, additive API), \
             update RUNTIME_SOURCE_FINGERPRINT to the actual value above."
        );
    }

    #[test]
    fn lockfile_version_reads_this_workspaces_pins() {
        assert_eq!(
            lockfile_version(CARGO_LOCK, "ast-grep-core").as_deref(),
            Some("0.45.3")
        );
        assert_eq!(lockfile_version(CARGO_LOCK, "no-such-crate"), None);
    }

    #[test]
    fn base_identity_is_stable_and_capability_independent() {
        let base = review_engine_identity(&["text.v1".to_owned()]);
        assert_eq!(base, review_engine_identity(&["ast.v1".to_owned()]));
        assert_eq!(
            base,
            review_engine_identity(&["text.v1".to_owned(), "ast.v1".to_owned()])
        );
    }

    #[test]
    fn toml_v1_folds_its_parser_version_in_but_only_for_rules_that_declare_it() {
        // A rule that only declares base-set capabilities keeps the frozen
        // v0.0.1 identity untouched by `toml.v1` existing at all.
        let base = review_engine_identity(&["text.v1".to_owned(), "ast.v1".to_owned()]);
        assert_eq!(base, V0_0_1_ENGINE_DIGEST);

        // A rule that declares `toml.v1` gets a different identity, folding
        // in the pinned `toml-span` version -- so a parser upgrade reopens
        // review decisions for `toml.v1` rules without touching every other
        // rule in the repository.
        let with_toml = review_engine_identity(&["toml.v1".to_owned()]);
        assert_ne!(base, with_toml);
        assert_eq!(
            with_toml,
            review_engine_identity(&["text.v1".to_owned(), "toml.v1".to_owned()]),
            "declaring toml.v1 alongside a base capability is the same identity \
             as toml.v1 alone -- the base identity itself does not change"
        );
    }

    #[test]
    fn v0_0_1_versions_currently_match_so_the_legacy_alias_applies() {
        // If this fails, a dependency in BASE_COMPONENTS/AST_GRAMMAR_COMPONENTS
        // has moved since v0.0.1 -- expected eventually, and exactly the
        // "identity changes automatically" behavior this module documents.
        assert_eq!(
            base_review_engine_identity(),
            V0_0_1_ENGINE_DIGEST,
            "component versions no longer match v0.0.1; the base identity \
             should now be a freshly computed WT-ENGINE-2 digest"
        );
    }

    #[test]
    fn capability_without_extra_components_keeps_the_base_identity() {
        assert_eq!(
            review_engine_identity(&["repo.v1".to_owned()]),
            base_review_engine_identity()
        );
    }

    /// Exercises the extension mechanism end to end using a fake capability
    /// table, proving: (1) a rule declaring the extended capability gets a
    /// different identity than the base one, (2) a rule that does not
    /// declare it keeps the base identity untouched, and (3) the result
    /// does not depend on the order capabilities are declared in.
    #[test]
    fn capability_extension_folds_in_only_for_declaring_rules() {
        fn identity_with(capabilities: &[String], extensions: &[(&str, &[&str])]) -> String {
            let mut extra: Vec<String> = capabilities
                .iter()
                .filter_map(|capability| {
                    extensions
                        .iter()
                        .find(|(cap, _)| cap == capability)
                        .map(|(cap, crates)| {
                            crates
                                .iter()
                                .map(|crate_name| {
                                    let version = lockfile_version(CARGO_LOCK, crate_name)
                                        .unwrap_or_else(|| "unknown".to_owned());
                                    format!("{cap}={crate_name}@{version}")
                                })
                                .collect::<Vec<_>>()
                        })
                })
                .flatten()
                .collect();
            if extra.is_empty() {
                return base_review_engine_identity().to_owned();
            }
            extra.sort();
            extra.dedup();
            let base = base_review_engine_identity();
            let mut parts: Vec<&[u8]> = vec![base.as_bytes()];
            let extra_bytes: Vec<Vec<u8>> =
                extra.iter().map(|part| part.as_bytes().to_vec()).collect();
            parts.extend(extra_bytes.iter().map(Vec::as_slice));
            semantic_digest("WT-ENGINE-2-CAPABILITY", &parts)
        }

        let fake_extensions: &[(&str, &[&str])] = &[("toml.v1", &["globset"])];

        let base_only = identity_with(&["text.v1".to_owned()], fake_extensions);
        let with_toml = identity_with(
            &["text.v1".to_owned(), "toml.v1".to_owned()],
            fake_extensions,
        );
        let reordered = identity_with(
            &["toml.v1".to_owned(), "text.v1".to_owned()],
            fake_extensions,
        );

        assert_eq!(base_only, base_review_engine_identity());
        assert_ne!(with_toml, base_only, "declaring the capability changes it");
        assert_eq!(with_toml, reordered, "declaration order does not matter");
    }
}
