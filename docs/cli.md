# CLI Contract

The CLI is a thin Clap adapter around `wt_core::dispatch(command, options)`. It maps only options accepted by the selected core command. `wt` and `watchtower` invoke the same implementation.

## Input Sources

`wt new` accepts exactly one of a positional JSON document, `--stdin`, or `--file PATH`. `wt update ID` currently accepts `--stdin` and requires `--expect-hash HASH`. `wt validate --file PATH` validates a submission without writing it. Inputs are UTF-8, bounded to 8 MiB by the CLI, and parsed with the core duplicate-key/trailing-data rejecting parser.

## Output

`--format json` writes exactly one JSON document to stdout. Argument errors are also JSON when the format flag is recognizable before Clap finishes parsing. Progress and diagnostics are not mixed into JSON stdout. Text check output gives each finding a `BLOCKING` or `ADVISORY` label, message, help, and a summary.

`wt schema rule|submission|tests|config|result|plan|waivers` is the exception to the result-envelope convention: it writes the corresponding checked-in schema file verbatim. The schema command does not read the repository or global configuration.

Exit codes are `0` for a completed non-blocking result, `1` for blocking findings or failed fixture expectations, `2` for invalid invocation/configuration/incomplete analysis, and `130` for a controlled interruption.

## Supported Command Mapping

The command list follows `spec.md` section 9.1: `init`, `new`, `check`, `plan`, `list`, `show`, `validate`, `test`, `update`, `set-mode`, `explain`, `config`, `schema`, and `cache clear`. Command-specific options are rejected by Clap or by the core option validator rather than ignored.

The CLI forwards explicit `--jobs 1..=3`; when omitted, the core chooses `min(available CPUs, 3)`. `--optimizer off` is the semantic reference path. `--no-cache` disables persistent raw-result caching while retaining in-run execution semantics. `--changed` narrows file-local work but keeps repository-rule dependencies in full scope.

## Readable authoring and occurrence review

`wt fmt [ID] [--check] [--global]` formats local programs by default. `new` and
`update` format automatically and store schema-3 Markdown-backed packages.
`wt review ID --decision ... --expect-evidence HASH --reason-file PATH` records an
explicit decision; replacement requires `--expect-hash`. `wt reviews` inspects
retained rationale and history metadata. `wt schema review` exposes review records.
See [the complete contract](readable-rules-and-reviews.md) for evidence boundaries,
staleness, extra watched files, legacy migration, and enforcement semantics.
