# Verification

The acceptance suite exercises the CLI through the built `wt` binary and validates real submissions, disk manifests, fixture files, plans, and command results with the Rust `jsonschema` crate.

Run the focused suite with:

```sh
cargo test -p wt-cli --test acceptance
```

The suite includes the `ast.v1` TSX vectors under `examples/template-number-decimal-step/`, including the retained `invalid-tsx.tsx` demanded-parse case. A demanded structural parse failure must be reported as incomplete analysis; a guarded `ast_match` call that never reaches an invalid file must not attempt to parse it.

## Shared-query runtime performance

Source text, match/capture text, and captured structural attributes use immutable shared storage. Each text snapshot lazily retains its content digest, so native query keys do not rehash the same bytes for each consumer. Source spans still use the original UTF-8 byte offsets, and consumers retain independent file handles, diagnostics, and logical budgets. The `cache_key_bytes_hashed` runtime statistic measures query-key hashing separately from reader `bytes_hashed`.

Reproduce with:

```sh
cargo test -p wt-runtime --release --locked --test shared_query_performance -- --ignored --nocapture
```

Measured on Linux x86-64, kernel 7.0.0-31-generic, Rust 1.94.0, release profile, four logical CPUs. Each workload uses 32 files and 128 consumer programs; times are the median of three repeats:

| Workload | Median time |
| --- | ---: |
| Whole-file text | 0.015 s |
| Shared regex | 0.060 s |
| Many matches | 1.330 s |

These are isolated query-runtime measurements, not end-to-end `wt check` throughput: compilation, file construction, disk enumeration, worker IPC, and persistent-cache operations are outside the timed region. Small samples and machine load limit generalization to other hardware. The test also asserts logical step/native-byte/temporary-byte counts and covers same-path/length content replacement, Unicode views, shared capture payloads, per-consumer handles, and optimized/reference behavior equivalence; no timing threshold is asserted by the test itself.

## Large-corpus work-count matrix

`cargo test -p wt-cli --test acceptance` also defines an ignored, expensive test that builds a reproducible 10,000-file, ~100 MiB corpus and runs 25, 100, and 1,000 simple shared-pattern rules against it, asserting the expected source-read, byte-hash, logical-query, physical-evaluation, and shared-hit counts described in [`docs/spec.md`](spec.md#11-shared-execution-planning-scheduling-and-caching). It is not part of the default `cargo test` run because corpus creation and three full scans take on the order of tens of minutes; run it explicitly when verifying query-sharing work counts after a change to the planner or cache layer:

```sh
cargo test -p wt-cli --release --locked --test acceptance performance_matrix_work_counts_10k_files_100mib_and_25_100_1000_rules -- --ignored --nocapture
```

The test asserts work counts (source reads, bytes hashed, logical/physical query counts, shared-result hits), not a wall-clock budget; it does not claim a specific throughput number, peak RSS, or cross-machine timing guarantee. Distinct-pattern, repository-rule, `--changed`, and cold/warm-cache scaling remain covered only by the smaller deterministic tests in the focused acceptance suite, not by a published large-corpus timing table.
