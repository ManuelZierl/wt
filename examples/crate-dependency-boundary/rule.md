# The app crate may depend only on the shared crate

## Description

`crates/app` may depend on exactly one workspace crate, `shared`, and nothing
else: no other workspace crate and no external (crates.io) crate, in any of
`[dependencies]`, `[dev-dependencies]`, `[build-dependencies]`, or a
target-qualified table such as `[target.'cfg(windows)'.dependencies]`. This is
the simplest version of a dependency-boundary invariant: an allowlist of
exactly one name, checked against every dependency table Cargo recognizes.

Uses `toml.v1` to parse `crates/app/Cargo.toml` into a structured document
(`file.toml()`) instead of a line-by-line regex over the manifest text. For
each dependency table found, every entry's actual package name is resolved
before comparing it to `shared`:

- A plain entry (`serde = "1.0"` or `serde = { version = "1.0" }`) is named by
  its own key.
- A **renamed** entry (`alias = { package = "forbidden-crate", version = "1" }`)
  is named by its `package` sub-key, not its key — matching only the key would
  silently miss the actual dependency (the motivating gap in a regex-based
  version of this rule: an aliased import escapes a pattern anchored on the
  declared name).
- A **dotted-key** entry (`forbidden.workspace = true` on one line and
  `forbidden.path = ".."` on another) is, structurally, the same kind of table
  value as an inline table (`{ workspace = true, path = ".." }`); `toml.v1`
  represents both identically, so no special-casing is needed for either form.

Each finding points at the most specific span available: the `package`
sub-key's value span for a renamed dependency, or the entry's own key span
otherwise.

## Rationale

Cargo dependency syntax has several ways to express the same fact (a bare
version string, an inline table, a multi-line table via dotted keys, a
`package = "..."` rename) and several tables it can appear in (normal, dev,
build, and per-target). A rule tracking this boundary by scanning manifest
text line-by-line has to reimplement enough of TOML's grammar to get all of
these right, and a rename in particular defeats any check anchored on the
key name alone. Parsing the manifest structurally with `toml.v1` means every
form is the same shape (a table entry, resolved to its actual package name)
by the time the rule's logic runs.

## Limitations

- The allowlist is a single name (`shared`), inlined directly in `check.wt`;
  a workspace with more legitimate shared crates would need more repeated
  table/entry-scanning blocks (WRL1 has no user-defined functions, arrays, or
  a way to iterate an authored list of names) or per-crate rule copies.
- Only `crates/app/Cargo.toml` is in scope; a workspace with several
  similarly-constrained crates needs one rule (or scope entry) per manifest.
- A malformed `Cargo.toml` (one that does not parse cleanly as TOML) is an
  analysis gap for this rule, exit `2`, never a silent "no dependencies
  found" pass — see `crates/wt-runtime`'s `toml_data` module tests for that
  behavior; `tests.json` has no way to express an expected gap, so it is not
  one of this package's fixture cases.
