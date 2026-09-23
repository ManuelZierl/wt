# Watchtower

Watchtower (`wt`, also shipped as `watchtower`) is a local-first CLI for running small, documented repository detectors. It accepts strict JSON rule submissions, stores readable `.wt` packages, and delegates validation and execution to `wt-core` and `wt-runtime`.

## Implementation Status

The current workspace implements the CLI adapter, strict JSON submission sources, package creation/update, streamed scoped file selection, validation, fixture tests, text/JSON output, verbatim bundled schema lookup, persistent content-verified caches with fixture gates, worker-backed execution, shared native queries, and the core exit-code envelopes. The bundled examples and CLI tests exercise those paths.

The full specification remains broader than this workspace. Current residual gaps include configurable runtime/optimizer profiles, hard OS memory enforcement on macOS/Windows, and the complete performance acceptance matrix. Repository changed-file checks retain the full authorized scope needed by repository rules. The checked-in schemas describe current JSON structures; they do not replace runtime validation. See [verification results](docs/verification.md) for the executed 10,000-file shared-query workload and its limits.

## Quickstart

Build the two executable names:

```bash
cargo build -p wt-cli
```

To install both commands into Cargo's binary directory:

```bash
cargo install --path crates/wt-cli --locked
```

### Install on another Ubuntu machine

For a private repository, copy this checkout (including `Cargo.lock`) to the machine, install the Rust toolchain named in `rust-toolchain.toml` using [rustup](https://rustup.rs/), then run from the checkout:

```bash
cargo install --path crates/wt-cli --locked
wt --version
watchtower --version
```

This builds both executables on the destination, without GitHub Actions. Cargo needs access to the dependencies on its first build. Alternatively, build a `.deb` on an Ubuntu machine with Cargo, Python 3, and `dpkg-deb`:

```bash
bash build-ubuntu-deb.sh
# Transfer dist/wt_0.1.0_amd64.deb to the other machine, then there:
sudo apt install ./wt_0.1.0_amd64.deb
wt --version
```

Build the `.deb` on the **oldest Ubuntu release** you plan to install it on, with the same CPU architecture as the destination. Linux binaries built against newer glibc may not work on older Ubuntu releases. The package declares the builder's glibc version as a conservative minimum and installs both `wt` and `watchtower` into `/usr/bin`.

Create a small repository and install the two shared-query examples as local advisory rules:

```bash
mkdir -p /tmp/wt-demo
printf 'request("foo123")\n' > /tmp/wt-demo/source.txt

target/debug/wt new --stdin --root /tmp/wt-demo --global-dir /tmp/wt-global \
  < examples/shared-request-foo.json
target/debug/watchtower new --stdin --root /tmp/wt-demo --global-dir /tmp/wt-global \
  < examples/shared-request-foo123.json
target/debug/wt check --no-global --root /tmp/wt-demo --global-dir /tmp/wt-global
```

The final command prints a human-readable advisory finding. Use `--format json` for one machine-readable document, including structured command errors:

```bash
target/debug/wt check --no-global --root /tmp/wt-demo --global-dir /tmp/wt-global --format json
```

The examples intentionally use advisory mode. A finding is therefore labeled `ADVISORY` and exits `0`; enforced findings exit `1`.

The decimal reference package is available both as a self-contained submission and as an editable package:

```bash
target/debug/wt validate --file examples/template-number-decimal-step.json --format json
target/debug/wt new --stdin --root /tmp/wt-demo --global-dir /tmp/wt-global \
  < examples/template-number-decimal-step.json
target/debug/wt test local/template-number-decimal-step --root /tmp/wt-demo \
  --global-dir /tmp/wt-global --no-global --format json
```

The package retains the invalid-TSX analysis-error vector separately because the fixture contract expresses expected diagnostics, not expected runtime failures. The decimal submission declares consolidated `text.v1` and `jsx.v1`; the schemas continue to accept `regex.v1` for legacy packages.

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

`wt schema rule|submission|tests|config|result|plan|waivers` prints the corresponding checked-in schema bytes, not a core-generated approximation.

See [`docs/cli.md`](docs/cli.md), [`docs/runtime-configuration.md`](docs/runtime-configuration.md), [`examples/`](examples/), [`spec.md`](spec.md), and [`schemas/`](schemas/) for contracts and current boundaries.
