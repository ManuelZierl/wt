# Watchtower (WT)
## Product and technical specification

> **2026-09-23 implementation amendment:** [Readable rules and occurrence reviews](docs/readable-rules-and-reviews.md) supersedes this document’s prose-storage, formatting, and contextual-exception handling. Rule package schema 3 stores Markdown documentation; pure detection is followed by evidence-bound review decisions. Other runtime and execution contracts remain unchanged.

**Working name:** Watchtower  
**Working command:** `wt`  
**Specification revision:** 2  
**Date:** 2026-09-22  
**Status:** Proposed implementation contract; this document does not describe an implemented or tested WT executable.

## Revision 2 summary and compatibility

This revision supersedes revision 1 as the proposed implementation contract. It retains global/local configuration, JSON agent interchange, human-readable packages, ignore semantics, fixtures, advisory/enforced modes, waivers, and trusted CI boundaries.

The changes are substantive: `check.wt` with an explicit WRL1 contract; a WT-owned compiler/query IR over the Rhai backend; file-centric scheduling; cross-rule query sharing; safe optional multi-pattern execution; granular caches; and optimizer conformance/inspection.

All revised JSON artifacts use `schema_version: 2`; the public detector language is `wt-rule-1`. These version numbers describe different contracts. Revision-1 `wt-rhai-1` submissions are not silently treated as WRL1. Migration must validate the body, remove an approved legacy entrypoint wrapper, change the declared language/file, update schemas, and rerun retained tests. No WT runtime release is asserted to exist, so this is a specification migration, not a claim of deployed compatibility.

The reference examples use top-level bodies and retain all original decimal fixtures. A new pair of text rules demonstrates shared matching with independent predicates. Examples and conformance vectors are expected behavior unless the package validation report explicitly says what was actually executed.

## 1. Product definition

Watchtower is a local-first command-line tool for executable repository knowledge.

A developer or coding agent discovers a recurring problem, describes the constraint behind it, and stores a small deterministic detector with its explanation and examples. WT applies the accumulated detectors to a repository and produces actionable findings for humans and machines.

**The product promise:** once a mistake has been understood well enough to detect mechanically, that understanding can survive the current agent session and be checked during subsequent development.

WT is not an LLM reviewer. An external agent can reason about a defect and author a rule; WT itself does not call a model, discover bugs autonomously, or decide whether a proposed architectural convention is correct.

The complete supported workflow is:

```text
Understand a defect
    -> write a documented rule and examples
    -> validate and run the rule
    -> inspect other matches and legitimate alternatives
    -> refine the rule without losing its regression examples
    -> enforce the validated rule
    -> maintain, revise, or retire it explicitly
```

### 1.1 Required product properties

- Operates as a CLI without an account, service, database server, daemon, or internet connection.
- Combines user-global rules and repository-local rules by default.
- Stores authoritative rules in readable, version-control-friendly files.
- Supports JSON input and output without interactive prompts, making it usable by any coding agent that can execute commands.
- Runs bounded, deterministic checks. Rules cannot modify the repository or access arbitrary operating-system capabilities.
- Is language-agnostic at its core: files, paths, strings, regular expressions, spans, conditions, and bounded iteration.
- Executes file-centrically and shares equivalent expensive native queries across independently maintained rules.
- Defines its own inspectable `.wt` language contract; valid arbitrary Rhai is not automatically valid WT.
- Distinguishes actual reported violations, cases requiring review, excluded files, and failed analysis.
- Preserves rule documentation, scope, limitations, positive examples, and counterexamples.

### 1.2 Explicit non-goals

WT does not provide autonomous LLM orchestration, cloud collaboration, model training, a marketplace, a general-purpose programming environment, an automatic code fixer, a type checker, whole-program verification, or a guarantee of convergence to correct software. It does not replace behavioral regression tests or better application abstractions.

There is no required editor extension, MCP server, pull-request bot, or graphical interface. Such integrations can invoke the CLI without changing its core contract.

## 2. Design decisions

| Area | Decision |
|---|---|
| Implementation | Rust CLI and native matching/runtime functions. |
| Rule language | Watchtower Rule Language v1 (`wt-rule-1`), a restricted public contract with Rhai as its parser/residual backend. |
| Main matching mechanism | Compiled Rust regular expressions and string operations. |
| Additional structure | Explicit, versioned, read-only syntax helpers; never an implicit semantic-analysis promise. |
| Agent interchange | Self-contained JSON rule submissions. |
| On-disk representation | JSON manifest plus a plain-text `.wt` code file and test fixtures. |
| Default new-rule policy | Enabled, advisory. It runs immediately but does not block ordinary checks. |
| Enforcement | Explicit `enforced` mode, with passing positive and negative fixtures required. |
| Default checking | Whole current repository; all enabled global/local rules, one file-centric shared execution plan. |
| Optimization boundary | Share pure queries, not rule state, policy, diagnostic identity, or approval. |
| Incrementality | Per-rule content-verified caches; `--changed` is explicit partial coverage. |
| Default ignores | Honor Git ignore rules for untracked files; do not accidentally omit tracked files. |
| Automatic fixes | None. Findings describe the expected change. |
| Trust boundary | Approved policy and final CI invocation must be protected outside the editing agent's control. |

Rhai supplies parsing and residual interpretation, not WT's language contract or query optimizer. WT validates a positive syntax/API subset and owns query extraction, execution sharing, and limits. Rust regex supplies native matching; neither embedding Rhai nor using Rust makes arbitrary nested work automatically cheap. [R1, R2, R3]

## 3. Configuration directories and repository discovery

### 3.1 Global directory

Resolve the global directory in this order:

1. Explicit `--global-dir PATH`.
2. Absolute `WT_CONFIG_HOME` environment variable.
3. On Unix-like systems, absolute `$XDG_CONFIG_HOME/wt` when `XDG_CONFIG_HOME` is set; otherwise `$HOME/.config/wt`.
4. On Windows, `%USERPROFILE%\.config\wt`, unless explicitly overridden above.

A relative environment override is an error rather than a path relative to an accidental working directory. The Unix default follows the XDG convention; the Windows path is a deliberate WT product decision for consistency with its dot-directory layout. [R4]

Do not create global directories merely because the user runs `wt --help`, `wt check`, or another read-only command. `wt init --global` and a successful global mutation create them as needed.

### 3.2 Local directory and scan root

The local configuration is `<root>/.wt/`.

Determine `root` as follows:

1. `--root PATH`, when provided.
2. Otherwise, the nearest enclosing Git worktree root, including worktrees whose `.git` entry is a file.
3. Outside Git, the nearest ancestor containing `.wt/`.
4. Otherwise, the current working directory.

An explicit root inside a Git worktree is permitted. Inherited Git ignore rules between the worktree root and the explicit root still apply, but local WT configuration is loaded only from the explicit root.

Running `wt check` from a subdirectory normally checks the repository, not just that subdirectory. Positional paths narrow the scan; they do not silently change the rule roots. Resolve positional paths relative to the invocation directory and require them to stay within the selected root.

Nested `.wt/` directories do not cascade. They are separate configurations only when explicitly selected as the root. An enclosing Git repository takes precedence over a nested `.wt/` during automatic discovery. This prevents surprising policy changes based on the caller's current directory.

### 3.3 Layout

```text
~/.config/wt/                         # illustrative global location
  config.json
  rules/
    no-retired-endpoint/
      rule.json
      check.wt
      tests.json
      fixtures/

repository/
  .wt/
    config.json
    waivers.json                      # optional; accepted violations only
    rules/
      template-number-decimal-step/
        rule.json
        check.wt
        tests.json
        fixtures/
          missing-step.tsx
          valid-conditional.tsx
          ...
  frontend/
  backend/
```

Commit local manifests, code, fixtures, configuration, and deliberate waivers. Caches are derived data and live in the platform cache directory, not the rule directory.

### 3.4 Missing directories and discovery errors

Missing global or local directories are normal. Existing unreadable or malformed configuration is an error. Never silently replace an unreadable configuration with defaults.

Rule directories are immediate children of `rules/` with a valid rule ID. Temporary dot-prefixed directories are ignored. An incomplete non-temporary rule directory is an error, not an invisible rule.

## 4. Configuration composition and identities

### 4.1 Qualified rule identities

A rule's `id` must match:

```regex
^[a-z][a-z0-9]*(?:[-.][a-z0-9]+)*$
```

IDs cannot contain slashes, traversal components, or arbitrary filenames. A directory name must equal its manifest's ID.

The effective identity is `global/<id>` or `local/<id>`. Both rules execute when the same unqualified ID exists in both scopes. There is no automatic local shadowing of a global detector.

Commands may accept an unqualified ID only if it resolves uniquely. Ambiguity is a structured error listing the candidates. Unknown IDs are errors.

### 4.2 Configuration merge rules

Precedence is built-in defaults, global configuration, local configuration, then explicit CLI options.

- Scalar settings use the last explicitly supplied value.
- Scan exclusion entries are additive, preserving origin and reason.
- Rules from both directories are combined, not replaced wholesale.
- Mode overrides are keyed by qualified rule ID and require a reason.
- An override of an unknown rule is an error, catching stale settings and typos. Overrides targeting a whole scope explicitly omitted by `--no-global` are instead reported as inactive; they must not make the documented local-only CI command fail.
- Unknown configuration fields and duplicate JSON keys are errors.
- Runtime limits cannot exceed the binary's documented absolute safety ceilings.

`wt config --format json` reports the effective configuration and the origin of every overridden value. `wt list` reports both the declared and effective mode of each rule.

Example local configuration:

```json
{
  "schema_version": 2,
  "scan": {
    "respect_gitignore": true,
    "honor_git_local_excludes": true,
    "include_hidden": true,
    "max_file_bytes": 4194304,
    "exclude": [
      {
        "glob": "frontend/generated/**",
        "reason": "Generated output; the source generator is checked separately."
      }
    ]
  },
  "rules": {
    "mode_overrides": {
      "global/no-retired-endpoint": {
        "mode": "advisory",
        "reason": "This repository still has a reviewed compatibility migration."
      }
    }
  }
}
```

The bundled `schemas/config.schema.json` fixes supported scan/runtime/optimizer keys, defaults, and absolute ceilings; `docs/runtime-configuration.md` lists their numeric values. JSON Schema does not replace cross-field memory-budget validation. Missing fields receive documented defaults. Configuration changes that reduce coverage must be visible in `wt config` and the JSON check result's effective-policy summary.

## 5. File selection and ignore behavior

### 5.1 Selection contract

