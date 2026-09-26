# completion::suggest call is not inside thread::spawn

## Description

Finds every `completion::suggest(...)` call and every `thread::spawn(...)` call with `ast.v1`,
then uses `span::contains` to test each `completion::suggest` call's span against every
`thread::spawn` call's span: a call is reported unless some `thread::spawn` call's span
encloses it. `ast_match` alone can express "found inside a span" (re-searching within a
captured span, as in `raw-sql-in-loop`), but not this negation; `span.contains` is what makes
"NOT inside" expressible as a plain boolean check.

## Rationale

A `completion::suggest` call left outside `thread::spawn` runs on the caller's thread and can
block it; the same call inside `thread::spawn`'s closure (at any nesting depth within it, since
the closure's body is still part of the `thread::spawn` call's own span) is fine. Neither a
regex nor a bare `ast_match` can express "call A, except when some call B's span encloses it"
directly; `span.contains` on two independently captured spans is the general primitive that
lets a rule state the exception instead of approximating it.

## Limitations

- Only recognizes the literal call path `thread::spawn`; an alias, a re-exported wrapper, or a
  different executor (a thread pool, an async runtime) is not recognized as "off the caller's
  thread".
- A `completion::suggest` call reached only through a helper function defined elsewhere, rather
  than appearing textually inside the `thread::spawn` call's span, is still reported.
