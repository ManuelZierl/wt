#!/usr/bin/env python3
"""Documentation conformance check.

- Extracts every ```bash fenced block from README.md and skills/wt/SKILL.md
  and runs it, checking that it succeeds (exit 0) or exits with the code
  documented by an immediately preceding ``<!-- docs-test: ... -->`` marker
  (``exit=N`` for a documented nonzero exit, or ``skip`` for a command that
  is intentionally not runnable offline, such as the install curl line).
  When a ```json block directly follows a command block, the command's
  stdout must parse to exactly that JSON (one or more values, in order).
- Confirms both files under skills/wt/references/ validate and install with
  ``wt new``.
- Confirms every relative link in a tracked Markdown file resolves to a real
  file.
- Confirms skills/wt/SKILL.md stays under the 4 KiB size target.

Runs entirely offline against the binaries in target/release/.
"""
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SKILL_SIZE_LIMIT = 4096

FENCE_RE = re.compile(r"```(?:bash|sh)\n(.*?)```", re.S)
MARKER_RE = re.compile(r"<!--\s*docs-test:\s*(.*?)\s*-->")
OUTPUT_RE = re.compile(r"\s*```json\n(.*?)```", re.S)


def extract_blocks(path):
    """Return [(command_text, marker_or_None, expected_json_or_None), ...]."""
    text = open(path, encoding="utf-8").read()
    blocks = []
    for m in FENCE_RE.finditer(text):
        preceding = text[: m.start()]
        marker = None
        markers = list(MARKER_RE.finditer(preceding))
        if markers:
            last = markers[-1]
            if preceding[last.end():].strip() == "":
                marker = last.group(1).strip()
        output = OUTPUT_RE.match(text, m.end())
        blocks.append((m.group(1).rstrip("\n"), marker, output.group(1) if output else None))
    return blocks


def expected_exit(marker):
    if marker and marker.startswith("exit="):
        return int(marker.split("=", 1)[1])
    return 0


def run_block(command_text, cwd, env):
    proc = subprocess.run(
        ["bash", "-c", command_text],
        cwd=cwd,
        env=env,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=600,
    )
    out = proc.stdout.decode("utf-8", "replace")
    err = proc.stderr.decode("utf-8", "replace")
    return proc.returncode, out, err


def json_values(text):
    decoder = json.JSONDecoder()
    values, pos = [], 0
    while True:
        while pos < len(text) and text[pos].isspace():
            pos += 1
        if pos == len(text):
            return values
        value, pos = decoder.raw_decode(text, pos)
        values.append(value)


def check_doc_commands(doc_label, doc_path, cwd_for, env, failures):
    for i, (block, marker, expected) in enumerate(extract_blocks(doc_path), start=1):
        kind = (marker.split()[0] if marker else None)
        if kind == "skip":
            print(f"[SKIP] {doc_label} block {i} (marked not runnable: {marker})")
            continue
        want = expected_exit(marker)
        cwd = cwd_for(block)
        rc, out, err = run_block(block, cwd, env)
        if rc != want:
            print(f"[FAIL] {doc_label} block {i} (expected exit {want}, got {rc})")
            print(textwrap_indent(out + err))
            failures.append(f"{doc_label} block {i}")
            continue
        if expected is not None:
            try:
                same = json_values(out) == json_values(expected)
            except ValueError:
                same = False
            if not same:
                print(f"[FAIL] {doc_label} block {i} output differs from the documented JSON")
                print(textwrap_indent(out + err))
                failures.append(f"{doc_label} block {i} output")
                continue
            print(f"[OK]   {doc_label} block {i} (exit {rc}, output matches)")
            continue
        print(f"[OK]   {doc_label} block {i} (exit {rc})")


def textwrap_indent(text, prefix="    "):
    return "\n".join(prefix + line for line in text.splitlines())


def check_reference_submissions(env, failures):
    refs_dir = os.path.join(ROOT, "skills", "wt", "references")
    for name in ("minimal-submission.json", "minimal-ast-submission.json"):
        ref = os.path.join(refs_dir, name)
        with tempfile.TemporaryDirectory(prefix="wt-docs-ref-") as tmp:
            cmd = f'wt validate --file "{ref}" --format json'
            rc, out, err = run_block(cmd, tmp, env)
            if rc != 0:
                print(f"[FAIL] reference {name}: wt validate exited {rc}")
                print(textwrap_indent(out + err))
                failures.append(f"reference {name} validate")
                continue
            cmd = f'wt new --stdin --format json < "{ref}"'
            rc, out, err = run_block(cmd, tmp, env)
            if rc != 0:
                print(f"[FAIL] reference {name}: wt new exited {rc}")
                print(textwrap_indent(out + err))
                failures.append(f"reference {name} new")
                continue
            print(f"[OK]   reference {name} validates and installs")