A check examines existing working-tree bytes, not index contents or committed blobs. Both tracked and untracked files are eligible. There is no implicit staged-only or changed-lines-only mode.

The selection pipeline is:

```text
root boundary and mandatory exclusions
    -> Git tracked/untracked classification
    -> Git ignore decision for untracked files
    -> explicit WT exclusions and requested paths
    -> each rule's repository applicability and path scope
    -> supported text-file eligibility
```

Every exclusion has a machine-readable reason. `wt explain PATH` identifies the decisive rule or filter and its source.

### 5.2 Git ignores

By default:

- Honor applicable `.gitignore` files, including nested files and Git's negation/precedence rules.
- Honor the current repository's Git-local excludes and user excludes when `honor_git_local_excludes` is true.
- Continue to scan tracked files even if an ignore pattern matches their names. Git ignore rules describe intentionally untracked files; already tracked files are not affected by them. [R5]
- Outside Git, evaluate `.gitignore` files within the scan root using the same pattern syntax, but do not consult a nonexistent index or parent rules outside that root.
- Do not implicitly honor unrelated `.ignore` or `.rgignore` files.

The implementation must not simply accept a walker's default settings. For example, the `ignore` crate defaults to filtering hidden files and exposes separate switches for Git and parent ignore behavior. WT's own semantics take precedence. [R6]

`--no-host-ignores` disables only Git-local and user-global ignore sources. Repository `.gitignore` files remain active. This supports less machine-dependent CI scans.

### 5.3 `--include-ignored`

`wt check --include-ignored` bypasses Git ignore filtering for this invocation. It does not bypass root boundaries, WT exclusions, rule scope, mandatory metadata exclusions, encoding requirements, or resource limits.

It does not recursively enter arbitrary symlink targets or embedded repositories.

`.venv`, `node_modules`, `.npm`, and similar directories are not magical WT exclusions. They are skipped when the repository or user ignore configuration excludes them. A repository that has not ignored its dependency directories must explicitly exclude them or expect WT to scan them.

No silent fallback directory-name blacklist is permitted. This avoids hiding legitimate project files just because their names resemble common dependencies.

### 5.4 Other filesystem behavior

- Include ordinary hidden files by default, subject to the same filters as other files.
- Never scan Git internals or WT rule/configuration storage as application source. Fixtures run through the rule-test runner instead.
- Do not follow symlinks or Windows junctions/reparse-point directory links. A tracked source symlink relevant to a rule is reported as an analysis gap, not followed outside the root.
- Do not descend into submodules or nested Git worktrees automatically. Report these boundaries. Check them as separate explicit roots.
- Detect binary data with a bounded initial probe of up to 8 KiB and, for files loaded within the size limit, the remaining bytes. A detected NUL classifies the file as outside the default text domain and counts as a binary skip. An unclassified oversized file remains an analysis gap; WT need not read an arbitrarily large file just to decide whether it is binary. Do not claim binary content was checked.
- An eligible, non-binary file with invalid UTF-8, an unreadable file, or an oversized file is an analysis gap and makes the check incomplete.
- The default maximum source-file size is 4 MiB. Users can raise it explicitly within the binary's ceiling or add a reasoned exclusion.
- A disappearing or changing file must be reported as unstable if detected. Hash the bytes actually analyzed and return that digest; a normal working-tree scan is not an atomic snapshot of a concurrently edited repository.

Normalize reported paths to root-relative forward-slash notation without changing their case. Reject absolute or traversing paths in rule scopes and fixtures. Do not use lossy path conversion to conflate distinct files; unsupported non-Unicode paths produce an explicit analysis gap when relevant.

Globs are case-sensitive, root-relative, and use `/`: `*` stays within a component, `?` matches one non-separator character, and `**` spans zero or more complete path components. Exclusions win over includes. Gitignore syntax and WT scope globs are separate documented syntaxes.

## 6. Rule representation

### 6.1 Authoritative package

A rule consists of:

1. `rule.json`: human-readable description, metadata, scope, declared patterns, diagnostic definitions, and code reference. Pattern names are local aliases, not user-maintained global query identities.
2. `check.wt`: the detector's plain-text source.
3. `tests.json` and optional fixture files: expected violations, accepted forms, and reviewed edge cases.

`check.wt` is the required detector filename. It contains a top-level WRL1 body, not a user-declared entrypoint. The authoritative manifest remains JSON for machine tooling, while detector comments and prose fields make the rule human-readable.

There is exactly one authoritative code source. A disk manifest references `check.wt`; it cannot also contain an inline copy. WT never requires an opaque database or compiled artifact to edit a rule.

### 6.2 Manifest fields

| Field | Requirement and meaning |
|---|---|
| `schema_version` | Required integer `2`. |
| `id` | Required stable ID, unique within its scope. |
| `title` | Required concise human-readable name. |
| `description` | Required statement of what the rule checks. |
| `rationale` | Required explanation of why the constraint matters. |
| `mode` | `advisory`, `enforced`, or `disabled`; defaults to `advisory` on submission. |
| `severity` | `error`, `warning`, or `info`; diagnostic importance, not an independent enforcement switch. |
| `execution` | `file` or `repository`; defaults to `file`. |
| `scope.include` | Required nonempty list of root-relative globs. |
| `scope.exclude` | Optional list of `{glob, reason}` entries. |
| `scope.require_files` | Optional literal root-relative paths that must all exist for repository applicability. |
| `patterns` | Optional named string/object regex definitions with static flags; aliases resolve before cross-rule query interning. |
| `diagnostics` | Nonempty map of diagnostic IDs to `{kind, message, help}`. |
| `limitations` | Required list; may be empty only when justified by the detector's narrow contract. |
| `code` | Required `language: "wt-rule-1"`, explicit capability list, and either disk `file: "check.wt"` or interchange `source`. |
| `tests_file` | Disk-only relative path to a test suite, when present. |
| `metadata` | Optional author, tags, original defect description, source locations, and other informational JSON. |

Informational metadata is never executable and cannot override core fields. Do not use a numeric “confidence” value as a substitute for fixture evidence or human review.

Relative package file references must stay within that rule package after canonicalization. Reject symlink-based escapes. Read each referenced file into a bounded immutable buffer before using it.

### 6.3 JSON submission versus disk form

`wt new` accepts a self-contained submission containing `code.source` and optionally a `tests` object with inline fixture content. It does not read arbitrary file references embedded in the submitted JSON.

On success, WT writes a readable package and replaces `code.source` with `code.file` in the disk manifest. The test suite may keep inline content; optional fixture files are supported for manual editing and export/import round trips.

`wt show ID --format json` returns the self-contained interchange form, resolving code and fixture references. Human text output displays the explanation, scope, limitations, effective mode, and test summary.

JSON is strict UTF-8 JSON: no comments, trailing commas, duplicate keys, or unknown structural fields. Explanatory prose belongs in description, rationale, limitations, or metadata. An external `README.md` can be present, but it is not a second authoritative rule definition.

## 7. Watchtower Rule Language and host API

### 7.1 Public language contract: `wt-rule-1`

A detector is a **Watchtower Rule Language v1 (WRL1)** program stored in `check.wt`. It is not an arbitrary Rhai script. The `.wt` extension communicates the contract; enforcement comes from WT's compiler, capability checks, and runtime, not the filename.

The manifest declares `code.language: "wt-rule-1"`. This language/API version is independent of the specification revision, JSON `schema_version`, Rhai dependency version, and optimizer implementation. Unknown language versions are validation errors. A backend upgrade must not silently expand WRL1 or change its established meaning.

A file rule is a top-level statement body with one implicit, read-only `file` binding:

```wt
// Pattern IDs refer to definitions in this rule's manifest.
for matched in rx::find_all(file, "retired_endpoint") {
    emit(matched.span, "retired-endpoint");
}
```

There is no `fn check(file)` wrapper. A repository rule has an implicit `repo` binding instead and can obtain authorized file handles through `repo.files()`. It does not gain filesystem access. Do not expose a mutable `rule` object, other rules' outputs, global configuration, environment, or the execution plan to detector programs.

Rule authors write readable local logic. WT extracts the analyzable queries and shares their execution automatically; authors do not declare dependencies on other rules or hand-maintain query IDs.

### 7.2 Supported language subset

The following is the complete construct whitelist; anything else is rejected even when Rhai would accept it.

| Construct | WRL1 contract |
|---|---|
| Comments | `//` line comments and `/* ... */` comments; no executable directives in comments. |
| Bindings | `let name = expression;`, `const name = literal;`, lexical block scope. Host/reserved names cannot be shadowed. |
| Reassignment | `name = expression;` for an existing local scalar binding only; no field, index, collection, or host-object mutation. |
| Values | Booleans, signed 64-bit integers, double-quoted escaped strings, unit `()`, and read-only WT host values. No floats, character literals, raw/interpolated strings, array literals, or map literals. |
| Operators | `!`, integer unary `-`, integer `+ - * / %`, string `+`, `== != < <= > >=`, `&& ||`, and grouping parentheses, subject to valid operand types. No implicit numeric/string conversions. |
| Conditions | Statement `if condition { ... } else { ... }`; conditions must be Boolean. `else if` is allowed. No expression-valued blocks or conditionals. |
| Iteration | `for item in sequence { ... }` over a finite immutable WT sequence; nested loops allowed within budgets. No numeric ranges, arbitrary iterators, string-character loops, or index-counter syntax. |
| Loop control | `break;` and `continue;` within the current loop. |
| Early completion | `return;` exits only this rule invocation. No return value. |
| Calls | Only statically named functions/methods in the requested WT capability registry; method receivers are checked. |
| Properties | Read-only documented host properties. No reflection or computed property names. |

Reject user-defined functions, recursion, closures/function pointers, `while`, `do`, `loop`, imports/modules, `eval`, `switch`, `try`/`catch`, `throw`, custom operators, compound assignment, destructuring, arbitrary indexing, and every unregistered function. This is a deliberate language boundary, not an instruction merely asking agents to avoid those features.

A `const` is an immutable scalar literal binding, not an arbitrary function evaluated during compilation. Identifiers used as pattern IDs, diagnostic IDs, attribute names, capture names, and path-glob arguments must be string literals or such constants. Computed pattern selection, including selection from file contents, is rejected. Dynamic text operands remain useful: a rule may construct an expected string and compare it with a captured expression, as the decimal example does.

`&&` and `||` evaluate left to right with short-circuit semantics. Calls and their operands otherwise evaluate in source order. Operations on a missing optional host value fail unless a guard avoids them. Integer overflow and division by zero are runtime errors, never wraparound or successful no-match results.

