# Decimal template inputs use decimal-compatible steps

## Description

Checks the verified template-number rendering context for an unintended integer step restriction.

## Rationale

HTML number inputs default to a step of one; decimal parameters need an explicit decimal-compatible step in this rendering context.

## Limitations

- Scoped to the two verified template-number rendering modules represented by the retained examples.
- Does not resolve aliases, wrapper components, helper calls, shadowed identifiers, or business types.
- Spreads, duplicate relevant attributes, and unrecognized step expressions remain review findings.
