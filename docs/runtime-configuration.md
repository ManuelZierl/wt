# Runtime Configuration

The current runtime and worker boundary apply these defaults and safety ceilings:

| Resource | Ceiling |
|---|---:|
| Detector source | 128 KiB |
| Source file | 4 MiB default, 64 MiB binary ceiling |
| Regex expression | 8 KiB |
| Patterns per rule | 128 |
| Logical steps, file / repository invocation | 1,000,000 / 10,000,000 |
| Regex results per call | 10,000 |
| Other sequence elements per call | 100,000 |
| Diagnostics per rule/file | 1,000 |
| Local variables | 256 |
| Frontend AST nodes | 1,000,000 |
| Nesting depth | 64 |
| Logical native input, file / repository invocation | 128 MiB / 1 GiB |
| Retained derived values | 64 MiB |
| Temporary values | 64 MiB |
| Repository input files | 10,000 |
| Repository snapshots | 100 MiB |
| File invocation watchdog | 2 seconds |
| Repository invocation watchdog | 30 seconds |
| Worker reservation | 256 MiB default; 512 MiB ceiling |
| Parent reservation | 256 MiB default; 512 MiB ceiling |
| Combined worker scheduling budget | 1 GiB |
| Compiled worker cache | 256 MiB |

File-local work is assigned to persistent worker processes. Jobs default to `min(available CPUs, admitted workers)` and explicit jobs are limited to `1..=3` under the absolute worker reservation; configured reservations may admit fewer. Source selection is streamed and reports content-verified source-read/byte-hash counts. Raw findings are cached by rule/path/content identity; fixture results are separately gated by package and fixture digests. `--no-cache` disables persistent cache use, not in-run query sharing.

On Linux, workers apply a hard `RLIMIT_AS` address-space limit equal to the configured worker reservation (256 MiB by default). macOS and Windows currently use the scheduling reservation and logical allocation limits without a hard OS memory quota.

Configuration schema 3 accepts these independently merged settings; omitted fields
retain their built-in values. Logical settings are positive integers no greater
than their documented ceilings. A smaller limit causes an explicit incomplete check
if the detector exceeds it; it never converts work into a successful no-match.

```json
{
  "schema_version": 3,
  "runtime": {
    "file_steps": 1000000,
    "file_native_bytes": 134217728,
    "repository_steps": 10000000,
    "repository_native_bytes": 1073741824,
    "worker_memory_bytes": 268435456,
    "parent_memory_bytes": 268435456,
    "total_memory_bytes": 1073741824
  },
  "optimizer": {"mode": "auto"}
}
```

`optimizer.mode` accepts `auto` or `off`; an explicit `--optimizer` overrides it
for check/plan, including `--optimizer auto` when configuration says `off`.
`wt config --format json` reports each effective value and its origin, and the
check's `effective_policy` contains the effective profile. Raw and fixture cache
keys include the effective runtime limits, so tighter limits cannot reuse an
earlier successful result. Result protocol 2 refuses a non-default profile.
Memory reservations accept 256–512 MiB per worker and parent, with a total of
512 MiB–1 GiB. Cross-field validation requires room for at least one worker.
Default jobs are capped by the admitted worker count; explicit `--jobs` above
it is an error. Linux workers apply the configured address-space limit at process
startup. macOS/Windows reservations are scheduling limits, not hard OS quotas.
Other runtime budgets and physical optimizer knobs
remain fixed and unsupported configuration keys fail validation.

Configuration schema 3 adds `coverage.expectations`: each entry has a qualified
`rule_id`, positive `minimum_files`, and nonempty `reason`. Loaded global and
local entries accumulate. An expected rule must be enabled, applicable, and
complete over at least that many eligible files. Narrowed checks report
`not_evaluated_partial`; omitting a global rule does not satisfy a local
expectation for it. `--allow-empty` does not bypass an expectation. Schema-2
configuration remains readable without the coverage field.

These limits are safety ceilings, not benchmark claims. They do not establish the full performance acceptance matrix, constant-time scaling, or whole-spec completion.
