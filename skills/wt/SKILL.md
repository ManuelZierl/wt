---
name: wt
description: Use when creating, inspecting, validating, testing, checking, updating, or enforcing Watchtower (WT) rules and configuration, including WRL1 detectors, fixtures, modes, coverage, and trusted CI.
---

# Watchtower (WT)

Use WT for deterministic, local repository knowledge. WT is not an LLM reviewer: it does not call a model, infer a new rule, or decide whether a convention is correct. The agent authors and reviews the rule; WT validates and executes it.

## Contract First

Run `wt capabilities --format json` and `wt guide author` against the installed executable before authoring. Verify the reported schema, protocol and guide digests match the binary you intend to use. `watchtower` is the alternate executable name, useful where `wt` names Windows Terminal. If neither executable is installed, report that prerequisite before attempting checks.

This skill is portable: the repository being checked need not contain WT's own sources. Use the installed `wt guide language` and installed command's schemas. When a WT source checkout is available, its `docs/cli.md`, `docs/runtime-configuration.md`, and `watchtower-spec-v3.md` provide additional detail. Verify a capability against the installed executable before relying on it; this skill may describe a newer version than the binary.

## Inspect Before Editing

Use qualified IDs (`local/name` or `global/name`) when possible. An unqualified ID must resolve uniquely.

```sh
wt config --format json
wt list --format json
wt show local/rule-id --format json
wt validate local/rule-id --format json
wt schema submission
wt schema tests
```

`config` shows configured and invocation-effective values and origins. `list` shows declared/effective modes, scope, test availability, and digests. `show` returns the self-contained rule at `data.rule` with its same-read package hash at `data.digest`. `plan` inspects the compiled plan without scanning application source:

```sh
wt plan --rule local/rule-id --format json
```

Use `--no-global` only when the intended policy is local-only. Otherwise the default combines global and local rules.

## Choose Protection, Then Author

Use an existing linter, behavioral regression test, type restriction, or API redesign when it preserves the lesson more reliably than a WT detector. A detector need not be invented for every fix. When WT is useful, distinguish a source-level recognition claim from the contextual question the reviewer must answer. Acceptable occurrences of a deliberate review pattern stay raw-positive.

Prefer a small, documented `text.v1` WRL1 detector. `check.wt` is a top-level statement body with an implicit read-only `file`; it is not `fn check(file)` and it is not arbitrary Rhai. Rhai is only the current backend for the restricted `wt-rule-1` contract.

`text.v1` supplies the text, regex, path, and repository APIs. Use spans from the original source (`matched.span`, `line.span`, or `file.span`). Request `jsx.v1` only when actual TSX parsing is needed. Never use I/O, environment, network, process, clock, random, dynamic evaluation, imports, or unregistered calls.

A new submission uses `schema_version: 3`, a stable ID, title, severity, a nonempty scope, diagnostics, `documentation.source` containing the Markdown contract/rationale/limitations/evidence, and:

```json
"code": {
  "language": "wt-rule-1",
  "capabilities": ["text.v1"],
  "source": "..."
}
```

Start from the complete runnable example in [`references/minimal-submission.json`](references/minimal-submission.json). It intentionally has a text-only detector, a nonempty positive fixture, and a nonempty negative fixture.

Keep the rule's limitation honest. A `violation` identifies a prohibited form; a `review` identifies a relevant form the detector cannot classify. A review is not permission to call uncertainty a confirmed bug.

## Validate, Create, Test, Check

`new` already validates, formats and executes supplied fixtures. For an installed rule, use `wt check --submission PATH` to scan a self-contained candidate without installing it. `new` accepts exactly one source: positional JSON, `--stdin`, or `--file`.

```sh
wt check --submission submission.json --format json
wt new --stdin --format json < submission.json
wt check --format json
```

`wt test` runs all discovered fixture suites when no ID is supplied. `wt check` runs all enabled rules and all eligible repository files when no path or rule filter is supplied. Inspect JSON, not just the exit code: require `complete: true`, no `errors`, no relevant `gaps`, and `scope.partial: false` for a full check. A complete check with blocking diagnostics exits `1`; invalid configuration, execution failures, and analysis gaps exit `2`.

Every expected fixture diagnostic must match; extra findings fail the case. Keep the original regression, accepted alternatives, meaningful edge cases, and false-positive counterexamples. Do not replace a positive with an empty or no-op negative: enforced rules require at least one expected finding and one applicable nonempty valid input with no findings.

