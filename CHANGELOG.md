# Changelog

## v0.0.2

Existing rules and review decisions from v0.0.1 keep working unchanged. Upgrading does not reopen any review decision.

### New

- `toml.v1` capability: `file.toml()` parses TOML into a tree with key and value spans (`get`, `entries`, `items`, `kind`, `text`, `span`). A file that fails to parse is an analysis gap. Example: [`examples/crate-dependency-boundary/`](examples/crate-dependency-boundary/).
- `ast.v1` contextual patterns: `file.ast_match_context(language, context, selector)` (also on a match) matches nodes that are not valid standalone code, such as match arms, struct fields, attributes and parameters. Example: [`examples/match-arm-wrong-handler/`](examples/match-arm-wrong-handler/).
- `span::contains(outer, inner)` for "X inside / not inside Y" checks. Example: [`examples/blocking-call-outside-spawn/`](examples/blocking-call-outside-spawn/).
- Optional rule field `"intent": "watch"` for rules whose job is to force re-review whenever specific code changes. `wt stats` labels them `watch`, never `noisy`.
- `wt stats` label `quiet`: the scope matches files and there are no current findings. `wt stats` also reports `scoped_files` and include globs that match nothing (`unmatched_include`).
- `wt validate` hints (`hint.capability`, non-blocking) when a `text.v1` rule targets only files that `ast.v1` or `toml.v1` could parse.
- Commands that take a finding or evidence hash (`wt inspect`, `wt review`, `wt reviews`) accept unique prefixes of at least 8 hex characters.

### Changed

- `wt check` text output is rustc-style: `warning[rule]: message`, `--> path:line:col`, a source snippet with a caret, and a status (`needs review`, `confirmed issue`, `violation`). Findings with a current `confirmed_issue` decision appear as one-liners under "Known issues". The summary line reads like `2 known issues · 0 need review · 0 blocking · 12 accepted · 143 files in 0.1s`. Hashes are shown as short prefixes. Output is colored on a TTY and respects `NO_COLOR`. JSON output only gained fields (`status`, `finding_prefix`, `evidence_prefix`, `snippet`, `summary.elapsed_ms`, `summary.known_issues`).
- `wt stats` `dead` now means the scope matches no files. Rules with zero findings are `quiet`, not `dead`.
- Review evidence is bound to an explicit engine identity: a WRL1 semantics epoch plus the exact versions of the dependencies that affect matching (regex, globset, rhai, ast-grep, tree-sitter grammars, and the TOML parser for `toml.v1` rules). A dependency or grammar bump still reopens decisions automatically. A wt release that does not change rule semantics no longer does.
- `wt guide author` and the agent skill say which capability to use: `ast.v1` for code in supported languages, `toml.v1` for TOML, `text.v1` for prose, comments, strings and other file types.

### Fixed

- `wt stats` no longer labels working regression guards `dead`.
- `wt check` text output no longer shows known confirmed issues the same way as new, unreviewed findings.
- `wt reviews`, `wt inspect` and `wt review` have text output instead of raw JSON.
