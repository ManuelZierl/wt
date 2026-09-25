# CLI Contract

The CLI is a thin Clap adapter around `wt_core::dispatch(command, options)`. It maps only options accepted by the selected core command. `wt` and `watchtower` invoke the same implementation.

## Input Sources

`wt new` accepts exactly one of a positional JSON document, `--stdin`, or `--file PATH`. `wt update ID` currently accepts `--stdin` and requires `--expect-hash HASH`. `wt validate --file PATH` validates a submission without writing it. Inputs are UTF-8, bounded to 8 MiB by the CLI, and parsed with the core duplicate-key/trailing-data rejecting parser.

## Output

`--format json` writes exactly one JSON document to stdout. Argument errors are also JSON when the format flag is recognizable before Clap finishes parsing. Progress and diagnostics are not mixed into JSON stdout. Text check output gives each finding a `BLOCKING` or `ADVISORY` label, message, help, and a summary. Check uses top-level findings and counts; other commands put their payload in `data`. `--detail full` on check includes accepted and waived findings and file/review inventory.

`wt schema rule|submission|tests|config|result|plan|review|capabilities` prints the installed schema. `wt capabilities --format json` is a standalone capability document. Neither reads the repository or global configuration.

Exit codes are `0` for a completed non-blocking result, `1` for blocking findings or failed fixture expectations, `2` for invalid invocation/configuration/incomplete analysis, and `130` for a controlled interruption.

Rule and review mutations coordinate through persistent lock files under
`wt-locks/` alongside the platform WT cache directory (for example,
`~/.cache/wt-locks/` on Unix with default paths). Lock files are not checked
into `.wt/` and `wt cache clear` does not delete them. Do not unlink live lock
files: an empty file can still be held by another process.

## Supported Command Mapping

The command list follows [`docs/spec.md`](spec.md) section 8.1: `init`, `capabilities`, `guide`, `new`, `check`, `plan`, `stats`, `list`, `show`, `validate`, `test`, `update`, `review`, `reviews`, `inspect`, `fmt`, `set-mode`, `explain`, `config`, `schema`, and `cache clear`. Command-specific options are rejected by Clap or by the core option validator rather than ignored.

The CLI forwards explicit `--jobs 1..=3`; when omitted, the core chooses `min(available CPUs, admitted workers)`. Configuration supports bounded file/repository execution limits, worker/parent/total memory reservations, and `optimizer.mode`; see [runtime configuration](runtime-configuration.md). `--optimizer off` is the semantic reference path; explicit `--optimizer auto` overrides a configured `off`. `--no-cache` disables persistent raw-result caching while retaining in-run execution semantics. `--changed` narrows file-local work but keeps repository-rule dependencies in full scope.

## Readable authoring and occurrence review

`wt fmt [ID] [--check] [--global]` formats local programs by default. `new` and
`update` format automatically and store Markdown-backed packages.
`wt review ID --decision ... --expect-evidence HASH --reason-file PATH` records an
explicit decision; replacement requires `--expect-hash`. `wt reviews` inspects
retained rationale and history metadata. `wt schema review` exposes review records.
`wt inspect ID` reevaluates an occurrence read-only and includes its previous
rationale and categorized evidence changes. `wt check --submission PATH` runs
one self-contained candidate with `candidate/<id>` identity, without installing
it or applying existing decisions. `wt update ... --preview` validates, formats,
and tests a candidate without changing the active package. `wt config --format
json` includes configured and effective settings; coverage expectations
are evaluated separately from the finding count.
See [the complete contract](readable-rules-and-reviews.md) for evidence boundaries,
staleness, extra watched files, and enforcement semantics.
