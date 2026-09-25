# Second-encounter experiment

The executable reference scenario is in
`examples/second-encounter.scenarios.json`; its source submission is
`examples/review-legacy-endpoint.json`. The CLI conformance test obtains
finding/evidence IDs from the running binary and checks unchanged acceptance,
copy, watched-file edit, owner edit and missing-occurrence behavior. This is a
deterministic state-machine test, **not** evidence of product value in later work.

For an actual evaluation, record the installed `wt capabilities --format json`
and installed guide digests, source revision and checksums, rule package and
fixtures, and exact commands/output. Create equivalent branches for a normal
agent with existing tests/lint/guidance and for that setup with WT enabled.
Do not give follow-up agents the original authoring conversation or held-out
outcome labels. Keep model/tool budgets and working context comparable.

Predeclare a team's useful cost/benefit and critical-miss criteria. Include
unchanged context, a new/copy occurrence, a harmless supported source variant,
changed supporting behavior, unrelated owner edits, moved/excluded input, and
correction of an old review. Report useful detections, misses, inappropriate
acceptances, first/repeated-review time, rule/fixture maintenance, setup/version
friction, WT latency and resource use, and final task quality. Keep raw
transcripts and distinguish WT's incremental contribution from defects the
agent found independently. No field-study outcome or threshold is claimed by
the deterministic test.
