# Raw SQL In Loop

This is the readable on-disk form of `raw-sql-in-loop.json`. Run the commands below from the repository root.

```bash
wt validate --file examples/raw-sql-in-loop.json --format json
wt new --stdin --root /tmp/wt-demo --global-dir /tmp/wt-global \
  < examples/raw-sql-in-loop.json
wt test local/raw-sql-in-loop --root /tmp/wt-demo \
  --global-dir /tmp/wt-global --no-global --format json
```

The package uses `check.wt` as its sole executable source and references `tests.json`. It declares both `ast.v1` and `text.v1`: `ast.v1` locates each `for` loop's body span, and `text.v1`'s `rx::find_in` regex-searches only within that span for a raw SQL string.
