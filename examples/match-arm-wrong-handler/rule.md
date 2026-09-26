# Action::Copy match arm does not call copy_document

## Description

Uses `ast.v1`'s contextual pattern form to match a Rust `match` arm: `Action::Copy => $BODY`
is not a complete node on its own (a bare arm cannot stand alone as an ast-grep pattern), so
`file.ast_match_context("rust", "match a { Action::Copy => $BODY }", "match_arm")` wraps it in
a minimal `match` and selects the `match_arm` node kind out of it. For every `Action::Copy` arm
found this way, the captured `$BODY` is searched (`body.ast_match(...)`) for a nested call to
`self.copy_document(...)`; an arm whose body never calls it is reported.

## Rationale

A dispatcher `match` over an action enum is easy to miswire during a refactor: copy-paste an
adjacent arm and forget to change which handler it calls. A plain-text or regex check cannot
reliably identify "the arm for this specific variant" without also matching the variant name
inside unrelated text; a structural match on the real `match_arm` node does not have that
problem, and the same shape (context + selector) generalizes to any Rust node that only makes
sense nested inside something else: a struct field, an attribute, a function parameter.

## Limitations

- Only recognizes a direct `self.copy_document(...)` call anywhere inside the arm's body; a
  call reached through a local variable or a helper method is not recognized as satisfying it.
- Matches by variant name (`Action::Copy`) textually inside the contextual pattern, so it does
  not verify that `Action` here is actually the same enum declared elsewhere in the crate.
