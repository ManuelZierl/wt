#!/bin/sh
# Documentation conformance check: runs every documented command shown in
# README.md and skills/wt/SKILL.md against the built binary, checks the
# reference submissions install, checks relative Markdown links resolve, and
# checks skills/wt/SKILL.md stays under its size target. Runs offline.
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"

cargo build --release --locked

exec python3 "$root/tests/docs_check.py"