Use `--optimizer off --no-cache` for the unshared correctness reference path when comparing execution behavior. Add `--stats` to inspect measured work. Keep operational counters separate when comparing the stable findings and coverage of reference, optimized, cached, and parallel runs.

## Modes, Severity, and Coverage

- `advisory`: executes, but findings do not block unless `--strict` is used.
- `enforced`: executes and findings block.
- `disabled`: does not execute and remains visible in inspection.
- `severity` (`error`, `warning`, `info`) labels importance; it does not determine blocking.
- `review` diagnostics block under enforced mode just like violations.
- Runtime or fixture failures and analysis gaps are incomplete results, not no-match results.

`--changed`, `--rule`, candidate previews and narrowed paths are partial coverage. `coverage.expectations` require each selected enabled and applicable qualified rule to complete the declared minimum number of eligible files in a full check. `--allow-empty` never bypasses an expectation. `--include-ignored` changes only Git-ignore filtering. `--show-reviewed` expands human output.

## Update Safely

Use the JSON `show` response's `data.rule` field as the self-contained submission, retaining all existing tests and fixture content. Do not submit the surrounding command envelope. Edit the submission and use `data.digest` from that same `show` response to preview and then update atomically:

```sh
wt list --format json
wt update local/rule-id --stdin --expect-hash sha256:... --preview --format json < updated-submission.json
wt update local/rule-id --stdin --expect-hash sha256:... --format json < updated-submission.json
```

The submission ID must stay the same. A stale hash must fail rather than overwrite a concurrent change. Test-case removal or replacement requires both `--allow-test-removal` and a nonempty `--reason`; prefer retaining fixtures instead.

Change mode explicitly and with a reason. Moving to `enforced` validates and runs the required positive and nonempty negative fixtures:

```sh
wt set-mode local/rule-id enforced --reason "Reviewed regression and counterexample" --format json
```

## Trusted CI

The final CI policy and invocation must be outside the editing agent's control. CI should protect the approved rule/configuration files, use the intended full-scope command, and fail closed on incomplete results. Keep rule packages and local policy in version control; treat caches as derived data. A model may help author or review a rule externally, but WT itself must remain deterministic and model-free.

```sh
wt check --no-global --no-host-ignores --no-cache --format json
```

Report the command, rule IDs, fixture outcome, full/partial coverage, findings, and analysis failures. Do not disable a rule, weaken its scope, or remove examples merely to make a check pass. A detector-recognition error belongs in the detector and fixtures. An acceptable occurrence of an intended review pattern belongs in an explicit evidence-bound occurrence review; do not narrow a detector just to remove it. An intentionally accepted violation requires separately justified accepted-risk authorization.

## Readable Rules And Occurrence Decisions

`new` and `update` automatically format `check.wt` and store long-form prose only
in `rule.md`. Use `wt fmt --check` for existing local scripts. `wt check` never
reformats source.

A `review` finding means **inspect this pattern**, not **this is a proven bug**.
An acceptable occurrence is still a raw positive. Keep that detector fixture.
After actual contextual review, record one decision with
`wt review FINDING_ID --decision acceptable --expect-evidence HASH --reason-file rationale.md` using the
IDs from the reviewed check. The command rejects stale evidence. Never bulk-accept
findings merely to pass a check. `confirmed-issue` stays actionable; `accepted-risk`
is a different, explicitly authorized decision, not a synonym for acceptable code.

`wt inspect FINDING_ID --format json` re-evaluates the current raw occurrence,
stored rationale, evidence digest, and specific stale reasons.
`wt reviews [FINDING_ID] --format json` exposes stored records, labeled not evaluated.
A replacement requires `--expect-hash` and appends history. Changed owning files,
repository-rule input sets, rule contracts, runtime semantics and explicitly
watched files reopen the decision. Copies do not inherit acceptance. Missing
matches are not automatically fixed. Summary results contain every
actionable diagnostic and errors, plus accurate partition counts. Use
`wt check --detail full --format json` for `reviewed`, `files`, and
`review_records`; absence of these arrays in the summary means omitted inventory,
not zero occurrences.

See `wt guide review` and `wt schema review` in an installed build. Every
contract wt accepts (rule/submission, configuration, review, fixture, and the
result protocol) is at version 1; wt rejects any other version with a clear
error.
