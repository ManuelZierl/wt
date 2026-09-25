# Install the WT agent skill

The loadable skill is [`skills/wt/SKILL.md`](../skills/wt/SKILL.md). Its directory also contains two self-contained sample submissions (`text.v1` and `ast.v1`) and a WRL1 reference. Copy the **whole `wt` directory**, so those relative references remain available.

The skill works in repositories that do not contain WT's sources. It uses the installed `wt` or `watchtower` executable, its help, and its schemas. WT requires no model provider, daemon, or network service.

## OpenCode

From a WT source checkout, install into the target project's skill directory:

```sh
mkdir -p /path/to/project/.opencode/skills
cp -R skills/wt /path/to/project/.opencode/skills/wt
```

Alternatively, register the source checkout's skills directory in an existing OpenCode configuration. Merge this field into the configuration rather than replacing it:

```json
{
  "skills": {
    "paths": ["/absolute/path/to/wt/skills"]
  }
}
```

Quit and restart OpenCode after installing or registering the skill. The new session can load the skill named `wt`. Installing the CLI does not automatically install or register the skill.

## Other Agent Skills-compatible tools

Copy `skills/wt/` to the skill location supported by the tool. For a tool that discovers project skills under `.agents/skills`:

```sh
mkdir -p /path/to/project/.agents/skills
cp -R skills/wt /path/to/project/.agents/skills/wt
```

Agents with file-reading support can also read `skills/wt/SKILL.md` directly. Discovery directories vary by harness; use its documented installation location.

## Included workflow

The skill covers inspection, narrow documented detectors (`text.v1` and structural
`ast.v1`), strict JSON submission, retained regression examples, validation, creation,
fixtures, plan inspection, full and partial checks, digest-protected updates,
advisory/enforced modes, and trusted CI boundaries. Its sample submissions are
exercised by the CLI acceptance tests and by `tests/docs.sh`.

When enforcing policy, protect the checker version, `.wt/` policy,
ignore settings, and final CI invocation through a trusted review boundary.
A completed WT scan means the selected detectors finished on their eligible
inputs; it does not prove application correctness.
