# WRL1 authoring reference

Use `code.language: "wt-rule-1"` and `code.capabilities: ["text.v1"]`.
The detector is a top-level body with a read-only `file` binding. Repository
rules declare `execution: "repository"` and receive `repo` instead.

```wt
for matched in rx::find_all(file, "retired") {
    if !matched.text.contains("documented_exception") {
        emit(matched.span, "retired-call");
    }
}
```

Declare `retired` in manifest `patterns`; declare `retired-call` in `diagnostics`.
Pattern and diagnostic IDs are string literals or literal `const` bindings.
Patterns use Rust regex semantics: named captures and UTF-8 matching, no
look-around or backreferences. Use independent patterns for independent rules;
WT shares equal demanded queries after resolving local aliases.

## Language

- `let`, literal `const`, scalar reassignment, `if`/`else`, finite `for` loops.
- `break;`, `continue;`, `return;` without a value.
- Boolean, signed integer, string, and unit `()` values.
- Arithmetic, comparisons, string concatenation, short-circuit `&&` and `||`.
- No functions, arrays/maps, indexing, arbitrary iteration, `while`, imports,
  dynamic calls, reflection, or host-object mutation.
- Budget exhaustion and missing-value operations are analysis failures.

## Core APIs

| API | Result |
|---|---|
| `file.path`, `file.text`, `file.span` | Relative path, original source text, whole-source span |
| `text::lines(file)` | Finite sequence of `line.text` / `line.span` |
| `text::contains(text, needle)` or `text.contains(needle)` | Exact substring Boolean |
| `text::starts_with(text, prefix)`, `text::ends_with(text, suffix)` | Exact prefix/suffix Boolean |
| `text::trim(text)`, `text::replace(text, from, to)` | Derived string without source span |
| `text::split(text, separator)` | Finite string sequence; separator is nonempty and static |
| `text::len_bytes(text)` | UTF-8 byte count |
| `path::matches(file.path, "src/**/*.txt")` | Case-sensitive root-relative glob match |
| `rx::is_match(file, "pattern")` | Presence Boolean |
| `rx::find_all(file, "pattern")` | Complete ordered sequence of source matches |
| `rx::find_in(span, "pattern")` | Matches in an independent source slice, with absolute offsets |
| `rx::capture_text("pattern", text)` | First captured match or unit, without a source span |
| `matched.text`, `matched.span` | Original matched text/span; span only for source searches |
| `matched.group_text("name")` | Named capture text or unit for a missing optional capture |
| `sequence.len`, `sequence.is_empty()` | Size/emptiness; each loop has its own cursor |
| `emit(span, "diagnostic")` | A rule-owned diagnostic at an authorized original-source span |
| `repo.files()` | Sorted authorized snapshot files, repository rules only |

Guard optional values before accessing properties or calling methods:

```wt
let matched = rx::capture_text("conditional", file.text);
if matched != () {
    let parameter = matched.group_text("parameter");
    if parameter != () && parameter == "known_parameter" {
        emit(file.span, "review-conditional");
    }
}
```

## Scope, spans, and syntax helpers

Globs are root-relative with `/`. `*` stays within a component; `**` spans
components. Exclusions have reasons and win over includes. `require_files`
controls applicability. Keep fixture virtual paths within the same scope.

Spans are opaque source handles. Never invent offsets or emit on a derived
string. Byte offsets are zero-based, end-exclusive; human columns count Unicode
scalar values. `find_all` completes or fails even if its consumer later breaks.

For real TSX elements, add `jsx.v1` and use `jsx::inputs(file)`. Inputs expose
`span`, `has_spread`, `duplicate("attribute")`, and `attr("attribute")`.
Attributes expose `kind`, `canonical`, and optional `static_string`. Spreads,
duplicates, and computed expressions may require a `review` diagnostic.
This helper does not resolve application types or business meaning. A demanded
parse failure is an error, not an empty input list.
