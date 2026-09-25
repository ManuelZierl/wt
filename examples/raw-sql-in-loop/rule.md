# Raw SQL string inside a for loop

## Description

Combines both structural matching capabilities: `ast.v1` finds every `for` loop and gives the
exact byte span of its body, and `text.v1`'s `rx::find_in` then regex-searches only inside that
span for a raw `SELECT ... FROM` string. A query text found anywhere else in the file (outside
any loop body) is not reported.

## Rationale

A raw SQL query built and executed on every iteration of a loop has the same per-item query
cost problem as `orm-query-in-loop`, but as free-form SQL text a purely structural pattern
cannot recognize it; a regex is the right tool for that part. `ast.v1` supplies what regex
cannot: a precise, syntax-aware boundary for "inside this loop" to search within, so the regex
never has to approximate that boundary with line counting or indentation guessing.

## Limitations

- Only looks at direct `for ... in ...:` loops, and only their immediate body span (a query
  built inside a nested `if` or a call from the loop body is still inside that span and is
  found; one reached only through a helper function defined elsewhere is not).
- `raw_sql`'s regex is deliberately narrow (`SELECT ... FROM`) so it does not fire on unrelated
  text; broaden it if your SQL style differs, but a broader pattern also risks matching the
  same text inside a string or comment that only *mentions* a query.