Conceptual statement grammar:

```text
program     := statement*
if_statement := "if" expression block ("else" (block | if_statement))?
statement   := "let" IDENT "=" expression ";"
             | "const" IDENT "=" literal ";"
             | IDENT "=" expression ";"
             | "if" expression block ("else" (block | if_statement))?
             | "for" IDENT "in" expression block
             | "break" ";" | "continue" ";" | "return" ";"
             | host_call ";"
block       := "{" statement* "}"
```

Expressions consist only of the values, operators, documented properties, and calls above. Operator precedence and string-escape handling are fixed by the pinned WRL1 frontend and covered by conformance tests; arithmetic binds before comparisons, comparisons before `&&`, and `&&` before `||`. No backend-only syntax is implicitly admitted.

Finite loops do not imply cheap execution: nested loops and repeated native calls can still be expensive. Budget exhaustion is an analysis failure, not a language-level proof that every program is efficient.

### 7.3 Rhai as a replaceable backend

Use Rhai for parsing and residual control-flow execution, with a WT-owned validation/lowering layer in between:

```text
check.wt + manifest
    -> bounded Rhai parsing, with optimization disabled
    -> fail-closed WRL1 syntax / binding / capability validation
    -> WT-owned Rule IR, call-site IDs, source map, guarded query templates
    -> cross-rule query interning and physical planning
    -> approved residual control flow + Rust query service
```

Rhai exposes AST walking and internal node access through its `internals` feature. This is a real implementation dependency: isolate the adapter, pin the dependency, test all accepted/rejected constructs, and do not assume every internal AST API is a stable public compiler framework. [R11]

Validate the **unoptimized** syntax tree and every source branch, including unreachable code. Otherwise dead-code elimination could hide forbidden syntax. Reject unknown AST node variants. Disable forbidden symbols in the parser where supported, but do not substitute a blacklist or source regex for the positive AST/call whitelist. Rhai provides symbol disabling, and its full optimization mode can evaluate functions; WT must not execute detector host functions while validating or extracting queries. [R12, R13]

The adapter may use a separate compile-only engine to resolve syntax without binding real source data. Residual code can run through the validated Rhai AST with registered host calls routed to the query service; there is no requirement to invent a new VM. WT's IR and logical cost accounting remain authoritative. Never concatenate or merge separate rule ASTs to obtain sharing: Rhai's AST merge appends statements and can replace same-named functions; it is not a cross-rule query optimizer. [R11]

Only explicitly registered native functions are available. `emit` is a rule-local output effect; matching and text/helper operations are read-only queries. Disable built-in I/O/debug output, module resolution, dynamic evaluation, and any ambient filesystem/network/process/clock/random access. Never enable Rhai's `unchecked` feature in an enforcement build. Configure limits; Rhai's operation limit is otherwise unlimited and does not by itself bound the cost of a native function. [R2]

WRL1 diagnostics identify the rule file, original source span, stable WT error code, unsupported construct, and allowed alternative. Example:

```text
WT102 check.wt:4:1: `while` is not part of wt-rule-1.
Use `for` over a finite WT sequence such as rx::find_all(...).
```

Required compiler error families include `WT100` syntax, `WT101` unsupported language version, `WT102` forbidden construct, `WT103` non-static identifier argument, `WT104` unregistered call/capability, `WT105` unsupported mutation/iteration, and `WT106` unknown pattern/diagnostic ID. Raw backend errors may be included as details, not used as the sole public contract.

### 7.4 Text and matching API

These are **WT host APIs**, not built-in Rhai functions. Pattern calls refer to declarations local to the rule. Strings/views and sequences are read-only; derived values retain bounded memory accounting.

| API / value | Behavior |
|---|---|
| `file.path`, `file.text`, `file.span` | Normalized root-relative path, original UTF-8 source view, and whole-file span. |
| `text::lines(file)` | Finite sequence of `{text, span}` lines in source order. A terminator is excluded from a line's text/span; an ending terminator does not create an extra final line. |
| `text::contains(text, needle)` | Case-sensitive exact substring presence; an empty needle is true. |
| `text::starts_with(text, prefix)`, `text::ends_with(text, suffix)` | Exact, case-sensitive tests. |
| `text.contains(needle)` | Approved alias of `text::contains(text, needle)` for String/TextView receivers; lowered to the same IR operation. |
| `text::trim(text)` | New text value with leading/trailing Unicode whitespace removed; no reportable span. |
| `text::split(text, separator)` | Finite sequence of new strings split on a nonempty literal separator; preserves empty fields; empty separator is an error. |
| `text::replace(text, from, to)` | Replace all non-overlapping occurrences of nonempty `from`; returns a bounded new string without a source span. |
| `text::len_bytes(text)` | UTF-8 byte length; not a character/display-column count. |
| `path::matches(file.path, glob)` | WT scope-glob semantics; glob literal/constant is compiled at rule-load time. |
| `rx::is_match(file, pattern_id)` | Boolean existence on the whole original file. Does not enumerate all matches. |
| `rx::find_all(file, pattern_id)` | Finite ordered sequence of all non-overlapping matches and named captures on the whole file. |
| `rx::find_in(span, pattern_id)` | Same operation on that span as an independent substring; returns absolute original-source coordinates. |
| `rx::capture_text(pattern_id, text)` | First match and named captures in a text value, or unit `()`; captures have text but no reportable source span. |
| `match.text`, `match.span` | Matched source view and original-source span for `find_all`/`find_in`. A capture-text result does not have `.span`. |
| `match.group_text(name)` | Named capture as text, or unit `()` when an optional group did not participate. Unknown declared capture name is a validation error. |
| `sequence.len`, `sequence.is_empty()` | Number of elements and emptiness of a WT sequence. Sequences have independent cursors for each consumer. |
| `emit(span, diagnostic_id)` | Append a declared diagnostic to this invocation's output, attributed to the current rule. |
| `repo.files()` | Repository-rule-only bounded sequence of authorized snapshot file handles, sorted by path. Not an independent filesystem walk. |

`text.v1` supplies the above surface. A rule can iterate only over sequences produced by an authorized API. There is no API to read a path supplied by script text, open a file by name, mutate a host handle, construct a reportable span, or obtain another rule's match cursor.

Optional values use unit `()`: unit equals only unit; comparing a missing value with any present text/scalar/host value yields false for `==` and true for `!=`. Equality of strings and TextViews compares their UTF-8 values, not object identity. Strings and TextViews are both valid text operands for comparisons and concatenation; concatenation returns a bounded owned string, not an original-source view. Collections and arbitrary host objects are not comparable except for documented unit checks. WT validates known operand types and checks remaining host types at runtime; a missing-field/type error is never interpreted as a negative match.

Pattern declarations support a string shorthand or an explicit form:

```json
{
  "patterns": {
    "request": "request\\([^\\r\\n]*\\)",
    "legacy": {
      "regex": "old_endpoint",
      "flags": {"case_insensitive": true}
    }
  }
}
```

Explicit flags are `case_insensitive`, `multi_line`, `dot_matches_new_line`, `ignore_whitespace`, and `crlf`, all defaulting to false. Unicode-aware UTF-8 matching is the baseline. Rust regex inline flag syntax is preserved in the expression; a pattern that permits invalid UTF-8 matches is invalid for WT's text domain. String shorthand expands to the same definition with default flags. Unknown flags are errors.

Use pinned Rust `regex` semantics: leftmost-first, non-overlapping iteration for each separate expression, named captures, and UTF-8 boundaries. Zero-width matches are permitted and must advance according to the engine's UTF-8-aware iteration semantics. No backreferences or look-around. Individual searches and repeated match iteration have different worst-case behavior; WT makes no unconditional linear-time claim. [R3]

`find_all` and `find_in` return a complete sequence within the configured limits or fail; never silently truncate. A `break` in the caller does not turn `find_all` into a first-match query. Use `is_match` when only existence is needed. Host implementations may use internal lazy storage only if complete-or-error behavior and limit checks remain identical.

Pattern IDs must resolve before scanning. **Deduplicate by the resolved expression, expanded flags, and engine semantics—not the local name.** The same expression under two names can share work; the same name with different expressions cannot. Distinct matching operations, regions, captures, and boundary semantics are not automatically equivalent (section 13).

### 7.5 Spans and findings

A reportable span is an opaque handle tied to one original file snapshot. A rule cannot forge a path, fabricate offsets, or emit for a file outside its declared inputs.

`start_byte` is zero-based and inclusive; `end_byte` is exclusive. Both are UTF-8 boundaries. Human line and column numbers are one-based; columns count Unicode scalar values, not bytes, display cells, or UTF-16 units. JSON includes byte spans and the declared display-coordinate convention.

Preserve CRLF and BOM bytes in snapshots and hashes. Derived strings and helper-canonicalized expressions have no reportable coordinates unless an explicit API maps them back. In particular, `rx::capture_text` must not manufacture an original span from a temporary string.

`rx::find_in` treats its input span as an independent haystack: anchors and boundaries refer to that slice. It is not equivalent to filtering whole-file matches to the same offsets. Query identity must record that distinction. Shared result storage may reuse offsets, but exposes fresh authorized handles and immutable per-consumer cursors.

### 7.6 Optional syntax helpers

The core remains text-first and language-agnostic. A rule must explicitly request a versioned helper capability such as `jsx.v1`. Helpers are bundled/trusted code, not arbitrary repository plugins. A missing required helper is a validation error.

`jsx.v1` is the reference TSX parsing adapter:

- `jsx::inputs(file)` yields actual direct lowercase `<input>` elements in source order, excluding strings/comments, with an immutable `span`.
- `input.attr(name)` returns the first lexical occurrence or unit; `input.has_spread` and `input.duplicate(name)` expose unresolved ordering/duplicates separately.
- Attributes provide `kind` (`string`, `expression`, or `boolean`), `canonical`, and optional `static_string`.
- `canonical` joins lexical expression tokens with one ASCII space, removes comments, and encodes decoded string-literal values as JSON strings. It does not evaluate expressions, resolve identifiers, or erase meaningful grouping tokens.
- `static_string` exists only for a literal string attribute or a braced string-literal expression, not a helper call or computed runtime value.

A demanded parse failure is an analysis error, not an empty input list. Parser/helper version and parse mode are query inputs. Parse once per demanded helper version and file-local transaction, then share read-only structure with authorized rules. Do not eagerly parse a file whose controlling branch never requests the helper.

This is not a promise that a regex such as `<input[^>]*>` parses JSX. Existing parsing technology can implement the helper. [R7]

