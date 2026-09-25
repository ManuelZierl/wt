# ORM query call inside a for loop

## Description

Flags an ORM call of the shape `<expr>.objects.<method>(...)` found anywhere in the body of a
`for` loop, a common shape for a Django-style N+1 query: the query runs once per iteration
instead of once for the whole collection.

## Rationale

A query issued inside a loop scales with the size of whatever is being iterated, which is
usually the exact thing the loop is iterating and therefore easy to miss in review. This is a
`review` finding, not an automatic violation: some loop-body queries are already bounded,
cached, or intentionally per-item, so a human should confirm before treating it as a bug.

## Limitations

- Matches the `.objects.<method>(...)` call shape structurally (via `ast.v1`), so it is
  insensitive to formatting and does not fire on the same text inside a string or a comment,
  but it also does not know whether `.objects` is really a Django manager: a coincidentally
  named attribute would also match.
- Only looks at direct `for ... in ...:` loops; a query reached through a helper function
  called from the loop body is not seen.
- Does not distinguish a query outside the loop (not flagged) from one inside it (flagged);
  that scoping is exactly what `m.node("BODY").ast_match(...)` (searching only within the
  captured loop body) is for.
