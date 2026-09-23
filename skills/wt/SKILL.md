---
name: wt
description: Use when creating, inspecting, validating, testing, checking, updating, or enforcing Watchtower (WT) rules and configuration, including WRL1 detectors, fixtures, modes, coverage, waivers, and trusted CI.
---

# Watchtower (WT)

Use WT for deterministic, local repository knowledge. WT is not an LLM reviewer: it does not call a model, infer a new rule, or decide whether a convention is correct. The agent authors and reviews the rule; WT validates and executes it.

## Contract First

Check `wt --version`, `wt --help`, and `wt schema submission` against the installed executable. `watchtower` is the alternate executable name, useful where `wt` names Windows Terminal. If neither executable is installed, report that prerequisite before attempting checks.

This skill is portable: the repository being checked need not contain WT's own sources. Use [the bundled WRL1 reference](references/wrl1.md) and the installed command's schemas. When a WT source checkout is available, its `docs/cli.md`, `docs/runtime-configuration.md`, and `spec.md` provide additional detail. The specification describes the intended contract; verify a capability against the executable before relying on it.

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

`config` shows effective values and origins. `list` shows declared/effective modes, scope, test availability, and digests. `show` exports a self-contained rule with detector and fixture content. `plan` inspects the compiled plan without scanning application source:

```sh
wt plan --rule local/rule-id --format json
```

Use `--no-global` only when the intended policy is local-only. Otherwise the default combines global and local rules.

## Author A Narrow Rule

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

Validate a submission without writing it, then create it through stdin. `new` accepts exactly one source: positional JSON, `--stdin`, or `--file`.

```sh
wt validate --file submission.json --format json
wt new --stdin --format json < submission.json
wt test local/rule-id --format json
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

`--changed` and narrowed paths are partial coverage. Record that fact and do not present the result as a repository-wide proof. `--include-ignored` changes only Git-ignore filtering. `--show-suppressed` exposes waiver-suppressed findings; stale waivers remain actionable, and waivers do not excuse analysis gaps.

## Update Safely

Use the JSON `show` response's `rule` field as the self-contained submission, retaining all existing tests and fixture content. Do not submit the surrounding command envelope. Edit the submission and use the `digest` returned by that same `show` response to update atomically:

```sh
wt list --format json
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

Report the command, rule IDs, fixture outcome, full/partial coverage, findings, and analysis failures. Do not disable a rule, weaken its scope, remove examples, or add a waiver merely to make a check pass. A detector-recognition error belongs in the detector and fixtures. An acceptable occurrence of an intended review pattern belongs in an explicit evidence-bound occurrence review; do not narrow a detector just to remove it. An intentionally accepted violation requires separately justified accepted-risk authorization or a legacy waiver.

## Readable Rules And Occurrence Decisions

`new` and `update` automatically format `check.wt` and store long-form prose only
in `rule.md`. Legacy schema-2 rules remain readable and migrate on explicit update.
Use `wt fmt --check` for existing local scripts. `wt check` never reformats source.

A `review` finding means **inspect this pattern**, not **this is a proven bug**.
An acceptable occurrence is still a raw positive. Keep that detector fixture.
After actual contextual review, record one decision with `wt review FINDING_ID
--decision acceptable --expect-evidence HASH --reason-file rationale.md` using the
IDs from the reviewed check. The command rejects stale evidence. Never bulk-accept
findings merely to pass a check. `confirmed-issue` stays actionable; `accepted-risk`
is a different, explicitly authorized decision, not a synonym for acceptable code.

`wt reviews --format json` exposes the current stored record hash and rationale.
A replacement requires `--expect-hash` and appends history. Changed owning files,
repository-rule input sets, rule contracts, runtime semantics and explicitly
watched files reopen the decision. Copies do not inherit acceptance. Missing
matches are not automatically fixed. Inspect `reviewed`, `review_records`, all
analysis errors and coverage alongside actionable `diagnostics`.

See `docs/readable-rules-and-reviews.md` in a WT checkout and `wt review --help` /
`wt schema review` in an installed build. Rule schema 3, review schema 1 and command
envelope schema 2 are independent; inspect the installed executable before use.