### 7.7 Execution isolation and resource budgets

Every logical rule/file invocation has fresh local variables, local accounting, and a private diagnostic sink. Repository invocations also have isolated state. Shared queries do not create rule order, cross-rule mutable state, or a way to change another rule's result.

Budgets have two distinct purposes:

1. **Logical per-rule limits** charge requested operations and result sizes independently of cache hits or other rules. Sharing cannot make an otherwise over-budget rule pass merely because it ran second.
2. **Physical executor limits** bound compiled automata, worker memory, shared results, and native calls. They protect the process even when many individually small rules share a worker.

Default resource profile (design values, not measured performance claims):

| Budget | Default |
|---|---:|
| Detector source per rule | 128 KiB |
| Source file | 4 MiB |
| Regex source / compiled expression | 8 KiB / 1 MiB |
| Patterns per rule | 128 |
| Logical WRL steps per file invocation | 1,000,000 |
| Logical native input bytes per file invocation | 128 MiB |
| Returned regex matches / other sequence elements per call | 10,000 / 100,000 |
| Diagnostics per rule/file | 1,000 |
| Local temporary values per invocation | 64 MiB |
| WRL nesting depth / local bindings per invocation | 64 / 256 |
| Retained derived results per file-local transaction | 64 MiB |
| Worker memory scheduling budget | 256 MiB |
| Combined executor scheduling budget, all workers and parent | 1 GiB |
| Residual invocation / individual native-call watchdog | 2 seconds / 2 seconds |
| Repository invocation watchdog | 30 seconds |

Repository limits are 10,000 input files, 100 MiB of retained source snapshots, 10,000,000 logical steps, and 1 GiB of logical native input bytes. The coordinator must check total memory reservations before dispatch. Default jobs are at most available CPUs and at most the number admitted by the total memory budget; explicit `--jobs` cannot bypass memory safety.

WRL1 logical cost is one step per executed statement (excluding a bare block), evaluated literal/local reference/property access, operator or host call, and visited loop element; operand evaluations are charged separately. A `for` statement is charged once for entering it and once per visited element in addition to the executed body. Short-circuited operands and untaken branches are not charged. Native queries additionally charge their input bytes (whole file, independent source slice, or the sum of text operands as appropriate); pattern/diagnostic IDs are not haystacks. This is a versioned WT cost table, not whatever operation-count definition a future Rhai release chooses. Charge native input bytes and returned elements on each logically demanded request, including a cache hit. Cache admission includes the effective resource profile and validated usage summary. The backend's own operation limit is additional defense, not the portable cost definition. [R2]

Frontend validation is also bounded (AST-node count and watchdog); its configured defaults and ceilings are listed in the runtime configuration reference. Use persistent worker processes, not one process per rule/file pair. Assign a complete file-local transaction to one worker so its applicable rules share the same snapshot and query arena. A worker compiles the required residual programs and matcher objects once per loaded version; separate processes may each have a compiled instance. Do not promise one physical regex object shared across process address spaces.

Per-consumer budget rejection must not poison an otherwise valid shared query. A query-wide failure is retained as an error and attributed to every consumer that actually demands it; it is not an empty cache entry. If a worker is killed, completed invocation results may be retained, but unfinished consumers are incomplete and the overall exit is `2`. Optional optimizer-group construction failure may instead fall back to individual queries, preserving semantics.

Bound all parsing, compiler work, IPC, text transformations, and native calls. Memory scheduling values are not themselves hard OS isolation; apply available process controls and kill/recover runaway workers. A timeout is a fail-safe, not a semantic predicate. Successful results must agree across valid physical plans; host-dependent exhaustion always remains explicit. This is defense in depth, not a formally proven hostile-code sandbox.

## 8. Modes, validation, and retained examples

### 8.1 Rule modes

| Mode | Executes in ordinary `wt check`? | Findings block? |
|---|---|---|
| `advisory` | Yes. | No, unless `--strict` is requested. |
| `enforced` | Yes. | Yes. |
| `disabled` | No. | No. Its exclusion remains visible. |

New rules default to advisory so an agent can investigate repository-wide matches before enforcing a hypothesis. A submission can request enforced mode only if it satisfies the same test requirements as `wt set-mode ID enforced`.

Severity expresses importance. Mode determines whether findings block. Neither mode nor severity changes whether execution failures are reported as incomplete.

Diagnostics have two kinds:

- `violation`: the detector recognized a form prohibited by its documented rule.
- `review`: the detector encountered a relevant form it deliberately refuses to classify as acceptable.

Both block under enforced mode. A review diagnostic must explicitly say what is unresolved, rather than falsely asserting a confirmed runtime bug.

### 8.2 Validation

`wt validate` checks manifests, strict JSON, IDs, safe file references, globs, regex compilation, unoptimized WRL1 syntax/profile, capabilities, static pattern/diagnostic references, safe bounded-iteration sources, query extraction, and fixture structure.

`wt test` runs detector fixtures without running or importing the application code. Fixtures are text inputs, not shell commands. Explicitly testing a rule without a suite returns `2` with `missing_test_suite`; it does not invent a passing test result. Ordinary checking may still run an advisory rule with no tests.

Every enabled rule's supplied test suite must pass before repository execution. Tests run against the same WRL1/query semantics as repository checks; reference/optimized equivalence is separately tested by engine conformance cases. Cache this result by its complete semantic/test digest. A rule with no suite may remain advisory; an enforced rule must have at least one expected diagnostic and one nonempty valid input with no expected diagnostic. A no-op or empty-file negative case alone is insufficient.

These mechanical requirements do not prove that the rule or examples are correct. The reviewer remains responsible for validating the underlying constraint.

### 8.3 Fixture format

```json
{
  "schema_version": 2,
  "cases": [
    {
      "name": "missing-step",
      "files": [
        {
          "path": "frontend/src/task-start-page.tsx",
          "fixture": "fixtures/missing-step.tsx"
        }
      ],
      "expect": [
        {
          "path": "frontend/src/task-start-page.tsx",
          "code": "decimal-step",
          "kind": "violation"
        }
      ]
    }
  ]
}
```

A file has exactly one of inline `content` and package-relative `fixture`. Interchange submissions contain only `content`.

Fixtures execute with normal rule applicability and path-scope semantics but without ambient global/local scan exclusions, repository ignores, or waivers. Repository rules see only the fixture case's virtual file set. Case names and virtual paths must be unique in their respective namespaces.

Expected diagnostics are a multiset of `(path, code, kind)`, optionally constrained by byte spans. Extra diagnostics fail the test. Missing diagnostics fail the test. An execution error fails an ordinary detector fixture; it cannot satisfy an expected finding. Engine conformance tests for expected compile/runtime failures are separate from these rule examples.

Retain the original defect, its correction, meaningful variations, and validated false positives. Rule updates must not silently discard existing cases through the CLI; removal or replacement of previous fixture identities requires `--allow-test-removal` and a reason. Direct file edits remain possible and are reviewed through version control.

## 9. CLI contract

All normal commands are non-interactive. No confirmation prompt is used for creation, inspection, validation, or checking. Destructive/replacing mutations require explicit arguments instead of an interactive prompt.

Common options are `--root PATH`, `--global-dir PATH`, `--format text|json`, and `--color auto|always|never`. Options irrelevant to a command are rejected rather than ignored. Shell quoting is the caller's responsibility; JSON via stdin is the recommended agent interface.

### 9.1 Commands

| Command | Contract |
|---|---|
| `wt init [--global]` | Create a minimal configuration/rules directory. Idempotent; never overwrite existing files. |
| `wt new JSON` | Create one local rule from a self-contained JSON argument. |
| `wt new --stdin [--global]` | Read one complete JSON submission from stdin. |
| `wt new --file PATH [--global]` | Read the same submission from a file. |
| `wt check [PATH ...]` | Run all enabled rules from both scopes using the shared file-centric plan. |
| `wt plan [--rule ID]` | Inspect static guarded query templates, aliases, consumers, and planned sharing without reading application source or claiming execution. |
| `wt list` | List rules, origins, mode overrides, scope, and test availability. |
| `wt show ID` | Explain a rule; JSON output is a self-contained export. |
| `wt validate [ID]` | Validate selected or all discovered rule packages. |
| `wt validate --file PATH` | Validate an interchange submission without creating it. |
| `wt test [ID]` | Execute selected or all discovered fixture suites. |
| `wt update ID --stdin --expect-hash HASH` | Replace the selected package atomically, retaining identity and protecting against concurrent edits. |
| `wt set-mode ID advisory|enforced|disabled --reason TEXT` | Change mode explicitly; enforcing first validates and passes the required fixtures. |
| `wt explain PATH [--rule ID]` | Explain file selection, rule scope, and skip reasons without executing application code. |
| `wt config` | Display effective configuration and value origins. |
| `wt schema rule|submission|tests|config|result|plan|waivers` | Print the corresponding versioned JSON Schema. |
| `wt cache clear` | Remove WT-derived cache entries, never source or rule files. |

There is no `wt fix` and no built-in agent/model configuration.

### 9.2 Creation and update behavior

Exactly one submission source is permitted: positional JSON, `--stdin`, or `--file`. Reject a missing source and reject multiple sources.

`wt new` defaults to local scope and creates `.wt/` lazily. `--global` selects global storage. Creation never silently overwrites an existing ID.

Validate the complete submission, compile the detector, and execute any supplied tests before committing a new package. A failed creation leaves no partially active rule. New advisory rules may omit tests, but receive a visible `untested` notice.

Use bounded input reads, a temporary sibling directory, and an atomic replacement strategy with crash recovery appropriate to the platform. Lock a target rule during mutations. `--expect-hash` compares the current complete package digest, not only the code file or an agent-supplied version number.

A successful JSON mutation response contains its qualified ID, resulting package path, mode, digest, and validation/test outcome. It must not claim that the rule has been semantically approved by a person.

### 9.3 `wt check` options

