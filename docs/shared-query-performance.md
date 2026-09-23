# Shared-query runtime performance

Source text, match/capture text, and cached JSX attributes now use immutable shared storage. Each text snapshot lazily retains its content digest, so native query keys do not rehash the same bytes for each consumer. Source spans still use the original UTF-8 byte offsets, and consumers retain independent file handles, diagnostics, and logical budgets.

`SourceFile.text` is now `SharedText` for Rust callers (`text: value.into()`). The CLI, worker JSON wire format, and WT rule language are unchanged. The `cache_key_bytes_hashed` runtime statistic measures query-key hashing separately from reader `bytes_hashed`.

## Local measurement

Measured sequentially on the same machine (Ubuntu, x86_64, Rust 1.94.0) in release mode against base commit `0819df8`. Each workload uses 32 files, 128 consumer programs, and the median of three fresh-snapshot runs. Results from this machine:

| Workload | Baseline | Patched | Baseline / patched |
| --- | ---: | ---: | ---: |
| Whole-file text | 1.836 s | 0.018 s | ~102x |
| Shared regex | 3.874 s | 0.089 s | ~43x |
| Many matches | 5.759 s | 1.734 s | ~3.3x |

The logical steps, native bytes, and temporary bytes in all three runs match the baseline. The tests also cover same-path/length content replacement, Unicode views, shared capture and JSX payloads, per-consumer handles, and optimized/reference behavior. No timing threshold is asserted.

These are isolated query-runtime measurements, not end-to-end `wt check` throughput. Compilation, file construction, disk enumeration, worker IPC, and persistent-cache operations are outside the timed region. Small samples and machine load limit generalization. Result traversal and independent predicates remain proportional to consumer/match counts.

To reproduce:

```sh
cargo test -p wt-runtime --release --locked --test shared_query_performance -- --ignored --nocapture
```
