# Decimal template inputs use decimal-compatible steps

## Description

Checks the verified template-number rendering context for an unintended integer step restriction.

## Rationale

HTML number inputs default to a step of one; decimal parameters need an explicit decimal-compatible step in this rendering context.

## Limitations

- Scoped to the two verified template-number rendering modules represented by the retained examples.
- Does not resolve aliases, wrapper components, helper calls, shadowed identifiers, or business types.
- Unrecognized step expressions remain review findings; missing or integer-only steps are the violation.
- Uses `ast.v1` structural matching (TSX), so it is naturally insensitive to whitespace, attribute order, and
  extra attributes, and ignores JSX-looking text inside strings and comments. It does not special-case spread
  attributes (`{...props}`) or a duplicated `type` attribute; either can make the match miss or pick either
  occurrence, unlike the earlier `jsx.v1` version of this rule.
