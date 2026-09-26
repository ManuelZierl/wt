# Watchtower — Product and technical specification

Working command: `wt`; alternate executable: `watchtower`.

> **Preserve situations worth checking, the reasoning already applied to them, and the conditions under which that reasoning needs to be revisited.**

Watchtower is a local-first, agent-neutral CLI for executable repository knowledge and retained review decisions. It keeps a text-first runtime, readable local/global rule packages, strict JSON authoring, the bounded WRL1 language (with optional structural `ast.v1` matching), Git-aware file selection, file-centric query sharing, a reference execution path, and content-verified caches. Occurrence-level review decisions are a central capability: a rule can flag a situation worth judgment (`review`) as distinct from a prohibited source form (`violation`), and a human or agent records one evidence-bound decision per occurrence that reopens when the evidence changes.

**MUST**, **MUST NOT**, and **SHOULD** below express this specification's requirements. SHOULD permits an explicitly documented, tested alternative with the same outcome. Explanations and implementation notes are not additional runtime capabilities. Every contract wt accepts — rule/submission, detector fixtures, review records, configuration, and the result/command protocol — is version **1**; wt accepts exactly that version and rejects anything else with a clear error, since there is no prior installed release to stay compatible with.

The accompanying JSON Schemas in `schemas/` validate representation. This prose defines cross-field semantics, safe paths, authorization boundaries, evidence validity, and execution behavior that schema alone cannot establish. `docs/cli.md`, `docs/readable-rules-and-reviews.md`, and `docs/runtime-configuration.md` are shorter task-oriented guides to the same contract; this document is the normative reference they link back to.

### Contents

