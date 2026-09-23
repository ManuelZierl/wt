# Runtime Configuration

The current runtime and worker boundary apply these fixed safety ceilings:

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
| Worker reservation | 256 MiB |
| Parent reservation | 256 MiB |
| Combined worker scheduling budget | 1 GiB |
| Compiled worker cache | 256 MiB |

File-local work is assigned to persistent worker processes. Jobs default to `min(available CPUs, 3)` and explicit jobs are limited to `1..=3` by the combined reservation. Source selection is streamed and reports content-verified source-read/byte-hash counts. Raw findings are cached by rule/path/content identity; fixture results are separately gated by package and fixture digests. `--no-cache` disables persistent cache use, not in-run query sharing.

On Linux, workers apply a hard 256 MiB `RLIMIT_AS` address-space limit. macOS and Windows currently use the scheduling reservation and logical allocation limits without a hard OS memory quota. Configuration exposes scanner policy, mode overrides, and file-size selection; configurable runtime/optimizer profiles remain unimplemented.

These limits are safety ceilings, not benchmark claims. They do not establish the full performance acceptance matrix, constant-time scaling, or whole-spec completion.