| Option | Meaning |
|---|---|
| `--include-ignored` | Bypass Git ignore filtering only. |
| `--no-host-ignores` | Do not consult Git-local or user-global exclusion files; retain repository `.gitignore` files. |
| `--no-global` | Do not load global rules/configuration. |
| `--rule ID` | Repeatable rule selection; IDs must exist. |
| `--strict` | Also make advisory diagnostics blocking. |
| `--no-cache` | Bypass all persistent derived caches; in-run sharing remains active unless the optimizer is disabled. |
| `--optimizer auto|off` | Default `auto`; `off` uses the unshared correctness reference path, not different rule semantics. |
| `--changed` | Explicit Git changed-file selection; partial coverage, with complete dependencies retained for repository rules. |
| `--base REV` | Base Git revision for `--changed`; defaults to `HEAD`, never fetches remote data. |
| `--jobs N` | Positive worker count; default is bounded by available CPUs and memory budget. |
| `--max-file-bytes N` | Explicitly adjust source-file eligibility within documented ceilings. |
| `--show-suppressed` | Include full waiver-suppressed diagnostics in human text output. |
| `--allow-empty` | Explicitly allow a no-rule or no-eligible-file result. |
| `--stats` | Include source/query/cache/batch/parser counts and timing/memory statistics separately from stable semantic output. |

Paths passed explicitly still obey ignores unless `--include-ignored` is present. An ignored explicit path is reported as such, not silently scanned. A partial path selection is explicitly recorded in output.

`wt check` does not change source, rules, mode, configuration, or waivers. Its only permitted persistent writes are optional derived cache entries; cache failure falls back to uncached execution with a notice. `wt set-mode` persists its supplied reason as informational mode-change metadata, without claiming a verified human approval identity.

### 9.4 Exit codes

| Code | Meaning |
|---|---|
| `0` | Completed the requested checks without blocking diagnostics; advisory diagnostics may exist. |
| `1` | Completed with blocking diagnostics; for `wt test`, an otherwise successfully executed fixture expectation failed. |
| `2` | Invalid invocation/configuration/rule, analysis gap, runtime failure, stale update hash, missing capability, or incomplete evaluation. |
| `130` | User interruption, when a controlled exit can be produced. |

Incomplete evaluation takes precedence over known findings: preserve findings, but return `2`. Zero enabled rules or zero eligible files across the requested scan returns `2` unless `--allow-empty` is explicit. Individual non-applicable rules are not errors and are listed with their reasons.

Disabled rule manifests must still be parseable for discovery. Their detector code is not executed by ordinary checks; `wt validate` can inspect them explicitly.

### 9.5 Explicit changed-file checks

`wt check` remains a full current-root scan with correctness-preserving caches. `wt check --changed [--base REV]` requests narrower file-local feedback and always reports `scope.partial: true`; it is not a substitute for the full scan after creating or editing a rule.

Resolve the base to a local Git commit/tree without network access, filters, hooks, or application execution. Non-Git roots and unresolved bases are errors. Select existing tracked files changed relative to the base (including staged/unstaged work) and eligible untracked files. Using the union of relevant Git name-diff sets is allowed to conservatively include a file whose final bytes equal the base. Renames select the existing destination; deletions are listed in coverage but have no source body to check.

Git ignores, WT exclusions, and rule scopes still apply. `--changed` and explicit positional paths cannot be combined. `--base` without `--changed` is invalid. A supplied revision is resolved to its immutable ID and recorded in output.

File-local rules run only on this selected set. Explicit repository rules receive their full normal authorized input set, including unchanged dependencies, because a deletion or one changed file can affect a cross-file invariant. Report those additional dependency files. If the file-local changed set is empty and no repository rule needs work, return an explicit `no_work` result with exit `0`; zero selected enabled rules is still an error without `--allow-empty`.

Changes to `.wt/`, ignore configuration, or global rules are a reason to run the ordinary full check. The CLI must warn when it detects changed local policy in a changed-only invocation; it cannot assume Git reveals every global-policy edit. Partial coverage remains visible even if all requested invocations completed.

### 9.6 Plan output and debugging

`wt plan` uses the same root, selected rules, validated language, and optimizer settings as checking. It does not execute source rules or fixture inputs. Data-dependent guards/regions remain symbolic, and optional batching decisions may wait for actual file sizes. A plan is not a passing check.

JSON plan output uses the plan schema and includes rule bindings, static query nodes, input templates, canonical pattern aliases, consumers, guards, result shapes, and optimization decisions/reasons. Plan-local node IDs are inspection handles, not persistent identifiers rule authors must maintain. Invalid rules produce the normal structured command error envelope with exit `2`, not a partially valid plan.

For execution debugging, compare `wt check --optimizer auto --no-cache --stats` with `wt check --optimizer off --no-cache --stats`. Output must retain each rule's own findings, even when their source spans coincide.

## 10. Diagnostics and machine output

Human output begins with file, line, column, severity, and qualified rule ID, followed by message and suggested direction. Advisory findings are explicitly labeled non-blocking, or blocking via `--strict`, so an error-level advisory diagnostic does not misleadingly imply a failed command. Avoid printing whole files or unlimited source snippets.

```text
frontend/src/task-start-page.tsx:89:15 error local/template-number-decimal-step
  Template numeric parameters require decimal-compatible input stepping.
  Use step="any" or the documented conditional equivalent.

1 blocking diagnostic; 0 review diagnostics; 2 files checked; analysis complete.
```

The example location is illustrative, derived from the supplied bug report rather than a new repository scan.

JSON stdout is exactly one JSON document, even on ordinary structured failures. Logs and progress go to stderr. Do not mix ANSI escapes, banners, or log lines into JSON stdout. Inputs named `--format json` must also get JSON for argument/validation failures when the format option can be parsed.

The result schema covers check envelopes and a small `command_error` envelope for failures before a check root/result can be assembled. Successful mutation commands return the fields specified in section 9.2.

The following synthetic result illustrates the envelope using the bundled source fixtures. It is not output from an executed WT runtime. All-zero policy/package digests are placeholders; the file digests and coordinates correspond to the fixture text.

```json
{
  "schema_version": 2,
  "command": "check",
  "status": "findings",
  "exit_code": 1,
  "complete": true,
  "root": "/workspace/project",
  "scope": {
    "partial": false,
    "include_ignored": false,
    "host_ignores": false
  },
  "policy_digest": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
  "coordinate_encoding": "unicode-scalar-columns",
  "diagnostics": [
    {
      "rule_id": "local/template-number-decimal-step",
      "rule_digest": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
      "code": "decimal-step",
      "kind": "violation",
      "severity": "error",
      "mode": "enforced",
      "blocking": true,
      "path": "frontend/src/task-start-page.tsx",
      "file_digest": "sha256:5df86ef397325371c720324d7305bfba032677d9c56d0f4ae99a129ea983165c",
      "start_byte": 41,
      "end_byte": 116,
      "start_line": 3,
      "start_column": 5,
      "end_line": 5,
      "end_column": 7,
      "message": "Template numeric parameters require decimal-compatible input stepping.",
      "help": "Use step=\"any\" or the documented conditional equivalent."
    }
  ],
  "suppressed": [],
  "errors": [],
  "rules": [
    {
      "id": "local/template-number-decimal-step",
      "digest": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
      "mode": "enforced",
      "status": "completed",
      "checked_files": 2
    }
  ],
  "files": [
    {
      "path": "frontend/src/task-start-page.tsx",
      "digest": "sha256:5df86ef397325371c720324d7305bfba032677d9c56d0f4ae99a129ea983165c",
      "status": "checked"
    },
    {
      "path": "frontend/src/template-pages.tsx",
      "digest": "sha256:8396153b37c9015a8727f44ff2818a040c8f8637c023a32bf38c8d502fbc2569",
      "status": "checked"
    }
  ],
  "summary": {
    "checked_files": 2,
    "blocking_diagnostics": 1,
    "advisory_diagnostics": 0,
    "review_diagnostics": 0,
    "binary_skips": 0,
    "analysis_gaps": 0
  }
}
```

Each diagnostic must include qualified rule ID, rule digest, diagnostic code and kind, severity, effective mode, computed `blocking` boolean, root-relative path, file-content digest, original byte span, display coordinates, message, and help.

`status` is `pass`, `findings`, `incomplete`, or `no_work`. Advisory findings give `findings` with exit `0`, not an assertion that no issues exist. `no_work` is exit `0` only with `--allow-empty`.

A successful `check` result also reports the requested selection, dependency expansion (when present), and optionally a plan digest. Operational `stats` are present only when requested and do not redefine semantic equality.

Stable diagnostic ordering is path, start byte, end byte, qualified rule ID, diagnostic code, kind. Sort error and coverage arrays deterministically as well. Deduplicate only exactly identical diagnostics from the same qualified rule; keep findings from different rules distinct.

Never describe a complete scan as proof of application correctness. It means the selected rules finished on their declared eligible inputs.

## 11. Waivers and justified exceptions

Separate a legitimate exception to a rule's contract from an accepted current violation.

A legitimate exception belongs in the detector or its scoped exclusions, with an explanation and fixture. A temporary accepted violation belongs in `.wt/waivers.json`.

A waiver file uses `{ "schema_version": 2, "waivers": [...] }`. Each entry requires `id`, `rule_id` (qualified), `code`, `path` (exact root-relative), `matched_text_digest` (SHA-256), and `reason` (nonempty). Optional zero-based `occurrence` indexes matching diagnostics in stable span order to disambiguate identical matching text in one file. A waiver that ambiguously matches multiple diagnostics is an error.

Waivers are applied after detection; they never prevent the detector from running. JSON always lists suppressed diagnostics and matching waiver IDs. A changed match no longer satisfies the waiver. Expiration is not implicitly evaluated against the wall clock; adding time-based expiry would require an explicit clock input and schema/API revision.

An unmatched waiver is listed as stale. Under `--strict`, stale waivers make validation incomplete until reviewed. Ordinary checking reports a non-blocking maintenance notice.

There is no automatic blanket baseline, no silent “ignore all current findings,” and no magic source-code comment that disables checks in arbitrary languages. Suppression is explicit policy data.

## 12. Worked rule: decimal template inputs

### 12.1 Contract and scope

The supplied example describes a parser that accepts finite decimal numbers and an equivalent input that explicitly uses `step="any"`. The task-start input omits this setting.

HTML's default number-input step is `1`, and valid values depend on the step base; `step="any"` removes the step restriction. It does not remove unrelated constraints such as `min`, `max`, or `required`. [R8]

The retained lesson is **not** “all number inputs need `step=\"any\"`.” It is:

> In this repository's verified template-number rendering context, a control must not impose an unintended step restriction on parameters that allow decimals.

The reference rule scopes itself to the two known modules supplied in the example. Its detection generalizes across matching controls, whitespace, attribute ordering, and local identifier names within that scope. Extending it to every `.tsx` file requires validating that the same domain contract applies there. This is a documented scope boundary, not evidence that other files are safe.

### 12.2 Detection strategy

