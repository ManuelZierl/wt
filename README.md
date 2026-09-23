# Watchtower

Watchtower (`wt`, also shipped as `watchtower`) is a local-first CLI for running small, documented repository detectors. It accepts strict JSON rule submissions and stores readable `.wt` packages. No account, daemon, or hosted service is required.

## Install (Ubuntu 22.04+ x86-64)

```bash
curl -fsSL https://raw.githubusercontent.com/ManuelZierl/wt/main/install.sh | bash
```

The installer downloads the latest GitHub Release, checks its SHA-256 digest, and installs `wt` and `watchtower` to `~/.local/bin` without sudo or a Rust toolchain. Add that directory to `PATH` if needed. Set `WT_INSTALL_DIR` to choose another location, or `WT_VERSION=v0.1.0` to pin a release.

### From source

With [Rust](https://rustup.rs/) installed, install both commands from GitHub or a checkout:

```bash
cargo install --git https://github.com/ManuelZierl/wt.git wt-cli --locked
# Or from this checkout:
cargo install --path crates/wt-cli --locked
```

To build distributable binaries locally without using GitHub Actions, install [Zig 0.13.0](https://ziglang.org/download/) and `cargo-zigbuild`, then run:

```bash
cargo install cargo-zigbuild --version 0.23.4 --locked
bash build-linux-release.sh
```

The script checks for a maximum glibc requirement of 2.35 and writes the release archive and its checksum to `dist/`. Run `bash tests/install.sh` to check the installer locally, then upload both files to a GitHub Release under the asset names produced by the script. To make a `.deb` for a particular Ubuntu version instead, run `bash build-ubuntu-deb.sh` on the oldest Ubuntu release you intend to support.

## Quickstart

From a source checkout, create a small repository and install the two shared-query examples as local advisory rules:

```bash
mkdir -p /tmp/wt-demo
printf 'request("foo123")\n' > /tmp/wt-demo/source.txt

wt new --stdin --root /tmp/wt-demo --global-dir /tmp/wt-global \
  < examples/shared-request-foo.json
watchtower new --stdin --root /tmp/wt-demo --global-dir /tmp/wt-global \
  < examples/shared-request-foo123.json
wt check --no-global --root /tmp/wt-demo --global-dir /tmp/wt-global
```

The final command prints a human-readable advisory finding. Use `--format json` for one machine-readable document, including structured command errors:

```bash
wt check --no-global --root /tmp/wt-demo --global-dir /tmp/wt-global --format json
```

The examples intentionally use advisory mode. A finding is therefore labeled `ADVISORY` and exits `0`; enforced findings exit `1`.

The decimal reference package is available both as a self-contained submission and as an editable package:

```bash
wt validate --file examples/template-number-decimal-step.json --format json
wt new --stdin --root /tmp/wt-demo --global-dir /tmp/wt-global \
  < examples/template-number-decimal-step.json
wt test local/template-number-decimal-step --root /tmp/wt-demo \
  --global-dir /tmp/wt-global --no-global --format json
```

The package retains the invalid-TSX analysis-error vector separately because the fixture contract expresses expected diagnostics, not expected runtime failures. The decimal submission declares consolidated `text.v1` and `jsx.v1`; the schemas continue to accept `regex.v1` for legacy packages.

## Readable Rules And Reviewed Occurrences

New and updated packages store formatted `check.wt` programs and authoritative
Markdown explanations in `rule.md`, with execution metadata in `rule.json`.
Legacy rule packages remain importable. Use `wt fmt --check` in CI and `wt fmt`
for manually edited local rules; ordinary checks never rewrite them.

A pattern can require review without being an error in every context. `wt review`
records one explicit decision against the evidence actually reviewed. Current
acceptances remain visible; changed context makes them actionable again. These
records are repository knowledge, not a cache or a replacement for trusted review.
See [the format, review lifecycle and migration guide](docs/readable-rules-and-reviews.md).

## Implementation Status

The current workspace implements the CLI adapter, strict JSON submission sources, package creation/update, streamed scoped file selection, validation, fixture tests, text/JSON output, verbatim bundled schema lookup, persistent content-verified caches with fixture gates, worker-backed execution, shared native queries, and the core exit-code envelopes. The bundled examples and CLI tests exercise those paths.

The full specification remains broader than this workspace. Current residual gaps include configurable runtime/optimizer profiles, hard OS memory enforcement on macOS/Windows, and the complete performance acceptance matrix. Repository changed-file checks retain the full authorized scope needed by repository rules. The checked-in schemas describe current JSON structures; they do not replace runtime validation. See [verification results](docs/verification.md) for the executed 10,000-file shared-query workload and its limits.

## Agent And CI Use

The recommended agent interchange is strict JSON on stdin:

```bash
wt new --stdin --format json < proposed-rule.json
wt validate --file proposed-rule.json --format json
wt test local/my-rule --format json
wt plan --rule local/my-rule --format json
wt check --rule local/my-rule --format json
```

For a repository-local CI policy, use `wt check --no-global --no-host-ignores --no-cache --format json`. This is reproducible with respect to selected policy, but it is not tamper-proof when the editing agent controls the repository, checker binary, CI invocation, or `.wt` files. A trusted review/CI boundary must protect those inputs, the mode configuration, waivers, and the final command.

The portable agent skill is [`skills/wt/SKILL.md`](skills/wt/SKILL.md), with a runnable submission and WRL1 API reference. [Installation instructions](docs/agent-skill.md) cover OpenCode and other Agent Skills-compatible tools.

## Safety And Ceilings

Rule source is capped at 128 KiB, source files default to 4 MiB with a 64 MiB binary ceiling, regex expressions at 8 KiB, patterns per rule at 128, and regex results per call at 10,000. The CLI bounds one JSON submission/file input to 8 MiB before strict parsing. File invocations have a hard 2-second watchdog and repository invocations a 30-second watchdog. Jobs default to `min(available CPUs, 3)` and explicit values are bounded to `1..=3`; workers reserve 256 MiB each within a 1 GiB combined scheduling budget. Linux workers apply a 256 MiB address-space limit; macOS/Windows use scheduling and logical limits. These are safety ceilings, not throughput promises.

`wt schema rule|submission|tests|config|result|plan|waivers|review` prints the corresponding checked-in schema bytes, not a core-generated approximation.

See [`docs/cli.md`](docs/cli.md), [`docs/runtime-configuration.md`](docs/runtime-configuration.md), [`examples/`](examples/), [`spec.md`](spec.md), and [`schemas/`](schemas/) for contracts and current boundaries.