def check_skill_size(failures):
    skill_md = os.path.join(ROOT, "skills", "wt", "SKILL.md")
    size = os.path.getsize(skill_md)
    if size >= SKILL_SIZE_LIMIT:
        print(f"[FAIL] skills/wt/SKILL.md is {size} bytes, must stay under {SKILL_SIZE_LIMIT}")
        failures.append("SKILL.md size")
    else:
        print(f"[OK]   skills/wt/SKILL.md is {size} bytes (< {SKILL_SIZE_LIMIT})")


LINK_RE = re.compile(r"\]\(([^)\s]+)(?:\s+\"[^\"]*\")?\)")


def check_links(failures):
    tracked = subprocess.run(
        ["git", "ls-files", "*.md"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=True,
    ).stdout.splitlines()
    checked = 0
    for rel in tracked:
        full = os.path.join(ROOT, rel)
        if not os.path.exists(full):
            # Deleted but not yet staged/committed; `git ls-files` still
            # lists it from the index.
            continue
        text = open(full, encoding="utf-8").read()
        for link in LINK_RE.findall(text):
            if link.startswith(("http://", "https://", "mailto:", "#")):
                continue
            target = link.split("#", 1)[0]
            if not target:
                continue
            target_path = os.path.normpath(os.path.join(os.path.dirname(full), target))
            checked += 1
            if not os.path.exists(target_path):
                print(f"[FAIL] {rel}: broken relative link '{link}'")
                failures.append(f"{rel} -> {link}")
    print(f"[OK]   checked {checked} relative links across {len(tracked)} tracked Markdown files")


def main():
    failures = []

    bin_dir = os.path.join(ROOT, "target", "release")
    if not os.path.exists(os.path.join(bin_dir, "wt")):
        print("target/release/wt is missing; run `cargo build --release` first.", file=sys.stderr)
        return 2

    env = os.environ.copy()
    env["PATH"] = bin_dir + os.pathsep + env.get("PATH", "")
    cargo_install_root = tempfile.mkdtemp(prefix="wt-docs-cargo-install-")
    env["CARGO_INSTALL_ROOT"] = cargo_install_root

    # README.md: everything except the `cargo install` line runs inside an
    # isolated, non-Git temporary repository seeded with a copy of examples/,
    # so root/global-dir discovery matches what the README shows (no flags)
    # without touching the real checkout or its `.wt/` state.
    readme_workdir = tempfile.mkdtemp(prefix="wt-docs-readme-")
    shutil.copytree(os.path.join(ROOT, "examples"), os.path.join(readme_workdir, "examples"))

    def readme_cwd(block):
        return ROOT if "cargo install" in block else readme_workdir

    check_doc_commands("README.md", os.path.join(ROOT, "README.md"), readme_cwd, env, failures)
    shutil.rmtree(readme_workdir, ignore_errors=True)
    shutil.rmtree(cargo_install_root, ignore_errors=True)

    # SKILL.md: run from a copy of skills/wt/ placed outside the Git
    # worktree, so `references/...` resolves exactly as an installed skill
    # would see it, and root discovery does not find this repository's .git.
    skill_workdir_parent = tempfile.mkdtemp(prefix="wt-docs-skill-")
    skill_workdir = os.path.join(skill_workdir_parent, "wt")
    shutil.copytree(os.path.join(ROOT, "skills", "wt"), skill_workdir)
    check_doc_commands(
        "skills/wt/SKILL.md",
        os.path.join(ROOT, "skills", "wt", "SKILL.md"),
        lambda block: skill_workdir,
        env,
        failures,
    )
    shutil.rmtree(skill_workdir_parent, ignore_errors=True)

    check_reference_submissions(env, failures)
    check_skill_size(failures)
    check_links(failures)

    if failures:
        print(f"\n{len(failures)} docs check(s) failed:")
        for item in failures:
            print(f" - {item}")
        return 1
    print("\nAll docs checks passed.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