1. Parse actual JSX inputs using `jsx.v1`.
2. Recognize the documented conditional type expression using a regex over canonical expression text.
3. Accept literal `step="any"`, `step={"any"}`, or the conditional equivalent using the same parameter identifier.
4. Report missing `step` or an explicit step of `1` as a recognized violation of the rule's required source convention.
5. Report spreads, duplicate relevant attributes, and unrecognized step expressions as review diagnostics, not invented certainty.
6. Ignore unrelated literal integer inputs outside the recognized template-input pattern.

The detector does not resolve aliases, wrapper components, helper calls, shadowed identifiers, or business types. It cannot prove that every expression producing a numeric input has been found.

### 12.3 Proposed detector code

```wt
for input in jsx::inputs(file) {
    let type_attr = input.attr("type");
    if type_attr == () || type_attr.kind != "expression" {
        continue;
    }

    let matched = rx::capture_text(
        "template_number_type",
        type_attr.canonical
    );
    if matched == () {
        continue;
    }

    if input.has_spread || input.duplicate("type") || input.duplicate("step") {
        emit(input.span, "unverified-step");
        continue;
    }

    let parameter = matched.group_text("parameter");
    let step = input.attr("step");
    if step == () {
        emit(input.span, "decimal-step");
        continue;
    }

    if step.static_string == "any" {
        continue;
    }

    let expected = parameter
        + " . type === \"number\" ? \"any\" : undefined";
    if step.kind == "expression" && step.canonical == expected {
        continue;
    }

    if step.static_string == "1" || step.canonical == "1" {
        emit(input.span, "decimal-step");
    } else {
        emit(input.span, "unverified-step");
    }
}
```

The named pattern is:

```regex
^(?P<parameter>[A-Za-z_$][A-Za-z0-9_$]*) \. type === "number" \? "number" : "text"$
```

This is a WRL1 statement body targeting WT's proposed host API, with `file` supplied by the runtime. It is not standalone Rhai code and has not been executed against an implemented WT runtime. Its JSX selection and canonical-text regex calls are visible to the query planner.

The accompanying example package contains the complete manifest, source, self-contained submission, and fixture suite.

### 12.4 Required expected outcomes

| Input | Expected result |
|---|---|
| Supplied conditional input without `step` | `decimal-step` violation. |
| Supplied fixed conditional input | No diagnostic. |
| Same pattern with `step="any"` or `step={"any"}` | No diagnostic. |
| Same pattern with `step="1"` or `step={1}` | `decimal-step` violation. |
| Reversed conditional step | `unverified-step` review. |
| Helper-derived step | `unverified-step` review. |
| Relevant input with spread attributes | `unverified-step` review. |
| Same pattern with a renamed ASCII parameter identifier | Same classification. |
| Different quote style, formatting, or ordinary attribute order | Same classification. |
| `onChange={(event) => ...}` before the type attribute | Correctly parsed; `>` in the arrow does not end the input. |
| JSX-looking text in a string or comment | No diagnostic. |
| Unrelated literal retry-count numeric input | Not in this detector's recognized template pattern. |
| Invalid TSX that cannot be parsed | Analysis error, never a passing detector result. |

## 13. Shared execution planning, scheduling, and caching

### 13.1 Required execution model

**Rules are independent policy units; expensive read-only operations are shared execution units.** Changing, disabling, or failing a rule does not redefine another rule's detector. WT—not the rule author—owns file discovery, I/O, query scheduling, deduplication, and parallelism.

The default engine must not run one independent repository scan per rule:

```text
load and validate all selected rules
    -> verify retained fixtures
    -> lower to WT Rule IR and guarded query templates
    -> intern common queries; build a physical plan
    -> enumerate the selected repository ONCE
    -> determine applicability and path masks
    -> per eligible file, assigned to one worker:
           read/hash one immutable source snapshot
           check per-rule result caches
           satisfy demanded shared searches/parses
           run remaining rule-local predicates
           retain completed findings / errors
           release file-local derived values
    -> run explicit repository reductions over authorized snapshots
    -> apply current modes and waivers
    -> stable output and coverage accounting
```

“Read once” means one source snapshot read per file in a normal invocation, not one byte access for all future operations. Hashing, parsing, and different regexes may still traverse those bytes. Git metadata reads, fixtures, and compiler inputs are separate. Working-tree race detection can require metadata checks/retries, which must be counted. Warm runs still verify content rather than trusting only timestamps.

File-local rules must not force all repository contents into memory. File-level parallelism is primary; do not assign every rule for the same file to a separate worker and lose locality. Scope/path masks should eliminate irrelevant rule/file pairs before source matching.

### 13.2 Logical plan versus physical plan

The **logical plan** records what each rule requests: source inputs, scoped query templates, branch guards, dependent spans or temporary text, local predicates, and diagnostic sinks. A query DAG may include parameterized nodes—for example a regex over every input attribute's canonical text. It need not statically enumerate every runtime substring.

The **physical plan** chooses memoization, optional multi-pattern batches, scheduling, and storage. It cannot alter the logical meaning. Rhai does not perform this optimization automatically: WT's compiler extracts the operations and its Rust query service executes/reuses them.

Mandatory optimized-engine behavior:

- One file discovery pass and one snapshot source read per selected file in a normal run.
- File-centric execution with fresh logical state per rule.
- Intern identical resolved pattern definitions across rules, including global/local scopes.
- Memoize identical demanded native queries within a file-local transaction, including successful empty results.
- Share demanded syntax parses/indexes for the same snapshot, helper version, and mode.
- Preserve matching behavior, coordinates, control flow, diagnostics, and failure visibility.

Cost-based optimizations, whose use is not mandatory for every workload:

- Batch same-haystack literal-presence tests.
- Use multi-regex presence scans to reject queries that cannot match.
- Reuse exactly identical predicate results on the same input region.
- Derive required-literal rejection from parsed regex semantics.

The engine must support a conservative unfused path. An optimization that cannot establish its correctness or exceeds its compilation budget falls back; an invalid original pattern/rule is still an error. No whole-program theorem prover or globally optimal plan is required.

### 13.3 The shared-pattern example

Rule A and rule B each declare the expression `request\([^\r\n]*\)` under their own local pattern names:

```wt
// rule A: check.wt
for matched in rx::find_all(file, "request") {
    if matched.text.contains("foo") {
        emit(matched.span, "contains-foo");
    }
}
```

```wt
// rule B: check.wt
for matched in rx::find_all(file, "call_region") {
    if matched.text.contains("foo123") {
        emit(matched.span, "contains-foo123");
    }
}
```

For a shared eligible file, the logical relation is:

```text
snapshot
    -> one canonical regex-find-all query
         -> immutable match sequence
              -> A: contains("foo")    -> A's finding
              -> B: contains("foo123") -> B's finding
```

In the ordinary optimized file-local transaction, the common regex enumeration occurs at most once when demanded. The two `for` statements may still have separate lightweight control-flow evaluation; sharing the expensive native operation does not require a new vectorized scripting VM.

For input `request("foo123")`, **both rules report**. For `request("foo")`, only A reports. Their scopes, metadata, fixtures, and modes remain independent. A narrower predicate does not replace or suppress a broader rule. An implementation may exploit literal implication only for precisely equal haystacks and compatible exact-string semantics; such inference is optional.

### 13.4 Query identity and memoization

Resolve pattern aliases before query interning. Use a canonical tuple, then hash it for efficient lookup; retain enough information to verify equality rather than trusting an unvalidated external digest.

```text
PatternKey = (
    regex expression bytes,
    fully expanded flags,
    regex / Unicode semantic version
)

QueryKey = (
    operation and result shape,
    PatternKey or helper identity,
    input snapshot identity or derived-text identity,
    source region and boundary mode,
    capture / normalization options,
    applicable query-wide limits
)
```

Snapshot identity includes actual content digest and source identity. Pure text results may internally share across identical bytes, but path-sensitive behavior and reportable span handles must always be rebound to the correct authorized file. Cross-file sharing is not required. A whole-file search, an independent-slice search, a Boolean existence test, and all-match enumeration are distinct operations.

Derived text keys include exact bytes or a verified digest plus length and the transformation/helper version. Such nodes can be memoized when demanded; their runtime inputs do not make the entire rule unplannable. Pattern definitions themselves remain static.

Do not share solely because pattern names match. Do not attempt arbitrary regex equivalence. Different regex spellings may remain separate even when a human believes them equivalent. Inline flags, capture definitions, Unicode settings, empty-match behavior, and anchoring participate in semantics.

A cache entry is `complete(value)` or `failed(error)` with recorded provenance, never an ambiguous partial vector. Consumers have independent cursors. Consume/skip/`break` in one rule cannot drain or mutate another rule's results. A complete result may satisfy a weaker operation only with a proven projection; an existence result cannot stand in for missing offsets or captures.

Keep demanded shared results until the last consumer in that file-local transaction finishes. Under memory pressure, reduce parallelism, use a bounded immutable representation, or fail explicitly; do not silently evict and recompute the same full-file query per subsequent rule. Cross-phase repository reductions may recompute evicted derived data from retained snapshots, but this is counted and never triggers another repository traversal or a read of newer file bytes.

### 13.5 Demand, absence, and failure semantics

Query extraction is not permission to eagerly execute every potential query. Preserve `if`, short-circuit, early-return, scope, and applicability guards. Optional prefilters may run speculatively only when their work is bounded and any speculative failure is discarded/falls back until the original query is actually demanded; they must not create an error that the logical rule would never encounter.

A rejected query returns its proper logical result—false or an empty sequence—not a universal instruction to skip the entire rule. For example:

```wt
if !rx::is_match(file, "required_header") {
    emit(file.span, "missing-header");
}
```

A file without the required literal is exactly where this rule can report. Likewise, code following an empty match loop may still emit. Skip an entire rule only with a sound control-flow proof that its remaining computation has no findings or relevant failure; ordinary lazy evaluation is always a valid fallback.

Do not lift a predicate on one match region to a file-wide result. `contains("foo")` elsewhere in the file does not establish that a particular matched call contains it. Do not move potentially failing calls out of branches or interpret syntax parse failure as “no candidate elements.”

Shared failures identify the canonical query and all actually affected rule/file invocations. A rule-local type error, rejected budget, or attempted unauthorized emission remains that rule's failure. All failure classes preserve nonzero/incomplete status; successful independent findings remain useful but do not make the check complete.

### 13.6 Multi-pattern execution

**Literal presence.** When enough compatible literal checks operate on the same file or region, a native multi-literal engine such as Aho-Corasick may return a Boolean per literal in one shared traversal. It must retain substring/overlap hits: `foo123` satisfies both `foo123` and `foo`. Ordinary leftmost non-overlapping matching can lose one of those identities; use all-pattern presence semantics, such as Standard overlapping search, or an equivalent proven algorithm. [R15]

