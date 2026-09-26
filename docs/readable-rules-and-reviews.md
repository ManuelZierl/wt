# Readable rules and occurrence reviews

Watchtower detectors may express either a prohibited pattern (`violation`) or a
pattern that deserves contextual judgment (`review`). Finding an acceptable use
of a review pattern is not, by itself, a reason to narrow the detector for every
future occurrence. Keep the detector and record the occurrence decision instead.
WT does not decide whether the underlying concern is a proven application bug.

## Readable packages

New and updated packages use rule schema **1**:

```text
.wt/rules/my-rule/
  rule.json       # identity, mode, severity, scope, patterns, diagnostic messages
  rule.md         # authoritative description, rationale, limitations, evidence
  check.wt        # formatted WRL1 source
  tests.json      # raw detector expectations, independent of occurrence decisions
```

The disk manifest contains `"documentation": {"file": "rule.md"}`. A self-contained
JSON submission/export instead contains `"documentation": {"source": "# ...\n"}`.
There is no second authoritative description/rationale/limitations copy. Markdown is not executed, and headings have no hidden policy semantics.
Recommended sections are **Intended constraint**, **What is detected**, **How to
review**, **Known limitations**, and **Evidence**. Record hypotheses as hypotheses;
passing detector fixtures does not prove the motivating application defect.

`check`, `show` and `fmt` do not change policy. The package digest includes
`rule.md`; raw detection cache keys exclude long-form prose.
Contract/documentation edits conservatively invalidate existing review
acceptances even when raw detection results are reusable.

## Automatic WRL formatting

`new` and `update` validate and format code before storing it, then validate the
formatted program. The returned digest identifies the actual stored bytes.

```sh
wt fmt                  # local packages only
wt fmt local/my-rule
wt fmt --check          # no writes; exit 1 if formatting differs
wt fmt --global         # explicitly select global packages
```

Formatting uses four-space blocks and one statement per line. It preserves tokens,
string literal contents and comments; it never hoists searches or changes guards.
`fmt` rejects invalid programs and validates all selected source before writing.
Formatting is idempotent. Normal `check` is always read-only for rules and reviews.

## Finding identity versus review evidence

Each raw diagnostic includes `finding_id`, `evidence_digest`, `matched_digest`,
`context_digest` and `engine_digest`. A finding ID identifies the qualified rule,
diagnostic code, exact path, byte span and matched bytes. It is deliberately
conservative: WT does **not** fuzzy-match moved code or transfer approval to copies.
Moving a match may produce a new finding rather than silently reusing a judgment.

Acceptance validity is separately bound to the complete rule-package digest,
runtime semantics, owning file bytes and the authorized input-set context. For
repository rules, additions/deletions/changes within the authorized input set
invalidate the decision. For file rules, changing any owning-file bytes invalidates
it even when the matched snippet is unchanged. Additional supporting files can
be recorded with expected hashes. No generic tool can infer every semantic
assumption from prose: dependencies outside the detector input set must be
explicitly watched.

