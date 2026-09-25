# Decimal Fixture Vectors

The cases in `../tests.json` are the ordinary positive, accepted, and review vectors for the decimal-step rule (see `../rule.md`). `invalid-tsx.tsx` is retained as a separate analysis-error vector: the fixture contract represents expected diagnostics, not expected runtime failures, so it is intentionally not part of the passing suite. A demanded `ast.v1` parse failure must be reported as incomplete analysis, never as a no-finding result.
