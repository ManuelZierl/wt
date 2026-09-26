# Examples

`shared-request-foo.json` and `shared-request-foo123.json` are small self-contained submissions used to demonstrate independent predicates sharing one native regex query.

`template-number-decimal-step.json` is a self-contained decimal-step rule submission (see [`docs/spec.md`](../docs/spec.md#15-worked-examples)). Its editable disk package is under [`template-number-decimal-step/`](template-number-decimal-step/), with `rule.json`, top-level `check.wt`, `tests.json`, and retained fixture files. The package includes positive findings, accepted counterexamples, review cases, a formatting/attribute-order variant, and a separate invalid-TSX analysis-gap vector. It declares `ast.v1` and matches TSX structurally with `file.ast_match(...)`.

[`orm-query-in-loop/`](orm-query-in-loop/) is a `python` `ast.v1` rule: it flags an ORM-style `.objects.<method>(...)` call found inside a `for` loop body (a possible N+1 query), with a positive fixture, a counterexample where the same call sits outside the loop, and a counterexample where the matching text only appears in a string or a comment. This is the rule the README's quickstart walks through.

[`raw-sql-in-loop/`](raw-sql-in-loop/) combines both capabilities: `ast.v1` finds each `for` loop and gives the byte span of its body, then `text.v1`'s `rx::find_in` regex-searches only inside that span for a raw `SELECT ... FROM` string.

[`match-arm-wrong-handler/`](match-arm-wrong-handler/) is a `rust` `ast.v1` rule using `file.ast_match_context(...)`: a bare `Action::Copy => $BODY` is not a complete node on its own, so the rule wraps it in a minimal `match` and selects the `match_arm` node kind out of it, then checks the captured body for a nested call to `self.copy_document(...)`.

[`blocking-call-outside-spawn/`](blocking-call-outside-spawn/) is a `rust` `ast.v1` rule using `span.contains`: it flags a `completion::suggest(...)` call unless some `thread::spawn(...)` call's span encloses it, with fixtures for a top-level call, a call inside the spawn closure, one inside a nested block within it, and text that only looks like a call in a string or comment.

[`crate-dependency-boundary/`](crate-dependency-boundary/) is a `toml.v1` rule: `crates/app/Cargo.toml` may depend only on the workspace's `shared` crate. `file.toml()` parses the manifest structurally, so a renamed dependency (`alias = { package = "forbidden-crate", ... }`), a dotted-key table (`forbidden.workspace = true`), and a plain inline table are all resolved to the same thing — an entry's actual package name — instead of a line-by-line regex that a rename or a dependency sub-table can slip past. Fixtures cover `[dependencies]`, `[dev-dependencies]`, `[build-dependencies]`, a target-qualified table, and a manifest that does not parse as TOML at all (an analysis gap, exercised as a direct unit test since `tests.json` cannot express one).

`review-legacy-endpoint.json` is a raw-positive review-trigger submission.
`second-encounter.scenarios.json` lists its later-work transitions (unchanged
recheck, a new copy, a watched-dependency edit, an owner edit, a missing
occurrence) without inventing hashes; a CLI conformance test derives evidence
and review IDs from the actual executable and drives the review lifecycle
through each transition. See [`docs/readable-rules-and-reviews.md`](../docs/readable-rules-and-reviews.md) for the review-decision contract these scenarios exercise.