`engine_digest` is that runtime-semantics binding. It is not a hash of the whole
compiled wt binary: it is a maintainer-controlled semantics epoch plus the exact
pinned dependency versions (read from the build's `Cargo.lock`) that implement
matching for the rule's declared capabilities — the WRL1 interpreter always, plus
`text.v1`'s regex engine, `path.v1`/`repo.v1`'s globs, or `ast.v1`'s ast-grep engine
and tree-sitter grammars, whichever the rule declares. A dependency bump (a grammar
update, say) changes `engine_digest` automatically and reopens affected decisions;
an interpreter or matcher semantics change with no dependency bump instead requires
the maintainer to advance the epoch by hand. A capability added after a release
folds its own pinned dependency into the identity only for rules that declare it;
other rules keep the identity they already had. There is no claim that unchanged
local evidence proves that an external service, requirement, or unrecorded
dependency remains unchanged.

## Explicit decision workflow

Run the check, inspect the occurrence and supporting code, and record **one**
decision using the IDs from the evidence actually reviewed:

```sh
wt check --format json
wt review sha256:FINDING_HASH \
  --decision acceptable \
  --expect-evidence sha256:EVIDENCE_HASH \
  --reason-file rationale.md
wt reviews --format json
```

The uppercase hashes above are placeholders. Real hashes are 64 lowercase hex
characters after `sha256:`. `review` runs a fresh uncached check and refuses stale,
absent, waived, ambiguous or incompletely analyzed findings. It also verifies the
owning file again before recording. `--rule ID`, `--no-global` and
`--no-host-ignores` can specify the intended check policy.

Use `--watch 'src/helper.rs=sha256:EXPECTED_HASH'` for additional reviewed file
contents; the hash must be supplied from the evidence reviewed, not invented by
WT. Up to 32 additional files are supported. Changed or unavailable watched files
make a decision stale. Review evidence never follows source or storage symlinks.
The check remains a working-tree scan, not an atomic snapshot of concurrent edits.

| Decision | Effect when evidence is current |
|---|---|
| `needs-review` | Remains actionable; explicitly reopens a previous decision. |
| `acceptable` | Accepts one `review` occurrence in the documented context. |
| `confirmed-issue` | Remains actionable; does not silently change the rule's mode. |
| `accepted-risk` | Explicitly accepts a known concern; permitted for either kind. |

CLI names use hyphens; JSON decision values use underscores. `acceptable` cannot
silence a `violation`; use the explicitly different accepted-risk decision when
authorization permits it. Decisions do not establish who authorized them: the
record is an inspectable repository artifact, not an authenticated approval.

## Storage and history

Decisions are repository-local even when the rule is global:

```text
.wt/reviews/<finding hash without sha256:>/
  00000001/decision.json
  00000001/rationale.md
  00000002/decision.json
  00000002/rationale.md
```

Every change appends an atomic, numbered revision with a link to the prior record
digest. Supply `--expect-hash` from `wt reviews` to replace an existing decision;
blind overwrites fail. No previous rationale is discarded. There are no timestamps
that change check semantics and no automatic accept-all/baseline operation.
`wt schema review` exposes the record schema. Manual edits are possible but must
satisfy the same structural and identity checks. Review state is not cache data;
`wt cache clear` never deletes it. Invalid state makes analysis incomplete while
preserving available raw findings.

## Check output and enforcement

Raw detection and fixture testing never consult decisions. Current accepted
occurrences move to the JSON `reviewed` array; actionable findings remain in
`diagnostics`. The summary gives raw, reviewed and actionable counts. Human output
shows the review/violation kind, IDs and stale-decision status. `review_records`
includes records that were not observed; that status is **not** a claim of a fix.
The rule may be out of scope, disabled, or no longer match. An unchanged acceptance
cannot excuse a runtime failure, missing analysis, or incomplete check.

Modes still control blocking: advisory findings do not block unless `--strict`;
enforced actionable findings block. A current explicit acceptance satisfies the
review obligation even in strict mode. `reviews` displays stored evidence and
rationale but labels validity `not_evaluated`; only a fresh check evaluates it.

The compact result protocol is the default, with compact check inventory and
explicit `--detail full`. Use the current bundled result schema for strict
validation. WRL1, the rule/submission/review/config/fixture contract version,
and the result protocol are independent things that happen to share the
same version number today.

## Agent and CI requirements

Improve a detector when it recognizes the wrong pattern. Record a contextual
decision when it correctly recognizes the intended review signal. Preserve raw
positive fixtures for acceptable occurrences; test acceptance in the review-state
layer rather than changing a detector expectation to suppress the pattern.
Do not target a quota of findings, invent a reproduction, accept all matches, or
add contextual exceptions merely to make a command pass.

Protect rules, Markdown contracts, decisions/history, ignore/scope settings,
checker version and the final invocation through trusted review/CI. An actor who
can rewrite all those inputs can weaken policy. Stored hashes are identities,
not signatures. `check` never creates a new approval or rewrites a decision.

```sh
wt fmt --check
wt check --no-global --no-host-ignores --no-cache --format json
```
