# WRL1 authoring reference

Use `code.language: "wt-rule-1"` and `code.capabilities: ["text.v1"]` (add
`"ast.v1"` for structural matching, below).
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

## Structural matching (`ast.v1`)

Add `ast.v1` for structural, syntax-aware matching over python, javascript,
typescript, tsx, or rust (see `wt capabilities` for the languages and grammar
versions this build actually has). `language` and `pattern` are string
literals, compiled once at rule-compile time; an invalid pattern or an
unsupported/disabled language is a validation error, not a runtime one.

| API | Result |
|---|---|
| `file.ast_match("language", "pattern")` | Finite sequence of structural matches in the whole file |
| `matched.ast_match("language", "pattern")` | Matches nested inside a previous match's span ("X inside Y") |
| `file.ast_match_context("language", "context", "selector")` | Same, but for a node kind that cannot stand alone as a bare pattern |
| `matched.node("NAME")` | The `$NAME`/`$$$NAME` metavariable capture, or unit if absent |
| `matched.text`, `matched.span` | Same as any other match |
| `span::contains(outer, inner)` | Boolean: `inner`'s byte range lies within `outer`'s, same file |

```wt
for m in file.ast_match("python", "for $X in $ITER:\n    $$$BODY") {
    for q in m.node("BODY").ast_match("python", "$QS.objects.$METHOD($$$)") {
        emit(q.span, "possible-n-plus-one");
    }
}
```

A file with any tree-sitter error/missing node is an analysis gap for every
`ast.v1` call that touches it (incomplete, exit 2), never a silent no-match;
parsing only happens for files an unguarded `ast_match` call actually reaches.
`text.v1` and `ast.v1` combine freely, e.g. `rx::find_in(m.node("BODY").span, "pattern")`
to regex-search only inside a captured structural span.

A bare pattern must parse as a complete node by itself, which rules out node
kinds that only make sense nested inside something else: a Rust `match` arm,
a struct field, an attribute, a function parameter. `ast_match_context` covers
these with ast-grep's contextual-pattern form: `context` is a standalone
snippet that must parse without error, and `selector` is the tree-sitter node
kind inside it to actually match against (an unknown kind, or one that
matches nothing in `context`, is a compile-time error, same as a bad plain
pattern). Metavariable captures work the same way.

```wt
for m in file.ast_match_context(
    "rust",
    "match a { Action::Copy => $BODY }",
    "match_arm"
) {
    emit(m.node("BODY").span, "copy-arm-body");
}
```

A child the context leaves out is unconstrained, not required absent: the
context above also matches a guarded arm like `Action::Copy if cond =>
$BODY`, since it says nothing about a guard's presence. To exclude guarded
arms, also match the narrower context and drop any span the two share:

```wt
for m in file.ast_match_context("rust", "match a { Action::Copy => $BODY }", "match_arm") {
    let guarded = false;
    for g in file.ast_match_context("rust", "match a { Action::Copy if $COND => $BODY }", "match_arm") {
        if span::contains(g.span, m.span) && span::contains(m.span, g.span) {
            guarded = true;
        }
    }
    if !guarded {
        emit(m.node("BODY").span, "copy-arm-body");
    }
}
```

`file.path` (and `path::matches`, which reads it) needs `text.v1` or
`path.v1` specifically — `ast.v1` does not cover it. An `ast.v1`-only rule
that reads `file.path` fails WT104; add `text.v1` (or `path.v1`) if it needs
the path as well as the structure.

`ast_match` alone can express "found inside a span" by re-searching within a
captured span, but not "found, unless some other span encloses it" (e.g. a
call that must run inside `thread::spawn`). `span::contains` on two
independently captured spans covers that case directly:

```wt
for suggest_call in file.ast_match("rust", "completion::suggest($$$)") {
    let inside_spawn = false;
    for spawn_call in file.ast_match("rust", "thread::spawn($$$)") {
        if span::contains(spawn_call.span, suggest_call.span) {
            inside_spawn = true;
        }
    }
    if !inside_spawn {
        emit(suggest_call.span, "blocking-call-outside-spawn");
    }
}
```

## Structured data (`toml.v1`)

Add `toml.v1` for structured access to a TOML document (a Cargo manifest or
other TOML config) instead of a line-by-line regex over its text (see
`wt capabilities` for the installed parser's identity). `toml.v1` alone does
not cover `file.path`, `file.text`, or `file.span`; add `text.v1` (or
`path.v1`, for `file.path` only) if a `toml.v1`-only rule also needs one of
those.

| API | Result |
|---|---|
| `file.toml()` | The document root value; a file that does not parse cleanly as TOML is an analysis gap |
| `value.get("key")` | Direct child of a table value named `key`, or unit; not recursive across dotted paths |
| `value.entries()` | Finite sequence of a table's entries (`.key`, `.key_span`, `.value`), source order |
| `value.items()` | Finite sequence of an array's elements, source order |
| `value.kind` | `"table"`, `"array"`, `"string"`, `"integer"`, `"float"`, or `"boolean"` |
| `value.text`, `value.span` | Decoded content (strings) or raw source text (everything else); source span |

```wt
let dependencies = file.toml().get("dependencies");
if dependencies != () {
    for entry in dependencies.entries() {
        let renamed = entry.value.get("package");
        if renamed != () {
            if renamed.text != "shared" {
                emit(renamed.span, "host-dependency");
            }
        } else if entry.key != "shared" {
            emit(entry.key_span, "host-dependency");
        }
    }
}
```

A renamed dependency (`alias = { package = "forbidden-crate", ... }`) is
resolved through its `package` sub-key, not its own key; a dotted-key
assignment (`a.b = 1`) and the equivalent inline table parse to the same
table-of-tables shape, so a rule does not need to special-case either form.
