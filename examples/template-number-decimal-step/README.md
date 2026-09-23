# Decimal Template Rule

This is the readable on-disk form of `template-number-decimal-step.json`. Run the commands below from the repository root; the current CLI validates submissions and discovers installed packages, but does not import an arbitrary package directory in place.

```bash
wt validate --file examples/template-number-decimal-step.json --format json
wt new --stdin --root /tmp/wt-demo --global-dir /tmp/wt-global \
  < examples/template-number-decimal-step.json
wt test local/template-number-decimal-step --root /tmp/wt-demo \
  --global-dir /tmp/wt-global --no-global --format json
```

The package uses `check.wt` as its sole executable source and references `tests.json`. It declares `text.v1` and `jsx.v1`; `regex.v1` remains accepted by the schemas for legacy packages. `fixtures/invalid-tsx.tsx` is intentionally outside the ordinary passing suite because an invalid demanded JSX parse is an analysis error, not an expected diagnostic result.
