# Watchtower

Every bug fix an agent makes can leave more behind than the fix itself: a small, documented, tested check for the pattern that caused it. `wt` (also shipped as `watchtower`) runs those checks locally — no account, daemon, network, or model calls. A finding means *look at this*, not *this is wrong*: a `violation` is a prohibited source form, a `review` is a situation worth a human's judgment. Decisions about a `review` finding are recorded against the exact evidence reviewed and reopen automatically when the owning code, the rule, or a watched dependency changes.

## The loop

An agent notices a possible Django N+1 query — an ORM call inside a loop, one query per iteration instead of one for the whole collection — and writes a rule for it, with fixtures, using structural (`ast.v1`) matching so formatting and comments don't confuse it: [`examples/orm-query-in-loop/`](examples/orm-query-in-loop/).

```bash
mkdir -p app
wt new --stdin --format json < examples/orm-query-in-loop.json
```

A teammate's change adds a real instance of it:

```bash
cat > app/tasks.py <<'PY'
def notify_active_users(users):
    for user in users:
        profile = Profile.objects.get(user=user)
        send_email(profile)
PY
wt check --format json | jq '{status, diagnostics: [.diagnostics[] | {path, code, kind, message}]}'
```

```json
{
  "status": "findings",
  "diagnostics": [
    {
      "path": "app/tasks.py",
      "code": "possible-n-plus-one",
      "kind": "review",
      "message": "An ORM query (`.objects.<method>(...)`) runs on every iteration of this loop, a possible N+1 query."
    }
  ]
}
```

That one really is an N+1: fetch the rows before the loop instead.

```bash
cat > app/tasks.py <<'PY'
def notify_active_users(users):
    profiles = Profile.objects.filter(user__in=users)
    for profile in profiles:
        send_email(profile)
PY
wt check --format json | jq '{status, actionable: .summary.actionable_occurrences}'
```

```json
{"status": "pass", "actionable": 0}
```

A second occurrence elsewhere is intentional and already cheap — a case for judgment, not a fix:

```bash
cat > app/views.py <<'PY'
def sync_status(orders):
    for order in orders:
        # Cached per request; batching adds complexity we don't need here.
        status = OrderStatus.objects.get(order=order)
        record(order, status)
PY
FINDING_ID=$(wt check --format json | jq -r '.diagnostics[0].finding_id')
EVIDENCE=$(wt check --format json | jq -r '.diagnostics[0].evidence_digest')
echo "Cached per request; batching adds complexity we don't need here." > rationale.md
wt review "$FINDING_ID" --decision acceptable --expect-evidence "$EVIDENCE" --reason-file rationale.md --format json | jq '{status, decision: .data.decision}'
wt check --format json | jq '{status, summary: {actionable: .summary.actionable_occurrences, reviewed: .summary.reviewed_occurrences}}'
```

```json
{"status": "pass", "decision": "acceptable"}
{"status": "pass", "summary": {"actionable": 0, "reviewed": 1}}
```

The decision is bound to `app/views.py`'s exact content and this rule's exact contract. Change either one and the acceptance reopens; a copy of the pattern elsewhere is a fresh, unreviewed occurrence, not an inherited approval.

## Install

Ubuntu 22.04+ x86-64:

<!-- docs-test: skip (downloads a GitHub release; not runnable offline) -->
```bash
curl -fsSL https://raw.githubusercontent.com/ManuelZierl/wt/main/install.sh | bash
```

The installer downloads the latest GitHub Release, verifies its SHA-256 digest, and installs `wt` and `watchtower` to `~/.local/bin` without sudo or a Rust toolchain. Add that directory to `PATH` if needed; set `WT_INSTALL_DIR` for another location or `WT_VERSION=v0.1.0` to pin a release.

With [Rust](https://rustup.rs/) installed, build from source instead:

```bash
cargo install --path crates/wt-cli --locked
```

## Why not Semgrep or ast-grep?

wt uses [ast-grep](https://ast-grep.github.io/)'s matching engine under `ast.v1` for the structural half of pattern-writing, plus a plain-text `text.v1` regex API — it doesn't reinvent syntax matching. What it adds is what happens *after* a match: a `violation`/`review` split instead of one severity, mandatory fixtures before a rule can enforce anything, evidence-bound review decisions that expire when the reviewed code actually changes, `wt stats` to see which rules are earning their place, and an authoring contract (strict JSON, a restricted rule language, no I/O) built for an agent to use directly. An inline `// eslint-disable` or `# noqa` comment never expires; a wt decision does.

## Agents and CI

The agent skill is [`skills/wt/SKILL.md`](skills/wt/SKILL.md) — see [`docs/agent-skill.md`](docs/agent-skill.md) for installing it into an agent harness. `wt capabilities --format json` and `wt guide author|review|language|stats` are the offline, version-matched reference an agent reads before authoring.

A local `wt check` is reproducible, not tamper-proof: an agent that controls the repository, the checker binary, the CI invocation, or `.wt/` can also weaken what it enforces. Protect those inputs and the final invocation with a boundary the editing agent does not control:

```bash
wt check --no-global --no-host-ignores --no-cache --format json
```

## Links

- [`docs/spec.md`](docs/spec.md) — the full product and technical specification.
- [`docs/cli.md`](docs/cli.md) — command, input/output, and exit-code contract.
- [`docs/readable-rules-and-reviews.md`](docs/readable-rules-and-reviews.md) — rule package format and the review-decision lifecycle.
- [`docs/runtime-configuration.md`](docs/runtime-configuration.md) — safety ceilings and configuration.
- [`docs/verification.md`](docs/verification.md) — how the test suite and performance measurements are reproduced.
- [`examples/`](examples/) — runnable rule packages, including the one above.
- [`schemas/`](schemas/) — the JSON Schemas `wt schema NAME` prints.
