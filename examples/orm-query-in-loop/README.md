# ORM Query In Loop

This is the readable on-disk form of `orm-query-in-loop.json`. Run the commands below from the repository root.

```bash
wt validate --file examples/orm-query-in-loop.json --format json
wt new --stdin --root /tmp/wt-demo --global-dir /tmp/wt-global \
  < examples/orm-query-in-loop.json
wt test local/orm-query-in-loop --root /tmp/wt-demo \
  --global-dir /tmp/wt-global --no-global --format json
```

The package uses `check.wt` as its sole executable source and references `tests.json`. It declares `ast.v1` and matches Python structurally: it finds every `for` loop, then searches inside the loop body's captured span for an ORM-style `.objects.<method>(...)` call.
