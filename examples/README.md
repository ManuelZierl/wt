# Examples

`shared-request-foo.json` and `shared-request-foo123.json` are small self-contained submissions used to demonstrate independent predicates sharing one native regex query.

`template-number-decimal-step.json` is the retained self-contained decimal rule submission referenced by `watchtower-spec-v3.md` section 12. Its editable disk package is under [`template-number-decimal-step/`](template-number-decimal-step/), with `rule.json`, top-level `check.wt`, `tests.json`, and retained fixture files. The package includes positive findings, accepted counterexamples, review cases, formatting/attribute-order variants, and a separate invalid-TSX analysis-error vector.

The decimal manifest declares `text.v1` and `jsx.v1`. The schemas still accept `regex.v1` for legacy packages, but the reference submission does not require it.

`review-legacy-endpoint.json` is a schema-3 raw-positive review-trigger submission.
`second-encounter.scenarios.json` lists its later-work transitions without
inventing hashes; the CLI conformance test derives evidence and review IDs from
the actual executable. See [the experiment protocol](../docs/second-encounter-experiment.md).