The empty literal has presence `true` without a scan. Keep case/Unicode transformations explicit. WT's baseline `contains` is case-sensitive exact string presence. Grouping is by the same input region and matching semantics, not merely by strings occurring somewhere in a rule. Small regions may be cheaper with separate native substring tests; use measured thresholds rather than mandatory batching everywhere.

**Regex presence.** `RegexSet` can identify which expressions match, but does not supply each rule's full match offsets or captures. Use it as an optional presence query/prefilter, followed by each demanded canonical exact query when needed. [R14]

A negative exact presence result can synthesize an empty `find_all` result within the same input/flag semantics. Positive presence is not a finding and cannot supply captures. Group patterns with compatible flags; preserve original per-pattern semantics and membership mappings. Never replace independent regex iteration with one alternation whose priority discards overlapping rule matches.

**Required literals.** Derive a rejection condition from the parsed regex representation only where absence proves no match. For alternation `foo|bar`, absence of `foo` alone is insufficient. Handle optional/empty matches, case-insensitive patterns, Unicode, and anchors correctly, or decline the optimization. User-provided metadata is not proof of a necessary literal.

**Cost control.** Combined automatons can be expensive to build and retain, and many positive regexes may require a second traversal for exact matches. Cap group sizes, compile memory, and batch complexity; fall back to individual matching without reducing correctness. Do not claim that all rules or all arbitrary regexes execute in one pass or constant time.

### 13.7 Explicit repository rules

Retain `execution: "repository"` for deliberate cross-file checks, but keep it outside the normal file-local fast path. The body receives only `repo`; its `repo.files()` sequence is a view over the coordinator's selected, authorized snapshot set.

The coordinator discovers files once, computes each rule's applicable set, and retains relevant snapshots within repository budgets while the file-local stage runs. Repository reductions execute after those inputs are finalized. They can request the same native query service and reuse retained derived results. They cannot perform an independent directory walk, read files by a constructed path, or observe newer disk contents than the file-local stage.

A repository rule may use bounded local counters/strings and nested iteration over finite sequences. It has no mutable global state or results from another rule. Its result cache depends on its **complete input manifest**, including normalized paths, content digests, applicability facts, and additions/deletions. Results from one file cannot establish a cross-file invariant on their own.

Explicit path selection narrows repository-rule inputs too, and is reported as partial. In `--changed` mode, repository rules instead receive their full normal input scope, so cross-file checks are not silently evaluated only on changed files (section 9.5). Any required snapshot gap makes that repository invocation incomplete.

### 13.8 Persistent caches and invalidation

Keep separate layers:

| Layer | Key / invalidation principle |
|---|---|
| Validated package and fixture result | Package/test bytes, WRL/backend/helper semantics, resource profile. |
| Compiled matcher / residual artifact | Canonical pattern/program definitions and binary/backend build identity; never trust serialized backend objects from untrusted storage. |
| In-run shared query results | Exact query identity for the current authorized snapshot and input region. |
| Persistent raw rule/file findings | Rule execution identity, normalized file path, exact content digest, effective applicability/scope facts, semantic versions, and logical limit profile. |
| Repository-rule findings | Rule identity plus full authorized input manifest, including missing/present applicability inputs. |
| Final rendering/enforcement | Recompute with current modes, diagnostic presentation, waivers, selected roots, and strictness. |

**Do not key every file result solely by the entire ruleset hash.** Adding rule B should invalidate/rebuild the combined plan and evaluate B, while unchanged rule A's still-valid raw results remain reusable. Editing one rule must not force unrelated rules to re-execute on unchanged inputs. A global helper/runtime semantic change can legitimately invalidate many entries.

The complete package hash is an acceptable conservative per-rule identity; narrower semantic digests may improve reuse if all observable inputs are accounted for. Pattern-sharing keys exclude aliases, author prose, and rule IDs, but rule findings remain attributed to their own identities. Scope and file path matter even when content is equal.

Mode/waiver changes must take effect without stale enforcement: store raw diagnostic codes/spans and apply current policy afterward, or include every relevant policy input in final-result keys. Every enabled supplied test suite must still have a valid passing entry before using source-result caches.

Read/hash selected file contents to verify warm hits. Size/mtime alone is insufficient. Warm unchanged runs can avoid regex/helper/control-flow work, not necessarily all filesystem traversal or hashing. `--changed` is an explicit narrower selection, not an implicit cache heuristic.

Never promote failed, truncated, interrupted, unstable, or budget-exhausted results into complete cached results. Corrupt entries are discarded. Cache write failure falls back to uncached checking with a notice. Caches contain no original source or full snippets by default; store coordinates and identifiers. Hashes are not authentication: use `--no-cache` or a trusted cache in protected acceptance runs.

All public digests use SHA-256 (`sha256:` plus 64 lowercase hex characters). File digests cover original bytes. Package digest framing remains `WT-PACKAGE-1`: sorted normalized referenced paths with length-prefixed path/content bytes; the manifest's schema/language fields participate. Derived execution/query/policy identities use separate versioned domain separators and golden serialization tests. A `plan_digest` describes the combined plan, not the sole key for individual-rule results. Hash framing uses UTF-8 paths, unsigned 64-bit big-endian length prefixes, sorted unique referenced paths, and a length-prefixed domain separator. Semantic-key tuples have explicit type tags and fixed field order; implementations must publish golden vectors rather than rely on platform-native serialization.

### 13.9 Inspection and reference execution

`wt plan [--rule ID] --format json` validates/compiles selected rules and reports their static guarded query templates, scopes, canonical pattern aliases, consumers, sharing candidates, and chosen optimization strategy. It does not execute fixture cases or read application source contents; data-dependent regions and final batching thresholds remain explicitly unresolved. It reports no invented timing, file counts, or completed analysis.

`wt check --stats` reports actual work separately from semantic findings: snapshot reads/bytes hashed, relevant rule/file pairs, residual invocations, result-cache hits, logical query requests, physical query evaluations, shared-result hits, parser evaluations, regex/literal batch work, peak memory, and timing. If showing “scans avoided,” label whether it is a model-derived estimate rather than a measured counter. Do not count metadata reads as source reads or hide worker-local compilation costs.

`wt check --optimizer off --no-cache` is the correctness reference path: keep the same snapshots, language, limits, scope, and output semantics, but disable cross-rule query sharing and multi-pattern/prefilter optimizations. It is a diagnostic/conformance option, not the default architecture. It may still compile each pattern once per worker and keep file-centric I/O.

Compare optimized/reference, cold/warm, serial/parallel, and permuted-rule-order runs. Their stable diagnostics, selected coverage, and logical classifications must agree on workloads that complete within physical safety limits. Hardware timings, cache statistics, and plan descriptions may differ. An exhausted optimized run cannot claim success merely because the reference might finish, or vice versa; either is explicitly incomplete.

### 13.10 Performance guarantees and boundaries

The design eliminates redundant work; it does not make accumulating arbitrary rules free. Runtime can still grow with the number of **distinct** expensive queries, rule-local predicates, total matching output, compiler work, and repository reductions. Many findings impose unavoidable output cost.

Required optimization priority is source locality and sharing of whole-file searches/parses. Cheap predicates over short match regions do not require sophisticated fusion. Preserve a simple native substring path when it costs less than building/querying a batch automaton.

The performance acceptance matrix in section 16 tests both highly shared and completely distinct rule sets. Do not advertise constant-time scaling, guaranteed convergence, or Ruff-equivalent speed without implementation evidence.

## 14. Agent integration and trusted enforcement

A normal agent workflow is:

```bash
wt schema submission
wt new --stdin --format json < proposed-rule.json
wt test local/template-number-decimal-step --format json
wt plan --rule local/template-number-decimal-step --format json
wt check --rule local/template-number-decimal-step --format json
wt show local/template-number-decimal-step --format json
# Refine code and examples; inspect all changed classifications.
wt set-mode local/template-number-decimal-step enforced \
  --reason 'Reviewed the contract and retained positive and negative cases.'
wt check --format json
```

Agent guidance should require a rationale, honest detection scope, preserved examples, investigation of other matches, and explicit treatment of uncertainty. WT should ship a short CLI usage document suitable for an agent instruction file, not integrate a specific model provider.

For reproducible repository checks, a CI invocation can use:

```bash
wt check --no-global --no-host-ignores --format json
```

This intentionally uses repository-local policy only. It is not tamper-proof when the agent can modify that policy or the CI command.

### 14.1 Necessary external protection

To enforce rules against a coding agent, protect the checker binary/version, final invocation, rule definitions, scope/exclusion configuration, and waivers through a trusted review/CI boundary. Changes to `.wt/`, relevant ignore settings, and CI configuration must be separately reviewable. Use `--no-cache` for the protected acceptance run when the agent can write the ordinary developer cache. Restricting only `check.wt` is insufficient because the agent could otherwise change applicability, disable the rule, or suppress its finding.

A mode flag, package hash, or lock file stored alongside code that the same agent can edit is not authorization. Digests provide identity and cache integrity, not proof that a trusted person approved a rule.

WT does not promise to stop an actor who controls the repository, checker executable, and final acceptance process. Its guarantee is narrower: the declared approved policy is executed faithfully under a trusted invocation, and weakening it is a visible policy change rather than an invisible side effect of checking.

No hosted approval system is required by this specification. Existing version-control review and CI controls can provide the boundary.

## 15. Rust implementation structure

Use a single Cargo workspace and a small number of coherent internal crates or modules. Separate public contracts from implementation details; avoid one crate per trivial subsystem.

```text
Cargo.toml
crates/
  wt-cli/
    src/main.rs
    src/args.rs
    src/commands/
    src/output/
  wt-core/
    src/config.rs
    src/discovery.rs
    src/rule.rs
    src/schema.rs
    src/selection.rs
    src/diagnostic.rs
    src/fixtures.rs
    src/waivers.rs
    src/cache.rs
    src/digest.rs
    src/runner.rs
  wt-runtime/
    src/language/grammar.rs
    src/language/validate.rs
    src/language/rhai_adapter.rs
    src/language/ir.rs
    src/language/source_map.rs
    src/planner/query_key.rs
    src/planner/intern.rs
    src/planner/guards.rs
    src/planner/batches.rs
    src/planner/physical_plan.rs
    src/query_service.rs
    src/reference_executor.rs
    src/text_api.rs
    src/regex_api.rs
    src/span.rs
    src/budget.rs
    src/worker.rs
    src/helpers/jsx.rs
schemas/
docs/
examples/
tests/
  cli/
  selection/
  runtime/
  language_conformance/
  optimizer_equivalence/
  cache_invalidation/
  performance_work_counts/
  fixtures/
  output/
  adversarial/
```