- [1. Product definition and success condition](#1-product-definition-and-success-condition)
- [2. Configuration directories and repository discovery](#2-configuration-directories-and-repository-discovery)
- [3. Configuration composition and identities](#3-configuration-composition-and-identities)
- [4. File selection and ignore behavior](#4-file-selection-and-ignore-behavior)
- [5. Readable rule packages and JSON interchange](#5-readable-rule-packages-and-json-interchange)
- [6. Watchtower Rule Language and host API](#6-watchtower-rule-language-and-host-api)
- [7. Validation, detector fixtures, and evidence quality](#7-validation-detector-fixtures-and-evidence-quality)
- [8. CLI contract and normal workflows](#8-cli-contract-and-normal-workflows)
- [9. Actionable diagnostics, compact output, and inspection](#9-actionable-diagnostics-compact-output-and-inspection)
- [10. Occurrence-level review decisions](#10-occurrence-level-review-decisions)
- [11. Shared execution planning, scheduling, and caching](#11-shared-execution-planning-scheduling-and-caching)
- [12. Agent authoring, review, and trusted enforcement](#12-agent-authoring-review-and-trusted-enforcement)
- [13. Implementation boundaries](#13-implementation-boundaries)
- [14. Performance and portability requirements](#14-performance-and-portability-requirements)
- [15. Worked examples](#15-worked-examples)

## 1. Product definition and success condition

Watchtower is a local-first, agent-neutral CLI for executable repository knowledge and retained review decisions. Humans or external coding agents understand a situation, choose a suitable protection, author a small detector where useful, review its occurrences, and retain the result in version-controlled files.

WT detects declared source patterns. It does not independently discover the originating bug, decide whether application behavior is correct, or turn an author's explanation into proof. A pattern may establish a prohibited source form, or merely identify a situation worth examining.

### 1.1 The supported learning loop

```text
Understand a concern and identify the relevant contract
    -> choose the cheapest reliable protection
    -> when WT is appropriate, write a detector, explanation, and fixtures
    -> validate recognition and challenge it with counterexamples
    -> scan the intended source scope
    -> classify each occurrence using actual contextual evidence
         recognition mistake -> improve detector and preserve the example
         acceptable review signal -> retain an occurrence decision
         confirmed concern -> fix, or keep actionable
         deliberately accepted concern -> explicit accepted-risk decision
    -> on later changes, run the same detector
    -> surface unreviewed occurrences and stale decisions
    -> correct, supersede, or retire knowledge explicitly
```

A successful later check should avoid repeating a still-valid review while surfacing new occurrences and evidence changes. Silence is not the goal by itself. Rule count and a green command are not measures of accumulated correctness.

### 1.2 Three legitimate rule intentions

| Intention | Promise | Appropriate diagnostic |
|---|---|---|
| Exact regression guard | Recognize specified source forms associated with a retained defect. Coverage outside those forms is not implied. | `review`, unless the matched form itself establishes a prohibited condition in the documented scope. |
| Review trigger | Recognize a situation for which context matters. An acceptable use can be a correct raw match. | `review`. |
| Explicit policy rule | Recognize a form prohibited by an explicitly adopted repository contract, whether or not every occurrence causes a runtime failure. | `violation`. |

The author SHOULD state the intention in `rule.md` and optional informational metadata. WT MUST NOT infer diagnostic kind from severity, security-related wording, or the word “bug.” No new executable semantic branch is attached to this taxonomy.

### 1.3 Choose protection rather than manufacture rules

An existing linter, a behavioral regression test, a type restriction, or an API redesign may preserve a lesson more reliably than a custom detector. The authoring workflow MUST permit the conclusion “no WT rule is justified.” A behavioral test and a WT review trigger may also be complementary.

WT does not execute application tests or arbitrary linter commands from rule programs. External verification can be referenced as evidence with its actual status. Creating a rule does not authorize a production experiment, package installation, network request, or application modification.

### 1.4 Product boundaries

Ordinary operation requires no account, network, database server, daemon, editor plugin, or model subscription. Rules and review records remain inspectable without the WT binary. Caches are disposable. Review decisions are not caches.

The core works with files, paths, text, patterns, opaque spans, conditions, and finite sequences. Optional trusted syntax helpers have explicit versioned contracts. A language-neutral runner does not mean that every detector is independent of source-language syntax.

The product is useful when avoided repeated investigation and prevented mistakes outweigh authoring, review, maintenance, and execution costs. This outcome is evaluated through later development, not inferred from successful rule compilation.

## 2. Configuration directories and repository discovery

### 2.1 Global directory

Resolve the global directory in this order:

1. Explicit `--global-dir PATH`.
2. Absolute `WT_CONFIG_HOME` environment variable.
3. On Unix-like systems, absolute `$XDG_CONFIG_HOME/wt` when `XDG_CONFIG_HOME` is set; otherwise `$HOME/.config/wt`.
4. On Windows, `%USERPROFILE%\.config\wt`, unless explicitly overridden above.

A relative environment override is an error rather than a path relative to an accidental working directory. The Windows dot-directory is a deliberate WT product decision.

Do not create global directories merely because the user runs `wt --help`, `wt check`, or another read-only command. `wt init --global` and a successful global mutation create them as needed.

### 2.2 Local directory and scan root

The local configuration is `<root>/.wt/`.

Determine `root` as follows:

1. `--root PATH`, when provided.
2. Otherwise, the nearest enclosing Git worktree root, including worktrees whose `.git` entry is a file.
3. Outside Git, the nearest ancestor containing `.wt/`.
4. Otherwise, the current working directory.

An explicit root inside a Git worktree is permitted. Inherited Git ignore rules between the worktree root and the explicit root still apply, but local WT configuration is loaded only from the explicit root.

Running `wt check` from a subdirectory normally checks the repository, not just that subdirectory. Positional paths narrow the scan; they do not silently change the rule roots. Resolve positional paths relative to the invocation directory and require them to stay within the selected root.

Nested `.wt/` directories do not cascade. They are separate configurations only when explicitly selected as the root. An enclosing Git repository takes precedence over a nested `.wt/` during automatic discovery. This prevents surprising policy changes based on the caller's current directory.

### 2.3 Layout and durable versus derived state

```text
~/.config/wt/
  config.json
  rules/<id>/
    rule.json
    rule.md
    check.wt
    tests.json
    fixtures/

repository/.wt/
  config.json
  rules/<id>/
    rule.json
    rule.md
    check.wt
    tests.json
    fixtures/
  reviews/<finding-hash>/
    00000001/decision.json
    00000001/rationale.md
    00000002/decision.json
    00000002/rationale.md
```

Rule packages and occurrence decisions are durable repository knowledge. Review decisions are local even when their detector is global. Global rules MUST NOT distribute local occurrence acceptances into other repositories.

Commit local manifests, explanations, code, fixtures, configuration, and deliberately accepted decisions. Derived caches live outside authoritative rule storage and may be deleted without deleting knowledge. Do not store a whole application snapshot or confidential source in a rule package merely to attach provenance.

Runtime-only lock files must not appear as unexplained source changes. A conforming implementation uses a documented coordination location or supplies narrowly scoped ignore entries during explicit initialization. Lock lifecycle is owned by the locking implementation; documentation MUST NOT instruct agents to unlink synchronization files opportunistically. An empty lock file does not prove that no process holds the lock.

### 2.4 Missing directories and discovery errors

Missing global or local directories are normal. Existing unreadable or malformed configuration is an error. Never silently replace an unreadable configuration with defaults.

Rule directories are immediate children of `rules/` with a valid rule ID. Temporary dot-prefixed directories are ignored. An incomplete non-temporary rule directory is an error, not an invisible rule.

## 3. Configuration composition and identities

### 3.1 Qualified rule identities

A rule's `id` must match:

```regex
^[a-z][a-z0-9]*(?:[-.][a-z0-9]+)*$
```

IDs cannot contain slashes, traversal components, or arbitrary filenames. A directory name must equal its manifest's ID.

The effective identity is `global/<id>` or `local/<id>`. Both rules execute when the same unqualified ID exists in both scopes. There is no automatic local shadowing of a global detector.

Commands may accept an unqualified ID only if it resolves uniquely. Ambiguity is a structured error listing the candidates. Unknown IDs are errors.

### 3.2 Configuration merge rules

Precedence is built-in defaults, global configuration, local configuration, then explicit CLI options.

- Scalar settings use the last explicitly supplied value.
- Scan exclusion entries are additive, preserving origin and reason.
- Rules from both directories are combined, not replaced wholesale.
- Mode overrides are keyed by qualified rule ID and require a reason.
- An override of an unknown rule is an error, catching stale settings and typos. Overrides targeting a whole scope explicitly omitted by `--no-global` are instead reported as inactive; they must not make the documented local-only CI command fail.
- Unknown configuration fields and duplicate JSON keys are errors.
- Runtime limits cannot exceed the binary's documented absolute safety ceilings.

`wt config --format json` distinguishes configured values from invocation-effective values and reports the origin of every overridden value. A CLI override MUST be reflected in the effective value, not merely in a separate contradictory flag. `wt list` reports both the declared and effective mode of each rule.

Example local configuration:

```json
{
  "schema_version": 1,
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

### 3.3 Explicit coverage expectations

Configuration `coverage.expectations` entries are optional:

```json
{
  "schema_version": 1,
  "coverage": {
    "expectations": [
      {
        "rule_id": "local/review-legacy-endpoint",
        "minimum_files": 1,
        "reason": "This repository must retain an active check over its endpoint configuration."
      }
    ]
  }
}
```

Expectations express adopted coverage policy, not learned scan history. They are additive across loaded configuration scopes; all loaded expectations must hold. Each references a selected, enabled, applicable rule and a positive minimum number of eligible source files completed for that rule. Zero findings is allowed; minimum findings is not supported. A missing/disabled/non-applicable expected rule, unmet file count, or required source gap is an incomplete policy check (exit `2`), even when unrelated rules succeed. `--allow-empty` cannot override an explicit expectation.

In a full unfiltered invocation, evaluate all loaded expectations. A deliberately narrowed `--rule`, `--changed`, or path check marks affected expectations `not_evaluated_partial`; it cannot establish full policy satisfaction. A global scope omitted by `--no-global` does not erase a repository-local expectation for a global rule: that local policy now cannot be satisfied and must be corrected deliberately. Expectations inside omitted global configuration are not loaded. Report these cases separately from inactive mode overrides.

`scope.require_files` remains an applicability test, not an assertion that files must exist. Do not overload it. Authors should avoid coupling independent language rules to the existence of both implementations. A file move may alter coverage; the rule and its expectation should be reviewed, not silently satisfied by a narrower scan.

No ordinary `check` writes a last-success baseline to infer expectations. History-aware coverage comparisons, if later introduced, require an explicit input with separate semantics.

## 4. File selection and ignore behavior

### 4.1 Selection contract

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

### 4.2 Git ignores

By default:

- Honor applicable `.gitignore` files, including nested files and Git's negation/precedence rules.
- Honor the current repository's Git-local excludes and user excludes when `honor_git_local_excludes` is true.
- Continue to scan tracked files even if an ignore pattern matches their names. This is the retained Git-aware selection contract.
- Outside Git, evaluate `.gitignore` files within the scan root using the same pattern syntax, but do not consult a nonexistent index or parent rules outside that root.
- Do not implicitly honor unrelated `.ignore` or `.rgignore` files.

The implementation must not simply accept a walker's default settings. WT's declared hidden-file, tracked-file, and parent-ignore semantics take precedence over any walker's defaults.

`--no-host-ignores` disables only Git-local and user-global ignore sources. Repository `.gitignore` files remain active. This supports less machine-dependent CI scans.

### 4.3 `--include-ignored`

`wt check --include-ignored` bypasses Git ignore filtering for this invocation. It does not bypass root boundaries, WT exclusions, rule scope, mandatory metadata exclusions, encoding requirements, or resource limits.

It does not recursively enter arbitrary symlink targets or embedded repositories.

`.venv`, `node_modules`, `.npm`, and similar directories are not magical WT exclusions. They are skipped when the repository or user ignore configuration excludes them. A repository that has not ignored its dependency directories must explicitly exclude them or expect WT to scan them.

No silent fallback directory-name blacklist is permitted. This avoids hiding legitimate project files just because their names resemble common dependencies.

### 4.4 Other filesystem behavior

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

### 4.5 Coverage does not depend on warning count

A rule can complete with zero findings and still have a limited recognition domain. Report enabled/disabled, applicable/non-applicable, eligible/completed file counts, and coverage expectation results separately from match counts. A complete partial check is permitted as feedback, but MUST remain labeled partial.

Directory discovery should prune irrelevant and ignored trees early where doing so preserves tracked-file and negation semantics. Counters must distinguish visited directories, metadata operations, source reads, and completed rule/file invocations. Merely noticing a nested repository does not establish that its source was analyzed.

Review evidence can require explicit supporting files outside a detector's matching scope. These bounded reads are separately authorized by the review operation and counted as evidence reads, never represented as detector coverage. Excluded or unavailable dependencies cannot be silently treated as unchanged.

## 5. Readable rule packages and JSON interchange

### 5.1 One authoritative representation for each concern

A rule package consists of `rule.json` (machine configuration), `rule.md` (long-form contract and reasoning), `check.wt` (formatted detector), and, when supplied, `tests.json` plus fixture files. There is one authoritative code source and one authoritative prose document. No database registration, compilation artifact, or online document is required to edit a rule.

New and explicitly updated packages use rule schema **1**. The manifest references `documentation.file: "rule.md"` and `code.file: "check.wt"`. There are no separate `description`, `rationale`, or `limitations` fields; the Markdown document is the only authoritative prose. `title` and short diagnostic messages stay structured for useful listings and command output.

### 5.2 Manifest fields

| Field | Requirement |
|---|---|
| `schema_version` | `1`; wt accepts exactly this version and rejects anything else. |
| `id`, `title` | Stable within its local/global scope; concise title. |
| `mode` | Advisory by default; explicit enforced/disabled modes are retained. |
| `severity` | Error, warning, or info; independent of mode and diagnostic kind. |
| `execution` | File by default, or explicit repository execution. |
| `intent` | Optional; `detect` (default, omitted on disk) or `watch` — a `watch` rule exists to force re-review when specific code changes, so findings and repeated acceptances are its expected steady state, not noise (`wt stats` reports it as `watch`, never `noisy`). |
| `scope` | Nonempty includes; reasoned excludes; optional applicability paths. |
| `patterns` | Statically declared local aliases for regexes and expanded flags. |
| `diagnostics` | Nonempty map of codes to `{kind, message, help}`. |
| `documentation` | Exactly `file: "rule.md"` on disk, or `source` in interchange. |
| `code` | Explicit `language: "wt-rule-1"`, capabilities, and one source/reference. |
| `tests_file` / `tests` | Disk suite reference / self-contained exported suite. |
| `metadata` | Informational authoring provenance, intent, tags, and evidence references; not authorization or executable policy. |

Pattern names remain identifiers with underscores; rule IDs may contain the documented hyphens/dots. Validation errors must identify the JSON pointer, accepted syntax, and a suggested correction. Do not silently rename an ID while leaving source references unchanged.

### 5.3 Markdown contract

Recommended headings are **Intended constraint**, **What is detected**, **How to review**, **Known limitations**, **Evidence**, and **Change history**. The text should distinguish a broad motivation from the detector's actual source-level claim.

A review trigger must describe which contextual questions remain open. An explicit policy rule must identify the adopted policy rather than present a style preference as a proven runtime defect. Evidence may be a hypothesis, a complete static argument, a reproduced failure, an unexecuted regression test, or an executed check. These labels describe different evidence; WT does not certify their truth by parsing them.

Markdown headings have no hidden execution semantics. A sentence such as “recheck when the collection becomes removable” is rationale, not an implemented dependency watch. Use explicit evidence dependencies for machine revalidation.

### 5.4 Interchange and editing

`wt new` accepts strict UTF-8 JSON using exactly one of a positional argument, stdin, or `--file`. Interchange uses `documentation.source`, `code.source`, and inline fixture `content`; it never follows arbitrary file references embedded in submitted JSON. On disk, WT materializes the prose and formatted program. Source-like fixtures SHOULD be materialized as separate files with safe generated names; small inline cases remain supported. Export resolves those references back into one self-contained object.

`wt show ID --format json` returns `data.rule` and the complete package `data.digest`, from the same bounded read. Use that digest for an optimistic update; do not fetch it separately from a later listing. A detected concurrent edit is an error.

JSON rejects duplicate keys, unknown structural fields, trailing commas, or contradictory inline/file representations. Package references are root-relative, bounded, non-symlink paths confined to the package. Source/display paths and fixture paths must not be lossy-converted or conflated.

### 5.5 Hashes and documentation edits

The package digest includes the manifest and all referenced authoritative files, including `rule.md` and fixtures. A prose edit can change the contract and therefore conservatively invalidates occurrence acceptances. The raw detector execution key may exclude long-form prose when all executable inputs are unchanged; final messages and policy are applied from the current package.

Do not automatically declare a prose change harmless by asking an LLM or normalizing Markdown. Formatting-only detector changes may conservatively invalidate reviews too; new/update autoformatting makes such churn less frequent. Any more precise compatibility rule needs a tested explicit contract.

## 6. Watchtower Rule Language and host API

### 6.1 Public language contract: `wt-rule-1`

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

### 6.2 Supported language subset

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

### 6.3 Rhai as a replaceable backend

Rhai may supply parsing and residual control-flow execution, or WT may interpret the validated AST through a restricted adapter. Both require a WT-owned validation/query boundary:

```text
check.wt + manifest
    -> bounded Rhai parsing, with optimization disabled
    -> fail-closed WRL1 syntax / binding / capability validation
    -> WT-owned validated call contract, source map, guarded query templates
    -> cross-rule query interning and physical planning
    -> approved residual control flow + Rust query service
```

Rhai exposes AST walking and internal node access through its `internals` feature. This is a real implementation dependency: isolate the adapter, pin the dependency, test all accepted/rejected constructs, and do not assume every internal AST API is a stable public compiler framework.

Validate the **unoptimized** syntax tree and every source branch, including unreachable code. Otherwise dead-code elimination could hide forbidden syntax. Reject unknown AST node variants. Disable forbidden symbols in the parser where supported, but do not substitute a blacklist or source regex for the positive AST/call whitelist. Rhai provides symbol disabling, and its full optimization mode can evaluate functions; WT must not execute detector host functions while validating or extracting queries.

The adapter may use a separate compile-only engine to resolve syntax without binding real source data. Residual code can run through the validated Rhai AST with registered host calls routed to the query service; there is no requirement to invent a new VM. WT's public operations, guarded query descriptions, and logical cost accounting remain authoritative. A complete backend-independent IR for every residual statement is not a product prerequisite; do not claim that an inspection-only query IR is a complete optimizing compiler. Never concatenate or merge separate rule ASTs to obtain sharing: Rhai's AST merge appends statements and can replace same-named functions; it is not a cross-rule query optimizer.

Only explicitly registered native functions are available. `emit` is a rule-local output effect; matching and text/helper operations are read-only queries. Disable built-in I/O/debug output, module resolution, dynamic evaluation, and any ambient filesystem/network/process/clock/random access. Never enable Rhai's `unchecked` feature in an enforcement build. Configure limits; Rhai's operation limit is otherwise unlimited and does not by itself bound the cost of a native function.

WRL1 diagnostics identify the rule file, original source span, stable WT error code, unsupported construct, and allowed alternative. Example:

```text
WT102 check.wt:4:1: `while` is not part of wt-rule-1.
Use `for` over a finite WT sequence such as rx::find_all(...).
```

Required compiler error families include `WT100` syntax, `WT101` unsupported language version, `WT102` forbidden construct, `WT103` non-static identifier argument, `WT104` unregistered call/capability, `WT105` unsupported mutation/iteration, and `WT106` unknown pattern/diagnostic ID. Raw backend errors may be included as details, not used as the sole public contract.

### 6.4 Text and matching API

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

Use pinned Rust `regex` semantics: leftmost-first, non-overlapping iteration for each separate expression, named captures, and UTF-8 boundaries. Zero-width matches are permitted and must advance according to the engine's UTF-8-aware iteration semantics. No backreferences or look-around. Individual searches and repeated match iteration have different worst-case behavior; WT makes no unconditional linear-time claim.

`find_all` and `find_in` return a complete sequence within the configured limits or fail; never silently truncate. A `break` in the caller does not turn `find_all` into a first-match query. Use `is_match` when only existence is needed. Host implementations may use internal lazy storage only if complete-or-error behavior and limit checks remain identical.

Pattern IDs must resolve before scanning. **Deduplicate by the resolved expression, expanded flags, and engine semantics—not the local name.** The same expression under two names can share work; the same name with different expressions cannot. Distinct matching operations, regions, captures, and boundary semantics are not automatically equivalent (section 11).

### 6.5 Spans and findings

A reportable span is an opaque handle tied to one original file snapshot. A rule cannot forge a path, fabricate offsets, or emit for a file outside its declared inputs.

`start_byte` is zero-based and inclusive; `end_byte` is exclusive. Both are UTF-8 boundaries. Human line and column numbers are one-based; columns count Unicode scalar values, not bytes, display cells, or UTF-16 units. JSON includes byte spans and the declared display-coordinate convention.

Preserve CRLF and BOM bytes in snapshots and hashes. Derived strings and helper-canonicalized expressions have no reportable coordinates unless an explicit API maps them back. In particular, `rx::capture_text` must not manufacture an original span from a temporary string.

`rx::find_in` treats its input span as an independent haystack: anchors and boundaries refer to that slice. It is not equivalent to filtering whole-file matches to the same offsets. Query identity must record that distinction. Shared result storage may reuse offsets, but exposes fresh authorized handles and immutable per-consumer cursors.

### 6.6 Structural matching (`ast.v1`)

The core remains text-first and language-agnostic. A rule must explicitly request the versioned `ast.v1` capability for structural, syntax-aware matching. It is bundled/trusted code, not an arbitrary repository plugin, and is available for `python`, `javascript`, `typescript`, `tsx`, and `rust` (see `wt capabilities` for the languages and grammar versions an installed build actually has; each is an optional Cargo feature, on by default).

- `file.ast_match("language", "pattern")` returns a finite sequence of structural matches in the whole file. `language` and `pattern` must be string literals or constants, compiled once at rule-compile time; an invalid pattern or an unsupported/disabled language is a validation error, not a runtime one.
- `matched.ast_match("language", "pattern")` searches only inside a previous match's span (nested "X inside Y" queries).
- `file.ast_match_context("language", "context", "selector")` (also `matched.ast_match_context(...)`) matches ast-grep's contextual-pattern form instead of a bare pattern, for a node kind that cannot stand alone as one — a Rust `match` arm, a struct field, an attribute, a function parameter. `context` is a standalone snippet that must parse without an error node, and `selector` is the tree-sitter node kind inside it to actually match against; all three arguments are string literals or constants, compiled once at rule-compile time. An unparseable `context`, an unknown `selector` kind, or a `selector` that matches nothing in `context` is a validation error, not a runtime one. Metavariable captures work the same as a bare pattern.
- `matched.node("NAME")` returns the `$NAME`/`$$$NAME` metavariable capture from the pattern, or unit `()` if the pattern did not bind it.
- `matched.text` and `matched.span` behave like any other match; `matched.span` combines freely with `text.v1`, for example `rx::find_in(matched.span, "pattern")` to regex-search only inside a captured structural span.

A file with any tree-sitter error or missing node is an analysis gap for every `ast.v1` call that touches it — incomplete, exit `2` — never a silent no-match. Parsing only happens for files an unguarded `ast_match` call actually reaches; a branch that never requests the language never pays its parse cost. Parser/grammar and engine versions are query inputs and appear in `wt capabilities`, so a grammar upgrade that changes matching semantics is detectable rather than silently reinterpreted.

`span::contains(outer, inner)` is a general containment predicate: true when `inner`'s byte range lies within `outer`'s in the same file (equal spans count as contained; merely adjacent, non-overlapping spans do not). `ast_match`'s own nesting (re-searching within a captured span) can express "found inside Y", but not its negation; `span::contains` over two independently captured spans is what lets a rule state "X, unless some Y's span encloses it" — for example, a call that must run inside `thread::spawn` — directly, instead of approximating it with a loop-and-flag search that still cannot express the exception. It needs no capability of its own beyond whatever capability produced the spans being compared (`file.span`, `matched.span`, `line.span`).

New structural or other syntax capabilities must be versioned, bounded, pure, source-coordinate-preserving, and explicit in installed capabilities. Where an existing external checker is the better protection for a concern, the agent may use it outside WT instead of forcing a WRL1 detector.

### 6.7 Execution isolation and resource budgets

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

WRL1 logical cost is one step per executed statement (excluding a bare block), evaluated literal/local reference/property access, operator or host call, and visited loop element; operand evaluations are charged separately. A `for` statement is charged once for entering it and once per visited element in addition to the executed body. Short-circuited operands and untaken branches are not charged. Native queries additionally charge their input bytes (whole file, independent source slice, or the sum of text operands as appropriate); pattern/diagnostic IDs are not haystacks. This is a versioned WT cost table, not whatever operation-count definition a future Rhai release chooses. Charge native input bytes and returned elements on each logically demanded request, including a cache hit. Cache admission includes the effective resource profile and validated usage summary. The backend's own operation limit is additional defense, not the portable cost definition.

Frontend validation is also bounded (AST-node count and watchdog); its configured defaults and ceilings are listed in the runtime configuration reference. Use persistent worker processes, not one process per rule/file pair. Assign a complete file-local transaction to one worker so its applicable rules share the same snapshot and query arena. A worker compiles the required residual programs and matcher objects once per loaded version; separate processes may each have a compiled instance. Do not promise one physical regex object shared across process address spaces.

Per-consumer budget rejection must not poison an otherwise valid shared query. A query-wide failure is retained as an error and attributed to every consumer that actually demands it; it is not an empty cache entry. If a worker is killed, completed invocation results may be retained, but unfinished consumers are incomplete and the overall exit is `2`. Optional optimizer-group construction failure may instead fall back to individual queries, preserving semantics.

Bound all parsing, compiler work, IPC, text transformations, and native calls. Memory scheduling values are not themselves hard OS isolation; apply available process controls and kill/recover runaway workers. A timeout is a fail-safe, not a semantic predicate. Successful results must agree across valid physical plans; host-dependent exhaustion always remains explicit. This is defense in depth, not a formally proven hostile-code sandbox.

### 6.8 Automatic formatting

All new and updated `.wt` programs MUST be formatted before storage. Validate the original program, format it, validate the formatted program, then store it atomically and return the resulting digest. Formatting failures leave the active package unchanged.

The formatter uses four-space indentation, block structure, and one statement per line. It preserves token order, string/regex contents, comments, and source behavior. It MUST be idempotent and tested on strings containing comment markers, escaped quotes, Unicode, multiline calls, nested conditions, `else if`, empty blocks, and line/block comments. A parser/printer that discards comments is insufficient.

`wt fmt [ID]` formats local packages by default; `--global` explicitly selects global packages. `--check` writes nothing and returns `1` on a formatting difference. Invalid selected source returns `2`. Validate all selected sources before beginning writes, protect each replacement with its observed digest, and report any concurrent-write failure; do not imply a multi-package transaction if only per-package atomicity is implemented.

`wt check` never formats source. A formatter never hoists a query, changes scope, inserts an exception, or otherwise attempts semantic optimization.

## 7. Validation, detector fixtures, and evidence quality

### 7.1 Mode, diagnostic kind, and review state are independent

| Mode | Runs? | Blocking behavior |
|---|---|---|
| `advisory` | Yes. | Actionable findings do not block unless `--strict` is used. |
| `enforced` | Yes. | Actionable review or violation findings block. |
| `disabled` | No. | Visible exclusion; cannot satisfy an explicit enabled-coverage expectation. |

A `review` diagnostic establishes a review-worthy signal, not a confirmed bug. A `violation` establishes a source form prohibited by the documented adopted contract. A review trigger may be enforced: it requires disposition, not elimination of every raw match. Severity does not change this logic. Valid occurrence decisions are applied under section 10; failures always remain incomplete.

New rules default to advisory. A submission requesting enforced mode has the same fixture requirements as explicit promotion. An untested advisory rule may be used for exploration but must display `untested` evidence, not a passing test claim.

### 7.2 What validation establishes

`wt validate` checks package/schema structure, IDs, safe paths, regex compilation, the unoptimized WRL1 whitelist, static references/capabilities, globs, and fixture structure. It does not execute fixture inputs unless the command explicitly includes a test stage; output identifies stages actually performed. `wt test` runs detector fixtures. `new` and `update` validate, format, and execute all supplied fixtures before activation.

Every enabled rule's supplied suite must have a passing result before repository execution; a content-verified cached fixture result may be reused. A failed suite cannot be bypassed merely because the rule is advisory. Explicit `test` of a rule with no suite is an error. Enforced rules require a raw-positive case and an applicable nonempty raw-negative case; runtime-error or out-of-scope cases cannot be the only negative evidence.

These gates establish behavior against submitted examples. They do not certify application correctness, universal generalization, a person's approval, or whether an agent's evidence narrative is true.

### 7.3 Fixture format and semantics

Detector fixtures use schema **1**. A suite contains uniquely named cases, each with unique virtual file paths, exactly one inline `content` or package `fixture` reference per file, and an expected diagnostic multiset. Source spans are optional constraints. Extra and missing diagnostics fail. A runtime failure cannot satisfy an expected diagnostic.

Fixtures obey rule applicability and path scopes but not ambient repository/global scan exclusions, Git ignores, or occurrence decisions. Repository rules receive only the case's virtual file set. Missing required marker files can make a case non-applicable; output must expose that so it is not counted as a meaningful negative.

**An acceptable occurrence of an intended review pattern remains a raw-positive fixture.** Its acceptance belongs in a lifecycle scenario, not an empty expected-diagnostic array.

Keep the following evidence categories distinct in documentation and validation summaries:

| Category | Interpretation |
|---|---|
| Raw positive | The detector should recognize this situation, regardless of later acceptability. |
| Raw negative | The detector should not recognize this source form in its supported domain. |
| Outside scope | Not evaluated by this rule; not proof the code is safe. |
| Known miss | Intended concern is present, but the detector cannot currently recognize it. It is a coverage limitation, not reassuring negative evidence. |
| Expected analysis failure | Invalid/unsupported demanded input; belongs in engine/conformance scenarios rather than ordinary no-finding fixtures. |

The fixture schema is not overloaded with new result categories. Preserve ordinary detector cases and document known misses separately with stable case references in the Markdown or a non-executable evidence appendix. A future executable challenge-suite format needs its own advertised version.

### 7.4 Before/after and challenge evidence

A rule motivated by a known correction SHOULD retain the original relevant source region, its corrected counterpart, and provenance identifying which is actual source and which is synthetic. Do not copy secrets or unrelated private code into published packages. When retention is inappropriate, reference the protected source/evidence rather than fabricate a substitute and label it original.

Challenge recognition with meaningful supported variations: whitespace, identifier renaming, attribute order, multiple candidates, unrelated nearby code, comments/strings, and routine extraction/refactoring where the claimed detector domain includes it. Generated variations are proposals; their expected outcomes require review. Do not assume every transformation preserves behavior.

A held-out challenge or independent reviewer should attack the recognition claim, not merely repeat the author's assertions. Do not optimize for exactly one repository finding. A useful rule can legitimately report multiple review signals.

### 7.5 Preserve corrections to examples

`wt update` must show added/removed cases, changed input bytes, changed expectations, and changed scope separately. Existing `--allow-test-removal --reason` remains the explicit authorization for destructive replacement/removal; the report must not misleadingly label all expectation edits as the same operation.

Prefer preserving a previously misclassified input and correcting its expectation with a recorded reason, then adding a stronger independent example. Do not replace the input merely because it no longer fits the detector. Wrong expectations are correctable; convergence is not the permanent preservation of earlier mistakes.

Update previews and VCS review retain the prior package digest and a structured change summary. Direct file editing remains supported, but protected acceptance must review these changes. Append-only occurrence history is a separate requirement; wt does not add another mandatory rule-history database.

### 7.6 Application verification is a separate layer

Record whether originating application evidence is hypothesis, statically established, reproduced, or awaiting verification; report exact commands/results only when executed. A source checker that passes after removing the old spelling cannot verify that the replacement code builds or fulfills the application contract. WT does not install application toolchains or silently run shell commands to manufacture such evidence.

## 8. CLI contract and normal workflows

All commands are non-interactive. Machine use must not require an interactive editor, confirmation prompt, network access, or application checkout beyond the selected sources. Replacing/accepting mutations use explicit arguments and expected hashes, not prompts.

Common selection options remain `--root`, `--global-dir`, `--no-global` where meaningful, `--format text|json`, and `--color auto|always|never`. Reject irrelevant/conflicting options. JSON stdin is preferred over shell-quoted source strings.

Anywhere a finding ID or evidence digest is accepted (`wt inspect`, `wt review ... --expect-evidence`, `wt reviews`), the shortest-unique hex prefix `wt check` prints for it (at least 8 characters, with or without the `sha256:` marker) is accepted in place of the full 71-character id; a full id keeps working exactly as before. Resolution happens in `wt-core`, against the run's current findings and stored reviews, so it applies identically whether the caller is a human or a JSON-driven agent. An ambiguous or unmatched prefix is a clear `2`-exit error listing the full candidate ids; `--watch PATH=sha256:HASH` file-content digests are a separate namespace and are never shortened or prefix-matched.

### 8.1 Command surface

| Command | Contract |
|---|---|
| `wt init [--global]` | Idempotent creation of configuration/rule directories and documented lock hygiene; never overwrite existing policy. |
| `wt capabilities --format json` | Compact installed build, protocol/schema/language/capability, resource-support, and guide identity report; offline and read-only. |
| `wt guide [TOPIC]` | Installed release-matched authoring guidance. Topics include `author`, `review`, `language`, and `stats`; never fetch mutable online content. |
| `wt new JSON` / `--stdin` / `--file PATH` | Exactly one self-contained submission; default local/advisory; validate, format, test, then atomically create. |
| `wt update ID --stdin --expect-hash HASH` | Replace a package atomically, preserving ID, checking concurrent edits and changed examples; accepts `--preview`. |
| `wt check [PATH ...]` | Whole selected root by default, all enabled global/local rules; raw detection followed by current disposition. |
| `wt check --submission PATH` | Preview one uninstalled self-contained candidate with normal file selection and fixture gates; no persistent package/review changes. |
| `wt fmt [ID] [--check] [--global]` | Format detector source, or report differences without writes. |
| `wt plan [--rule ID]` | Inspect guarded queries and sharing decisions without scanning application contents. |
| `wt stats` | Rerun a check with the same scope/flags as `wt check` and report each rule's mode, severity, intent, in-scope file count, unmatched include globs, raw findings, review decisions by outcome, and a derived `disabled`/`unknown`/`useful`/`dead`/`watch`/`quiet`/`noisy`/`active` signal; see `wt guide stats`. |
| `wt list` | Identities, origin, declared/effective mode, scope, and fixture availability. |
| `wt show ID` | Human contract or self-contained JSON export with its same-read package digest. |
| `wt validate [ID]` / `--file PATH` | Validate package/submission; compact success and actionable failures. |
| `wt test [ID]` | Run selected/discovered detector fixtures, never application code or occurrence acceptances. |
| `wt review FINDING_ID --decision ... --expect-evidence HASH --reason-file PATH` | Record one evidence-bound decision; optional `--watch` and required `--expect-hash` for a replacement. |
| `wt reviews [FINDING_ID]` | Inspect durable decision records and Markdown reasons, labeled `not_evaluated` until checked against a current scan. |
| `wt inspect FINDING_ID` | Fresh, read-only evaluation and bounded review context: raw finding, contract, current/prior decision, evidence changes, and exact IDs for a subsequent review. |
| `wt set-mode ID advisory|enforced|disabled --reason TEXT` | Explicit policy change; enforcing validates fixtures; reason retained. |
| `wt explain PATH [--rule ID]` | Explain scope, ignores, applicability, expected coverage, and gaps. |
| `wt config` | Separate configured and invocation-effective values, with origins. |
| `wt schema NAME` | Installed version-1 schemas; names include rule, submission, tests, config, result, plan, review, capabilities. |
| `wt cache clear` | Delete derived WT caches only. |

`capabilities`, `guide`, `inspect`, candidate preview, and structured update preview are current wt interfaces. Their presence is not inferred from version strings; an installed executable advertises support.

### 8.2 A minimal normal loop

```bash
wt capabilities --format json
wt guide author
wt new --stdin --format json < proposed-rule.json
wt check --format json
```

Do not require `validate`, `new`, `test`, and repeated full `check` commands when creation already performed the same fixture gate and nothing changed. Explicit validation and tests remain valuable during drafts or debugging. Output must identify which stages ran and which verified results were reused.

After a review signal is inspected:

```bash
wt inspect "$FINDING_ID" --format json
wt review "$FINDING_ID"   --decision acceptable   --expect-evidence "$EVIDENCE_DIGEST"   --reason-file rationale.md
wt check --format json
```

Shell variables above stand for actual returned identifiers, not hashes invented by the author. If the decision already exists, provide the current review record hash as `--expect-hash`.

### 8.3 Draft preview and update preview

A candidate preview does not install, enable, disable, or migrate a rule. It analyzes the submitted detector alone, labels identity `candidate/<id>` in the result output, excludes existing occurrence acceptances, and reports actual scope and findings. It does not purport to satisfy the repository's active coverage policy. Candidate IDs cannot receive durable review decisions.

An update preview checks the current digest and prepares exactly the formatted candidate package that a real update would store. It displays code/docs/scope/mode/test changes and fixture results. It does not silently run an expensive repository comparison; the caller can explicitly run a candidate scan. A later real update repeats the digest guard; a preview is not a write reservation.

### 8.4 Check options retained

`--include-ignored` bypasses Git ignore filtering only. `--no-host-ignores` excludes Git-local/user ignores but keeps repository `.gitignore`. `--rule ID` is repeatable and resolves qualified identities. `--strict` blocks actionable advisory findings. `--no-cache` bypasses persistent caches but keeps in-run sharing. `--optimizer auto|off` selects the optimized or unshared reference path. `--jobs N` and `--max-file-bytes N` remain bounded by the published resource profile. `--stats` requests measured work. `--allow-empty` permits ordinary empty work but never overrides coverage expectations or analysis failures.

`--show-reviewed` expands human output; `--detail summary|full` selects the inventory projection. Full output is explicit, not an implicit plan dump. Default output always includes all actionable findings and errors; it does not silently truncate them.

Ordinary `check` never changes source, rule files, modes, or review records. It may write disposable caches. It does not silently advance an authoritative “last seen” state. Without an explicit historical scan input, label work `unreviewed`, not “new since your last scan.”

### 8.5 Changed-file checks

Default `check` is a full current-root scan with content-verified caches. `--changed [--base REV]` is explicit partial coverage. Resolve a local Git base without fetching, hooks, content filters, or application execution. Select existing changed tracked files, eligible untracked files, rename destinations, and report deletions. `--base` without `--changed`, non-Git roots, or simultaneous positional paths are errors.

File rules run only on the selected changed set. Repository rules receive their full normal authorized scope, because a single deletion/change can affect a cross-file property. Explicit ordinary path selection instead narrows the authorized set and remains partial. A partial repository input set cannot authorize a durable acceptance or reuse a full-context acceptance as though it had been revalidated.

An empty changed set is explicit `no_work` with exit `0` when there are selected enabled rules and no repository reduction requiring execution. Changes to rules or policy warrant a full scan; warn when locally detectable, without claiming to observe every global-policy change. Review records not evaluated by a partial run remain `not_evaluated`, not fixed or current.

### 8.6 Exit codes and errors

| Code | Meaning |
|---|---|
| `0` | Requested operation completed without blocking actionable findings; a valid current acceptance can satisfy a review obligation. |
| `1` | Completed check has blocking actionable findings, a fixture expectation differs after successful execution, or `fmt --check` found differences. |
| `2` | Invalid invocation/schema/rule/policy/review state, stale mutation evidence, capability mismatch, source gap, unsupported demanded input, or incomplete evaluation. |
| `130` | Controlled user interruption, where supported. |

Incomplete evaluation takes precedence over findings. Retain valid partial output but return `2`. Zero selected enabled rules or no eligible files returns `2` unless an ordinary empty scan is explicitly allowed. Never turn missing required evidence into a successful empty result. Successful narrow previews are labeled previews, not passing repository policy.

Argument failures use structured JSON when the requested output format/version can be parsed. No ANSI sequences, log banners, or progress lines appear in JSON stdout. Progress and developer logs use stderr.

## 9. Actionable diagnostics, compact output, and inspection

### 9.1 The normal output is the work queue

A human result should foreground unreviewed/current-open findings and stale acceptances, not repeat every accepted occurrence or internal query node. Text output uses a `rustc`-style block per actionable finding: an `error`/`warning` label (computed from blocking status, not mode/kind, which remain in JSON and `wt inspect`), a `rule_id`, the message, a `--> path:line:col` pointer, one line of source context before and after the matched span with a caret underneath it, and `= key: value` notes for `status` (`needs review`, `confirmed issue`, or `violation`; a reopened stale decision says why), `help`, the evidence prefix, and the `wt inspect` command to run next. For example:

```text
warning[local/current-cwd-label]: label cwd Current shell directory only at ...
  --> crates/kea-app/src/app/rendering.rs:377:44
    |
376 |     <previous line>
377 |     <matched line>
    |                                            ^^^^^^^^^^^^^^^^^^^^^^^
378 |     <next line>
    |
    = status: needs review
    = help: Verify the branch uses ...
    = evidence: bb2c1904
    = inspect: wt inspect aa1af802

2 known issues · 0 need review · 0 blocking · 12 accepted · 143 files in 0.2s
```

This is an illustrative result, not a measured test. A finding already covered by a current `confirmed_issue` decision is instead a one-line entry in a trailing `Known issues:` section, so it stays visible without burying new findings; `--show-reviewed` (or `--detail full`) adds a similar `Accepted:` section for current `acceptable`/`accepted_risk` decisions, otherwise hidden. The closing summary line's counts and nouns are singular/plural-correct (`1 known issue`, `1 needs review`, `1 file`); an incomplete analysis is reported as a loud, separate `error: analysis incomplete` line first, never folded silently into a clean-looking summary. Severity, `review` versus `violation`, advisory versus enforced mode, and computed blocking status must be visually distinct. An advisory error-level finding is not a failed command unless strict mode applies. Color follows `--color`, a non-empty `NO_COLOR`, and TTY detection; pipes never carry ANSI codes.

An occurrence diagnostic must say what was recognized. A text prefix match cannot assert that an acknowledgement was bypassed; a same-named `.filter()` cannot assert that displayed state changed. The Markdown contract explains how to investigate the remaining question.

### 9.2 Raw occurrence fields

Each raw occurrence retains qualified rule ID, package digest, diagnostic code/kind, severity, effective mode, original path/byte span, one-based Unicode-scalar display coordinates, source digest, and current message/help. Evidence-bound output additionally carries `finding_id`, `matched_digest`, `context_digest`, `engine_digest`, and `evidence_digest`, plus additive `status` (the human triage bucket: `needs_review`, `confirmed_issue`, or `violation`), `finding_prefix`/`evidence_prefix` (the shortest-unique display prefix for this run), and a `snippet` (the checked source's line before, the matched span's first line and caret-end column, and the line after; `null` when source text was unavailable) that text rendering uses instead of reopening the file.

Different rules retain separate findings even on identical source spans. Exact duplicate emissions from one rule may be deduplicated in stable order. A rule invocation that fails cannot supply a supposedly complete finding set; incomplete status survives any retained earlier findings.

Related evidence may be displayed only when actually available with verified source coordinates and a documented helper/API origin. WRL1's existing `emit` does not magically produce related spans. Do not invent a relationship merely because a regex captured a similar name. A future diagnostic API for related locations requires advertised capability support.

### 9.3 Result protocol

The compact projection is the one result protocol wt has. Every ordinary structured command result identifies `schema_version`, command, status, exit code, and detail projection. Check results use their documented top-level fields; other normal command results put command-specific payload in `data` with explicit `stages`, `errors`, and `notices`. `capabilities` is the separately versioned standalone capability document, and `schema` emits the requested schema document; neither is falsely wrapped as a check result. Check results include completeness, selection scope, effective policy, coordinate encoding, all actionable `diagnostics`, all `errors`, coverage expectation results, notices, partition counts, and output-inventory availability.

The two raw-result partitions are disjoint:

- `diagnostics`: actionable raw findings, including unreviewed, confirmed/open, needs-review, and stale acceptances.
- `reviewed`: raw findings covered by a current acceptable or accepted-risk decision.

`summary.raw_occurrences = actionable_occurrences + reviewed_occurrences`. Failed invocations cannot imply additional unseen raw occurrences are zero; completeness/coverage remains separate. Blocking counts are computed over actionable findings only. Review and violation counts are labels, not additional disjoint partitions.

Default `summary` detail includes all actionable findings/errors and accurate counts for accepted occurrences, but omits their full arrays and the full source inventory. `inventory` explicitly says which lists are included. `--detail full` includes `reviewed`, `files`, and `review_records`. Omission is never represented as an empty array that falsely claims no records exist.

### 9.4 Useful stale-decision inspection

`wt inspect FINDING_ID` must provide the current raw finding; rule Markdown; previous decision/rationale/record hash; current evidence hash; and machine-readable stale reasons. At minimum distinguish owner file, rule package, runtime semantics, repository input inventory, watched file, and missing/unavailable dependency changes.

Where earlier source bytes are available through a verified local VCS object or an explicitly retained bounded evidence attachment, show the relevant diff. Otherwise show changed paths/digests and state `previous_content_unavailable`; do not fabricate a diff or copy the complete repository into review storage by default.

An unchanged finding hash is not evidence that context is unchanged. Conversely, a moved occurrence may have a new identity; show possible related history only as a non-authoritative suggestion when available, never as inherited approval. A missing occurrence is described with actual coverage information, not automatically labeled fixed.

### 9.5 Output and verification transparency

Routine `validate` success contains identity, validity, and stages performed/reused—not a full AST/plan. Routine `test` shows counts and failed-case details; full individual results are explicit. `plan` is the place for detailed query graphs. `config` reports final CLI-applied values consistently with the check's effective policy.

All semantic arrays are deterministically ordered. Operational counters/timing are optional and outside semantic equality. An unavailable metric is `not_measured`, not zero. Do not count binary skips or out-of-scope files as analyzed source. There is no default silent actionable-finding truncation; an explicit future paging/output-limit contract must disclose omitted work without changing enforcement.

## 10. Occurrence-level review decisions

### 10.1 Stateless detection, stateful interpretation

```text
source snapshots + rule definitions
    -> raw detector findings

raw findings + current policy + stored decisions + current evidence
    -> actionable findings and current reviewed findings
```

Rule programs MUST NOT read or mutate review records, inspect whether a finding was accepted, or change other rules. Detector fixtures run without decisions. Occurrence state belongs to the host's post-detection layer; a correct raw match stays visible through inspection even when accepted.

### 10.2 Decision and freshness are different dimensions

| Decision | Meaning | Effect when evidence is current |
|---|---|---|
| `needs_review` | Reopen or leave a question unresolved. | Actionable. |
| `acceptable` | This review signal is justified in the recorded context. | Reviewed; permitted only for `review` diagnostics. |
| `confirmed_issue` | Contextual review found the concern applicable. | Actionable; no implicit severity/mode change. |
| `accepted_risk` | The concern applies but leaving it is explicitly authorized. | Reviewed; may apply to either diagnostic kind. |

The CLI uses hyphenated decision names where needed; JSON uses the underscores above. `acceptable` must not silence a `violation`. “Seen,” “not investigated,” or “we need the check green” are not acceptance decisions.

Freshness is current, stale, not evaluated, or invalid. Observation is present, absent after comparable evaluation, or not observed because coverage differs. These are not compressed into a single ambiguous state. Missing findings are never automatically declared fixed. Listing stored records alone reports validity `not_evaluated`.

A stale acceptance becomes actionable when its occurrence is detected again. An absent/moved occurrence has no actionable raw finding to attach to, but its historical decision remains inspectable with the applicable absence/coverage reason. Invalid review storage makes the operation incomplete; it does not suppress findings.

### 10.3 Finding identity versus validity

Retain the conservative review-v1 identity boundary: qualified rule, diagnostic code, exact root-relative path, byte span, and matched bytes. IDs are opaque SHA-256 values returned by WT. The source location is not a guarantee of semantic identity across refactors.

Do not automatically fuzzy-match, normalize, rebind, or transfer approval to copied/relocated code. A new copy is a new review obligation. If an old location is reused while the owning file changes, the evidence digest must invalidate the prior decision even if the finding ID happens to match.

Exact relocation can generate a new unreviewed finding rather than a stale instance under the old ID. This is a deliberate conservative boundary, not a claim of stable semantic tracking. More precise correspondence is deferred until one-to-one matching and invalidation behavior are specified and independently tested.

### 10.4 Required evidence for a current acceptance

A decision is bound to the exact rule package, matching/helper/runtime semantic identity, owning source bytes, relevant detector input context, and any explicitly watched supporting files. A file rule's minimum context is its whole owning file; a repository rule includes its full authorized input inventory with normalized paths and content hashes. Additions and deletions invalidate repository-rule context even if the original matched line is unchanged.

The runtime semantic identity (`engine_digest`) is not a hash of the whole compiled binary: it is a maintainer-controlled epoch plus the exact pinned versions, read from the build's `Cargo.lock`, of the dependencies that implement matching/scoping semantics for the rule's declared capabilities (the WRL1 interpreter; `text.v1`'s regex engine; `path.v1`, `repo.v1`, and scope globs; and, for rules declaring `ast.v1`, the ast-grep engine and each pinned tree-sitter grammar). A dependency version bump — a grammar update included — changes it automatically and reopens affected decisions with no epoch bump needed. An interpreter or matcher semantics change with no dependency bump requires the maintainer to advance the epoch by hand; that is the only other way this identity moves. A capability added after a given release folds its own pinned dependency into the identity only for rules that declare that capability; rules that don't keep the identity they already had.

Additional `--watch PATH=sha256:EXPECTED_HASH` dependencies are explicit literal file paths, at most 32 in review schema 1. WT verifies the hash supplied from the material actually reviewed. It does not invent the expected evidence for an old review, follow symlinks, or treat unreadable dependencies as unchanged. Supporting files outside detector scope remain bounded repository-local reads with separate accounting. A confirmed missing watched path makes an acceptance stale; it is not by itself a parser/runtime failure. An I/O error that prevents determining dependency state is reported as an evidence-evaluation error and makes the overall check incomplete. Neither condition leaves the acceptance current.

For a review depending on the *absence* of alternative implementations, individual existing file watches are insufficient. The author must either bind to a detector/repository input inventory that covers that assumption, use a sufficiently broad existing boundary, or leave the finding unresolved. This revision does not pretend that prose alone watches future files. Arbitrary dependency glob/negative-watch records require a later review-schema version; no undocumented fields are added to review v1.

Whole-file invalidation intentionally reopens some unrelated edits. A current decision means only “all recorded evidence still agrees,” not “all possible semantic dependencies are known.” External services, requirements, DNS/network behavior, and unrecorded files remain outside that guarantee.

### 10.5 Fresh review-write protocol

`wt review` receives an actual finding ID, expected evidence digest, decision, and nonempty rationale file. For replacement it also requires the current stored record hash. Each call concerns one occurrence; no bulk-accept flag or automatic baseline is supported.

The command MUST:

1. Resolve the intended loaded policy and validate existing review history.
2. Run a fresh uncached detector check with the rule's full normal evidence scope. Rule selection may limit independent rules, but a partial path/changed-only context cannot establish an acceptance.
3. Find exactly one eligible occurrence and reject missing, ambiguous, candidate-preview, incomplete, or stale evidence.
4. Read bounded rationale bytes and verify supplied watched hashes. Reuse source snapshots within this operation, then revalidate watched/owner/input-inventory evidence before publishing when concurrent edits are detectable.
5. Under a per-record lock, recheck the expected previous record hash and atomically append a complete new revision. Publish nothing on failure.
6. Return the written record hash, decision, evidence identity, and storage path. Do not claim authenticated human approval merely because a rationale exists.

Each source set is a working-tree read, not a filesystem-wide atomic snapshot. A concurrently changing source must never be represented as a verified old snapshot; detected races reject/reopen. An immutable external checkout is the stronger acceptance boundary where required. The command never fixes application code or changes a detector to make a decision valid.

### 10.6 Durable storage and concurrency

```text
.wt/reviews/<finding hash without sha256:>/
  00000001/decision.json
  00000001/rationale.md
  00000002/decision.json
  00000002/rationale.md
```

Review schema **1** retains `finding_id`, `evidence_digest`, `matched_digest`, `rule_digest`, `file_digest`, `context_digest`, `engine_digest`, qualified `rule_id`, diagnostic `code`, path/span, decision, `watched_files`, `rationale_file`, and `previous` record digest. The record digest covers the structured decision and actual rationale bytes using the pinned review profile's framing; identity/hash golden vectors are an implementation qualification requirement, not fabricated by the example package. WT must return it; callers never derive approval from a guessed checksum.

Revisions are numbered, bounded, append-only through the CLI, and linked to the preceding record. Partial directories, malformed chains, duplicate revision numbers, out-of-root paths, invalid IDs, and symlink/reparse escapes are errors. Manual editing remains physically possible but cannot evade the same validation. Hash chains provide integrity/identity, not signatures, independent authorship, or authorization.

Concurrent edits with the same expected record/package hash must not silently overwrite one another. Review records and rule updates are distinct transactions; a rule change after a review invalidates the review. Cache clearing must not touch `.wt/reviews`, fixtures, Markdown, or approval history. Ordinary checks do not append observation timestamps or decisions.

### 10.7 Disposition ordering and failures

Validate all loaded policy/review state, execute raw detection, then evaluate occurrence decisions. `diagnostics` and `reviewed` occupy separate disjoint output partitions.

An unchanged acceptance cannot excuse a demanded parser failure, invalid rule, failed fixture gate, missing required source, or incomplete repository context. If unrelated independent work fails, the overall check is incomplete even if some other decisions can be evaluated; those evaluations never imply full-policy success.

`confirmed_issue` and `needs_review` remain actionable. Valid current `acceptable`/`accepted_risk` records satisfy their occurrence even in strict mode. Policy still determines who may supply those records under a trusted outer boundary.

### 10.8 Review burden and correction

New evidence may show an old judgment was wrong. Append a new needs-review/confirmed-issue/acceptable decision with its corrected rationale; preserve prior reasoning rather than rewriting history. Retiring a detector changes policy explicitly and does not declare old findings fixed.

The system should help users distinguish useful reopenings from repeat work caused by overly broad evidence boundaries. Do not lower evidence protection just to improve a benchmark. Measure second-encounter outcomes and refine supported boundaries deliberately. Human and agent reviewers are both allowed under project policy; every routine review need not become a human sign-off ceremony.

## 11. Shared execution planning, scheduling, and caching

### 11.1 Required execution model

**Rules are independent policy units; expensive read-only operations are shared execution units.** Changing, disabling, or failing a rule does not redefine another rule's detector. WT—not the rule author—owns file discovery, I/O, query scheduling, deduplication, and parallelism.

The default engine must not run one independent repository scan per rule:

```text
load and validate all selected rules
    -> verify retained fixtures
    -> validate residual control flow and extract guarded query templates
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
    -> apply current modes and evidence-bound occurrence decisions
    -> stable output and coverage accounting
```

“Read once” means one source snapshot read per file in a normal invocation, not one byte access for all future operations. Hashing, parsing, and different regexes may still traverse those bytes. Git metadata reads, fixtures, and compiler inputs are separate. Working-tree race detection can require metadata checks/retries, which must be counted. Warm runs still verify content rather than trusting only timestamps.

File-local rules must not force all repository contents into memory. File-level parallelism is primary; do not assign every rule for the same file to a separate worker and lose locality. Scope/path masks should eliminate irrelevant rule/file pairs before source matching.

### 11.2 Logical plan versus physical plan

The **logical plan** records what each rule requests: source inputs, scoped query templates, branch guards, dependent spans or temporary text, local predicates, and diagnostic sinks. A query DAG may include parameterized nodes—for example a regex over every input attribute's canonical text. It need not statically enumerate every runtime substring.

The **physical plan** chooses memoization, optional multi-pattern batches, scheduling, and storage. It cannot alter the logical meaning. The host—not arbitrary script execution—extracts operations and its Rust query service executes/reuses them. A full backend-independent residual IR is optional; query inspection must truthfully state what is actually modeled.

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

### 11.3 The shared-pattern example

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

### 11.4 Query identity and memoization

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

### 11.5 Demand, absence, and failure semantics

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

### 11.6 Multi-pattern execution

**Literal presence.** When enough compatible literal checks operate on the same file or region, a native multi-literal engine such as Aho-Corasick may return a Boolean per literal in one shared traversal. It must retain substring/overlap hits: `foo123` satisfies both `foo123` and `foo`. Ordinary leftmost non-overlapping matching can lose one of those identities; use all-pattern presence semantics, such as Standard overlapping search, or an equivalent proven algorithm.

The empty literal has presence `true` without a scan. Keep case/Unicode transformations explicit. WT's baseline `contains` is case-sensitive exact string presence. Grouping is by the same input region and matching semantics, not merely by strings occurring somewhere in a rule. Small regions may be cheaper with separate native substring tests; use measured thresholds rather than mandatory batching everywhere.

**Regex presence.** `RegexSet` can identify which expressions match, but does not supply each rule's full match offsets or captures. Use it as an optional presence query/prefilter, followed by each demanded canonical exact query when needed.

A negative exact presence result can synthesize an empty `find_all` result within the same input/flag semantics. Positive presence is not a finding and cannot supply captures. Group patterns with compatible flags; preserve original per-pattern semantics and membership mappings. Never replace independent regex iteration with one alternation whose priority discards overlapping rule matches.

**Required literals.** Derive a rejection condition from the parsed regex representation only where absence proves no match. For alternation `foo|bar`, absence of `foo` alone is insufficient. Handle optional/empty matches, case-insensitive patterns, Unicode, and anchors correctly, or decline the optimization. User-provided metadata is not proof of a necessary literal.

**Cost control.** Combined automatons can be expensive to build and retain, and many positive regexes may require a second traversal for exact matches. Cap group sizes, compile memory, and batch complexity; fall back to individual matching without reducing correctness. Do not claim that all rules or all arbitrary regexes execute in one pass or constant time.

### 11.7 Explicit repository rules

Retain `execution: "repository"` for deliberate cross-file checks, but keep it outside the normal file-local fast path. The body receives only `repo`; its `repo.files()` sequence is a view over the coordinator's selected, authorized snapshot set.

The coordinator discovers files once, computes each rule's applicable set, and retains relevant snapshots within repository budgets while the file-local stage runs. Repository reductions execute after those inputs are finalized. They can request the same native query service and reuse retained derived results. They cannot perform an independent directory walk, read files by a constructed path, or observe newer disk contents than the file-local stage.

A repository rule may use bounded local counters/strings and nested iteration over finite sequences. It has no mutable global state or results from another rule. Its result cache depends on its **complete input manifest**, including normalized paths, content digests, applicability facts, and additions/deletions. Results from one file cannot establish a cross-file invariant on their own.

Explicit path selection narrows repository-rule inputs too, and is reported as partial. In `--changed` mode, repository rules instead receive their full normal input scope, so cross-file checks are not silently evaluated only on changed files (section 8.5). Any required snapshot gap makes that repository invocation incomplete.

### 11.8 Persistent caches and invalidation

Keep separate layers:

| Layer | Key / invalidation principle |
|---|---|
| Validated package and fixture result | Package/test bytes, WRL/backend/helper semantics, resource profile. |
| Compiled matcher / residual artifact | Canonical pattern/program definitions and binary/backend build identity; never trust serialized backend objects from untrusted storage. |
| In-run shared query results | Exact query identity for the current authorized snapshot and input region. |
| Persistent raw rule/file findings | Rule execution identity, normalized file path, exact content digest, effective applicability/scope facts, semantic versions, and logical limit profile. |
| Repository-rule findings | Rule identity plus full authorized input manifest, including missing/present applicability inputs. |
| Review disposition | Revalidate evidence and current review record; no cached approval from an earlier context. |
| Final rendering/enforcement | Recompute with current modes, diagnostic presentation, review disposition, selected roots, and strictness. |

**Do not key every file result solely by the entire ruleset hash.** Adding rule B should invalidate/rebuild the combined plan and evaluate B, while unchanged rule A's still-valid raw results remain reusable. Editing one rule must not force unrelated rules to re-execute on unchanged inputs. A global helper/runtime semantic change can legitimately invalidate many entries.

The complete package hash is an acceptable conservative per-rule identity; narrower semantic digests may improve reuse if all observable inputs are accounted for. Pattern-sharing keys exclude aliases, author prose, and rule IDs, but rule findings remain attributed to their own identities. Scope and file path matter even when content is equal.

Mode/decision changes must take effect without stale enforcement: store raw diagnostic codes/spans and apply current policy and validated occurrence decisions afterward, or include every relevant policy input in final-result keys. Every enabled supplied test suite must still have a valid passing entry before using source-result caches.

Read/hash selected file contents to verify warm hits. Size/mtime alone is insufficient. Warm unchanged runs can avoid regex/helper/control-flow work, not necessarily all filesystem traversal or hashing. `--changed` is an explicit narrower selection, not an implicit cache heuristic.

Never promote failed, truncated, interrupted, unstable, or budget-exhausted results into complete cached results. Corrupt entries are discarded. Cache write failure falls back to uncached checking with a notice. Caches contain no original source or full snippets by default; store coordinates and identifiers. Hashes are not authentication: use `--no-cache` or a trusted cache in protected acceptance runs.

All public digests use SHA-256 (`sha256:` plus 64 lowercase hex characters). File digests cover original bytes. Package digest framing remains `WT-PACKAGE-1`: sorted normalized referenced paths with length-prefixed path/content bytes; the manifest's schema/language fields participate. Derived execution/query/policy identities use separate versioned domain separators and golden serialization tests. A `plan_digest` describes the combined plan, not the sole key for individual-rule results. Hash framing uses UTF-8 paths, unsigned 64-bit big-endian length prefixes, sorted unique referenced paths, and a length-prefixed domain separator. Semantic-key tuples have explicit type tags and fixed field order; implementations must publish golden vectors rather than rely on platform-native serialization.

### 11.9 Inspection and reference execution

`wt plan [--rule ID] --format json` validates/compiles selected rules and reports their static guarded query templates, scopes, canonical pattern aliases, consumers, sharing candidates, and chosen optimization strategy. It does not execute fixture cases or read application source contents; data-dependent regions and final batching thresholds remain explicitly unresolved. It reports no invented timing, file counts, or completed analysis.

`wt check --stats` reports actual work separately from semantic findings: snapshot reads/bytes hashed, relevant rule/file pairs, residual invocations, result-cache hits, logical query requests, physical query evaluations, shared-result hits, parser evaluations, regex/literal batch work, peak memory, and timing. If showing “scans avoided,” label whether it is a model-derived estimate rather than a measured counter. Do not count metadata reads as source reads or hide worker-local compilation costs.

`wt check --optimizer off --no-cache` is the correctness reference path: keep the same snapshots, language, limits, scope, and output semantics, but disable cross-rule query sharing and multi-pattern/prefilter optimizations. It is a diagnostic/conformance option, not the default architecture. It may still compile each pattern once per worker and keep file-centric I/O.

Compare optimized/reference, cold/warm, serial/parallel, and permuted-rule-order runs. Their stable diagnostics, selected coverage, and logical classifications must agree on workloads that complete within physical safety limits. Hardware timings, cache statistics, and plan descriptions may differ. An exhausted optimized run cannot claim success merely because the reference might finish, or vice versa; either is explicitly incomplete.

### 11.10 Performance guarantees and boundaries

The design eliminates redundant work; it does not make accumulating arbitrary rules free. Runtime can still grow with the number of **distinct** expensive queries, rule-local predicates, total matching output, compiler work, and repository reductions. Many findings impose unavoidable output cost.

Required optimization priority is source locality and sharing of whole-file searches/parses. Cheap predicates over short match regions do not require sophisticated fusion. Preserve a simple native substring path when it costs less than building/querying a batch automaton.

The performance acceptance matrix in section 14 tests both highly shared and completely distinct rule sets. Do not advertise constant-time scaling, guaranteed convergence, or Ruff-equivalent speed without implementation evidence.

### 11.11 Constant-size hot-path identity and immutable payloads

A shared-result hit must not rehash or deep-copy the complete file merely to find the cached result. Compute a source digest once for the immutable snapshot within a process/transaction and reuse it. Use bounded interned query/pattern identities and source-region handles for in-run lookup. Distinct snapshots at the same path and length must remain distinct; raw allocation addresses without safe lifetime ownership are not sufficient identity.

Across process boundaries an independently verified worker snapshot may require a separate digest. Count it explicitly. Do not claim literally one hash across coordinator, workers, fixtures, and review writes when each performs its own verification. The required scaling property is that unchanged snapshot hashing does not multiply with each consuming rule/query.

Represent source text, regex matches/captures, parsed attributes, and shared sequences as immutable views/shared storage where practical. Each consumer retains independent iteration and authorized source handles. A cache hit should not clone the entire matched-text/capture payload. Owned transformations remain bounded; derived views need correct digest/lifetime accounting. Full-value string equality still compares content, not only pointer identity.

Retain per-consumer logical budgets independently of physical sharing. A cached result cannot make an over-budget logical invocation valid. Account actual physical allocations separately; metrics must not confuse conservative logical charges with copied bytes.

### 11.12 Residual work and review cost are real work

Shared regex enumeration does not remove an M-by-N comparison loop over its results. Instrument residual iterations and avoid claiming one physical query means constant execution time. Do not hoist an inner query out of a guard solely to speed it up; retain demand/failure semantics. Add indexing/fusion only after evidence of material cost and a compatible bounded contract.

Track source hashing, query-key hashing, physical copied/retained bytes, compilation, shared hits, native scans, residual execution, and review-evidence reads separately. `cache_key_bytes_hashed` must include work not counted by the source loader. The host should group identical reviewed owner/dependency reads within one operation rather than reread/hash the same file once per decision. Fresh review writes still revalidate evidence; no stale acceptance cache substitutes for that check.

The planner must distinguish static operation templates from resolved reusable identities. Different `group_text` arguments or receivers cannot share solely because the operation name agrees. An inspection field called `canonical_identity` must identify actual equivalent semantics or be replaced by an explicitly non-shareable template identity.

## 12. Agent authoring, review, and trusted enforcement

### 12.1 Release-matched guidance

`wt capabilities` and `wt guide` must work offline and be generated/bundled with the executable, its schemas, and its runnable examples. The report includes build version/commit (or explicit unknown), supported specification profile, output/schema versions, WRL/helper semantics, feature flags, and effective resource/isolation support.

A feature described only on mutable `main` must not be assumed installed. A requested unsupported workflow is a visible compatibility error or explicitly labeled legacy fallback; never silently claim to have tested the new features. Package installation/upgrading remains an authorized external operation, not a detector side effect.

### 12.2 The authoring contract

The external agent should understand the motivating concern, verify the intended source contract, inspect existing protections, and choose an exact guard, review trigger, adopted policy rule, or non-WT protection. It should prefer a small claim, not a tiny accident-specific source scope.

Create meaningful raw positives and negatives, retain before/after evidence when available, challenge supported variations, inspect both findings and likely misses, and report actual coverage. Independent review should receive the actual code/fixture diff and be asked to construct counterexamples. Do not substitute repeated assertions or a passing generated suite for application evidence.

Recognition errors change detectors. Contextual acceptances become occurrence decisions. Accepted risk needs explicit authority. Do not target a quota of findings, bulk-accept current output, or narrow detection solely to pass a check. Correct old examples with recorded reasons rather than quietly replacing them.

The agent must distinguish evidence states in its report: detector validated; fixtures executed; source occurrence matched; application claim statically established or reproduced; application tests blocked/not run; review decision recorded and evidence current. WT cannot infer these statuses from prose.

### 12.3 Feedback without duplicated work

Successful new/update output states which checks ran and which verified artifacts were reused. Repeated unchanged schema, plan, fixture, and whole-repository retrieval should not be a mandatory ritual. Use targeted preview while authoring and full intended policy before final acceptance. Tool errors should identify their repair path without requiring a full plan dump.

A normal source-rule check does not require the application's dependencies, but it also does not verify that application. Toolchain prerequisites and application test execution belong to the external development workflow. Missing `pkg-config`, Node, or a default Rust toolchain is not a reason to relabel detector tests as application tests.

### 12.4 Approval authority

Ordinary agent review is allowed where project policy permits it. High-risk exceptions may require an independent agent/person or a protected review process. WT's local record labels, rationale, hashes, and lock files are not authenticated approval identities.

Protect the checker binary/semantic versions, final invocation, loaded rules, rule Markdown, scope/ignore settings, coverage expectations, modes, decisions, and relevant history through a trusted outer boundary. An actor who controls all of these can weaken policy. A signed/hosted approval service is not a requirement of this local CLI.

A documented protected local-policy invocation is:

```bash
wt capabilities --format json
wt fmt --check
wt check --no-global --no-host-ignores --no-cache --format json
```

This is not automatically an effective security boundary: CI/review must protect these inputs, and deliberate global-rule expectations must be reconciled with local-only selection. No command may turn an incomplete result into a passing build because accepted findings exist.

## 13. Implementation boundaries

Retain the small Rust workspace split into `wt-cli`, `wt-core`, and `wt-runtime`. Module names are implementation suggestions, not reasons to rewrite tested code.

| Boundary | Responsibility |
|---|---|
| CLI | Argument validation, version/capability discovery, command dispatch, compact/full rendering. |
| Package/authoring | Strict schemas, Markdown/source/fixture materialization, formatting, atomic new/update/preview, expected digests. |
| Selection/policy | Root resolution, Git-aware eligibility, configuration composition, explicit coverage expectations. |
| Runtime | WRL whitelist and typed host calls, guarded query extraction, native matching/helpers, spans, logical budgets. |
| Query service/workers | Snapshot ownership, interned query identities, shared immutable results, isolated residual execution, physical limits. |
| Review layer | Finding/evidence identities, record loading/history, current validity, atomic decision writes, non-authoritative inspection. |
| Caches | Disposable compiled/fixture/raw-result storage; never durable approvals. |
| Conformance | Language, selection, formatting, schemas, identity, lifecycle, optimizer/cache equivalence, performance accounting. |

Do not merge rule source programs to obtain sharing. Do not let the review layer rewrite the runtime's raw findings. Keep raw detection and post-processing independently testable. Dependencies and semantic versions are pinned per release; an adapter cannot silently enable new arbitrary Rhai syntax.

No full replacement compiler, general syntax engine, standalone approval database, or agent orchestration framework is required by this specification. Existing reliable matching infrastructure can be used behind the documented capabilities; future external-checker integrations need explicit coordinate/evidence/failure mappings rather than arbitrary subprocess access from WRL.

## 14. Performance and portability requirements

The product should make repeated checking inexpensive enough for coding-agent feedback loops. This specification sets architectural and testable work-count requirements, not unmeasured wall-clock claims.

### 14.1 Required workload matrix

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

### 14.2 Deterministic work-count acceptance tests

On a cold, one-file, two-rule sharing fixture with at least one region matching the common regex:

- Normal optimized execution reads one source snapshot and performs one canonical exact enumeration when both consumers demand it.
- Both logical consumers retain their own findings and mode behavior.
- The reference path performs separate exact enumerations and produces the same findings.
- Adding more rules with the same resolved query does not multiply that query's physical evaluations within the transaction, although predicates and output can increase.

When rule B is added to a valid warm ruleset, A's unchanged rule/file raw result remains reusable. When only B is edited, A is not invalidated merely because the combined plan hash changes. File deletion invalidates applicable repository-reduction manifests.

Budgets and worker caches must be instrumented. No test should “pass faster” because files, matches, parse errors, or failed rules were silently skipped.

### 14.3 Portability

Support Linux, macOS, and Windows without Python or Node for ordinary WT execution. The package validation script supplied alongside this specification is a document/example QA tool, not a runtime dependency of WT.

Cover spaces, non-ASCII filenames, Unicode columns, CRLF, BOM, Windows separators, Git worktrees, and invocation from repository subdirectories. Keep parser/compiler dependencies behind WT interfaces and pin them through the release lockfile.

**Naming contract:** Retain both `wt` and `watchtower` executable names. Installation must not replace a pre-existing Windows command alias. The alternate name remains available for disambiguation.

## 15. Worked examples

[`examples/`](../examples/) contains complete, runnable rule packages that double as regression fixtures for the docs test:

- [`orm-query-in-loop/`](../examples/orm-query-in-loop/) — an `ast.v1` Python rule flagging a Django-style `.objects.<method>(...)` call found inside a `for` loop body (a possible N+1 query), including a fixture where the same call sits outside the loop and one where the matching text only occurs in a string or comment.
- [`raw-sql-in-loop/`](../examples/raw-sql-in-loop/) — combines `ast.v1` (to find each loop and the byte span of its body) with `text.v1`'s `rx::find_in` (to search only inside that span for a raw SQL string), demonstrating the two capabilities used together.
- [`template-number-decimal-step/`](../examples/template-number-decimal-step/) — an `ast.v1` TSX rule enforcing that a template's numeric input control does not impose an unintended integer step restriction on a parameter that allows decimals, with positive, negative, review, and analysis-gap fixtures.
- [`match-arm-wrong-handler/`](../examples/match-arm-wrong-handler/) — an `ast.v1` Rust rule using `file.ast_match_context(...)` to match a `match` arm (a node kind that cannot stand alone as a bare pattern), flagging an `Action::Copy` arm whose body never calls `self.copy_document(...)`.
- [`blocking-call-outside-spawn/`](../examples/blocking-call-outside-spawn/) — an `ast.v1` Rust rule using `span::contains` to flag a `completion::suggest(...)` call unless some `thread::spawn(...)` call's span encloses it, with fixtures covering a top-level call, a call inside the spawn closure, one inside a nested block within it, and a string/comment negative.
- [`review-legacy-endpoint.json`](../examples/review-legacy-endpoint.json) with [`second-encounter.scenarios.json`](../examples/second-encounter.scenarios.json) — a minimal `text.v1` review trigger used to exercise the occurrence-review lifecycle end to end: an unchanged raw-positive occurrence keeps its current decision, a copy is independently actionable, and a watched-dependency or owner-file edit reopens the original acceptance.

Each package's `rule.md` documents its own intended constraint, detection strategy, and known limitations; this specification does not duplicate that per-rule reasoning.
