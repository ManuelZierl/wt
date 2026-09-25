---
name: wt
description: Use when creating, inspecting, validating, testing, checking, updating, or enforcing Watchtower (WT) rules and configuration, including WRL1 detectors, fixtures, modes, coverage, and trusted CI.
---

# Watchtower (WT)

WT is deterministic, local repository knowledge, not an LLM reviewer: it never calls a model or decides whether a convention is correct. The agent authors and reviews the rule; WT validates and executes it. `watchtower` is the alternate executable name. Run `wt capabilities --format json` before authoring and verify this skill against its reported schema/guide digests; if neither `wt` nor `watchtower` is installed, report that instead of attempting checks.

## Choose protection, then author

Prefer an existing linter, regression test, type restriction, or API redesign when it preserves the lesson more reliably — a WT rule is not required for every fix. A `violation` is a prohibited source form; a `review` is a form worth a human's judgment, and an acceptable occurrence of it is still a raw-positive finding, not a reason to narrow the detector. Use `text.v1` (`rx::find_all`, path globs) for text/regex patterns, and add `ast.v1` (`file.ast_match(language, pattern)`, `matched.node("NAME")`) for a structural shape — a call, a loop body, an element — so formatting and string/comment text cannot cause false matches. `wt guide language` has the full API and language list; combine both freely.

## Author, validate, test, check

```bash
echo "This note is complete." > notes.txt
wt new --stdin --format json < references/minimal-submission.json
wt check --submission references/minimal-submission.json --format json
wt check --format json
```

Start from [`references/minimal-submission.json`](references/minimal-submission.json) (`text.v1`) or [`references/minimal-ast-submission.json`](references/minimal-ast-submission.json) (`ast.v1`); each has a positive and a negative fixture, required for an enforced rule. Inspect JSON, not just the exit code: `complete: true`, no `errors`, no relevant `gaps`. Exit `1` is blocking findings; exit `2` is invalid input, a runtime failure, or an analysis gap. `wt guide author` has the schema, modes, coverage, and `--expect-hash` update details.

## Rule hygiene: `wt stats`

Before adding many rules, or when a check gets noisy, run `wt stats --format json`: it reruns a check and reports each rule's derived signal — `dead`, `noisy`, `useful`, `active`, `disabled`, or `unknown` (incomplete check). `wt guide stats` has exact definitions.

## Recording a review decision

A retained `TODO` trips the rule above and blocks:

<!-- docs-test: exit=1 -->
```bash
echo "TODO: revisit before ship" > notes.txt
wt check --format json
```

If it's deliberate, record why instead of deleting the check:

```bash
FINDING_ID=$(wt check --format json | jq -r '.diagnostics[0].finding_id')
EVIDENCE=$(wt check --format json | jq -r '.diagnostics[0].evidence_digest')
echo "Documented as intentional during this migration." > rationale.md
wt review "$FINDING_ID" --decision accepted-risk --expect-evidence "$EVIDENCE" --reason-file rationale.md
```

One decision per occurrence, bound to the reviewed evidence; it reopens when the owning file, rule, or a watched file changes. `acceptable` (only for `review` findings) never silences a `violation` — `accepted-risk` above is the explicit, authorized way to keep one. `wt inspect FINDING_ID` shows why a decision went stale. `wt guide review` has the full contract.

## Rules for agents

- Never weaken a rule's pattern/scope, remove a fixture, or bulk-accept findings just to make a check pass. Fix the detector when it recognizes the wrong pattern; record a review decision for a correctly recognized acceptable occurrence.
- Never disable a rule to silence it; change its mode explicitly with `wt set-mode ID ... --reason TEXT`. The final CI invocation, `.wt/` policy, and checker binary need a trusted boundary the editing agent does not control.

See `wt guide author|review|language|stats` for detail.