Use maintained Rust libraries where they fit: CLI parsing, strict JSON/Schema generation, `regex` for regex execution, Rhai for the validated syntax/residual backend, optional Aho-Corasick/RegexSet for cost-effective batching, `ignore` and Git metadata support for file selection, and an existing parser for the explicit TSX helper. `gix` exposes repository discovery, index and excludes functionality, and configurable capabilities; select only the read-only features required by WT. Do not enable credential helpers, filters, hooks, or network operations just to inspect a repository. [R9]

Keep logical Rule IR and query interfaces free of Rhai-specific node types outside the frontend adapter. Put per-file snapshot ownership and query lifetime in the runner, not rule scripts. The reference executor and optimized executor must share the public language/API semantics, while using independent physical plans.

Dependency versions must be pinned through the release lockfile and exposed in diagnostic build information. Changing matching or helper behavior requires an appropriate WT API/capability compatibility decision, not an unnoticed upgrade.

## 16. Performance and portability requirements

The product should make repeated checking inexpensive enough for coding-agent feedback loops. This specification sets architectural and testable work-count requirements, not unmeasured wall-clock claims.

### 16.1 Required workload matrix

Use a reproducible corpus of at least 10,000 UTF-8 files totaling 100 MiB, plus focused large-file and pathological-match cases. Run 25, 100, and 1,000 scoped file-local rules under these separate scenarios:

| Scenario | Required observation |
|---|---|
| Identical searches, different predicates | Physical exact enumerations scale with distinct demanded queries, not duplicate rule count, within each file transaction. |
| Identical patterns under different local names | Same sharing as identical names. |
| Same local name, different regex definitions/options | No incorrect sharing. |
| Mostly distinct patterns | No assumed constant-time scaling; measure batching benefit and compiler/memory cost. |
| Many literals on equal regions | Correct all-literal presence, including prefix/overlap pairs. |
| Syntax-assisted rules | One demanded parse per file/helper version within a file-local transaction. |
| Sparse applicability | Irrelevant extensions/paths do not run expensive operations. |
| Match-heavy / zero-width patterns | Correct spans and complete-or-error limits; no silent truncation. |
| Repository reductions | Bounded snapshot memory and complete input-set invalidation. |

For each, measure cold start (including compilation), warm unchanged, one-file-changed, one-rule-added, one-rule-edited, one-file-deleted, and helper-version-changed runs. Compare optimizer `auto` and `off`, serial and parallel execution, and valid warm caches. Separately report strict full scans and explicit changed-file scans.

Publish hardware, operating system, tool/backend versions, source corpus, rule definitions, cache conditions, wall/CPU time, peak memory, snapshot reads/bytes, compile cost, logical requests, physical queries, result-cache hits, parse counts, diagnostics, and errors. A warm full scan may still spend time hashing bytes. Do not hide that work.

### 16.2 Deterministic work-count acceptance tests

On a cold, one-file, two-rule sharing fixture with at least one region matching the common regex:

- Normal optimized execution reads one source snapshot and performs one canonical exact enumeration when both consumers demand it.
- Both logical consumers retain their own findings and mode behavior.
- The reference path performs separate exact enumerations and produces the same findings.
- Adding more rules with the same resolved query does not multiply that query's physical evaluations within the transaction, although predicates and output can increase.

When rule B is added to a valid warm ruleset, A's unchanged rule/file raw result remains reusable. When only B is edited, A is not invalidated merely because the combined plan hash changes. File deletion invalidates applicable repository-reduction manifests.

Budgets and worker caches must be instrumented. No test should “pass faster” because files, matches, parse errors, or failed rules were silently skipped.

### 16.3 Portability

Support Linux, macOS, and Windows without Python or Node for ordinary WT execution. The package validation script supplied alongside this specification is a document/example QA tool, not a runtime dependency of WT.

Cover spaces, non-ASCII filenames, Unicode columns, CRLF, BOM, Windows separators, Git worktrees, and invocation from repository subdirectories. Keep parser/compiler dependencies behind WT interfaces and pin them through the release lockfile.

**Naming constraint:** Windows Terminal already uses `wt.exe` and the `wt` alias. Retain `wt` as the working interface, but ship an unambiguous `watchtower` executable as well; do not replace an existing Windows alias. Final public package naming remains a release decision. [R10]

## 17. Acceptance criteria

The implementation is accepted only when the following behaviors are demonstrated by automated tests and documented examples.

### 17.1 Configuration and CLI

- Global-only, local-only, and combined rules work.
- Identical IDs across scopes both run; ambiguous short IDs fail clearly.
- Missing configuration is distinguished from malformed/unreadable configuration.
- `wt new` supports positional JSON, stdin, and file input without prompts.
- Raw manifest/`check.wt` edits are picked up; no opaque rebuild step is required.
- `code.language: "wt-rule-1"` rejects arbitrary Rhai constructs and legacy wrappers with actionable diagnostics.
- Plan inspection reports real static relationships without claiming a source scan occurred.
- Atomic creation/update failure does not leave a partly active rule.
- Concurrent updates fail with a stale digest instead of silently losing changes.
- JSON output is parseable on success, findings, validation errors, and runtime failures.

### 17.2 Selection

- Gitignore nesting, negation, tracked files, untracked files, host excludes, and `--include-ignored` follow the specified contract.
- Hidden source files are not accidentally omitted.
- Dependency directories are skipped because of actual ignore/exclusion decisions, not undocumented names.
- Explicit paths, subdirectories, worktrees, non-Git directories, nested repositories, and symlink boundaries behave as specified.
- Every skip and analysis gap can be explained.

### 17.3 Rule correctness and execution

- The supplied decimal examples and their counterexamples have the expected outcomes.
- Missing step, wrong step, reversed conditional, arrow callbacks, spreads, comments, and renamed identifiers are distinct fixture cases.
- Enforced rules require passing positive and negative examples.
- Retained tests catch a refinement that accidentally suppresses the original defect.
- All APIs preserve original source spans and reject forged/out-of-root findings.
- Rules cannot perform filesystem writes, shell execution, network calls, dynamic imports, or environment reads.
- Infinite/expensive work, excessive allocations, invalid regexes, and parser failures cannot produce a clean result.

### 17.4 Determinism and enforcement

- Warm/cold cache results, optimizer auto/off, serial/parallel, and permuted-rule-order results agree semantically for completed workloads.
- Shared predicates retain independent findings: `foo123` can satisfy both `foo` and `foo123`.
- Shared queries use resolved definitions/flags/input regions; aliases and different names do not change matching.
- An absence rule still runs when a positive prefilter returns false.
- Scope/branch guards prevent speculative parser errors from becoming observed failures.
- A shared-query error propagates to demanded consumers without fabricating empty results.
- Cold shared-pattern work-count tests prove one physical enumeration per canonical demanded file query.
- Adding/editing a rule does not invalidate unrelated per-rule raw results merely by changing the plan hash.
- File changes, paths/renames, fixture changes, rule changes, flags, region boundary modes, helper versions, resource profiles, and repository input-set additions/deletions invalidate relevant caches.
- Changed-file checks disclose partial coverage and retain complete dependency input sets for repository reductions.
- Candidate/advisory findings are visible without silently becoming enforced policy.
- Enforced findings and review diagnostics block as specified.
- Disabled rules, waivers, coverage reductions, and configuration origins are visible.
- Incomplete results return `2` even when some findings are available.
- CI documentation explicitly describes the trusted boundary and does not imply self-protection by editable local files.

## 18. Definition of product success

Do not optimize for the number of rules or an empty findings list alone.

Success means that, during ordinary development, validated mistake classes recur less often, new instances are found cheaply, legitimate code changes are not routinely obstructed, and maintaining the rules costs less effort than repeatedly rediscovering the defects.

The product is complete when the CLI can create, explain, test, run, enforce, update, and retire these executable lessons using the readable on-disk format and the machine-facing contracts above. It does not need scientific novelty or a hosted platform to fulfill that purpose.

## References and evidence boundary

The numbered requirements above are proposed WT design decisions. References support external technical facts and implementation options, not a claim that WT has already been built.

The decimal defect and module locations were supplied by the user. No repository was inspected or modified to create this specification. The example fixtures are designed expectations; they have not been run through a WT implementation.

- **[R1] Rhai embedded language:** https://docs.rs/rhai/latest/rhai/ and https://rhai.rs/book/safety/sandbox.html
- **[R2] Rhai resource limits and dynamic evaluation:** https://rhai.rs/book/safety/max-operations.html and https://rhai.rs/book/language/eval.html
- **[R3] Rust regex syntax, limits, and iteration complexity:** https://docs.rs/regex/latest/regex/ and https://docs.rs/regex/latest/regex/struct.Regex.html
- **[R4] XDG Base Directory Specification:** https://specifications.freedesktop.org/basedir/latest/
- **[R5] Git ignore semantics:** https://git-scm.com/docs/gitignore
- **[R6] `ignore::WalkBuilder` options:** https://docs.rs/ignore/latest/ignore/struct.WalkBuilder.html
- **[R7] Structural pattern matching background:** https://ast-grep.github.io/guide/pattern-syntax.html
- **[R8] HTML number inputs and step behavior:** https://html.spec.whatwg.org/multipage/input.html#number-state-(type=number)
- **[R9] Read-only Git integration options through `gix`:** https://docs.rs/gix/latest/gix/
- **[R10] Windows Terminal's existing `wt` command:** https://learn.microsoft.com/en-us/windows/terminal/command-line-arguments

- **[R11] Rhai AST inspection, `internals`, and merge semantics:** https://rhai.rs/book/engine/ast.html and https://docs.rs/rhai/latest/rhai/struct.AST.html
- **[R12] Rhai symbol disabling:** https://rhai.rs/book/engine/disable-keywords.html
- **[R13] Rhai optimization levels and function evaluation:** https://docs.rs/rhai/latest/rhai/enum.OptimizationLevel.html
- **[R14] RegexSet presence versus match coordinates:** https://docs.rs/regex/latest/regex/ and https://docs.rs/regex/latest/regex/struct.RegexSet.html
- **[R15] Aho-Corasick overlap and match-kind semantics:** https://docs.rs/aho-corasick/latest/aho_corasick/enum.MatchKind.html
