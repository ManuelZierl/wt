# Decimal Fixture Vectors

The cases in `../tests.json` are the ordinary positive, accepted, and review vectors from `spec.md` section 12.4. `invalid-tsx.tsx` is retained as a separate analysis-error vector: the fixture contract represents expected diagnostics, not expected runtime failures, so it is intentionally not part of the passing suite. A demanded JSX parse failure must be reported as incomplete analysis, never as a no-finding result.
