# PRD-TRADING-DATA-LAKE-AHDM-001 — real decomposed task set

The 17 `TASK-TDL-*.md` files here are a user's own PRD decomposition, copied
verbatim. They are the reference for what a decomposed-PRD task file looks like:
a fenced ```yaml block carrying `task_id`, `prd`, `domain`, `title`,
`workstream`, `complexity`, `status`, `depends_on`, `blocks`, `source_sections`,
`required_env_keys`, `required_tools` and `deliverable_contracts`, followed by
the markdown sections (Purpose, Scope, Files Expected/Forbidden to Change,
Acceptance Criteria, Focused Tests, Adversarial Review Notes, an optional
`PRIOR-RUN-FINDINGS` block, Required Task Checklist, Global Constraints).

They exist because `parse_task_file` was previously only ever exercised against
task files this repository wrote itself — a closed loop in which a field the
parser ignored was also a field no test file declared. `blocks`, `complexity`,
`status` and the two file-scope sections were all in that category.

## `expected-parse.json`

The per-field expectation table the round-trip test in
`src/command/workflow_live_task_universe_parsing.rs` asserts against.

It is deliberately **not** produced by the Rust parser under test — that would
make it a snapshot of the bug rather than a check on it. Each record is read out
of its `.md` file by an independent reader: the YAML block by PyYAML, the
markdown sections by a short extractor applying the same documented rule
(list items under a case-insensitively matched heading, deduplicated and
sorted). Regenerate it only from the `.md` files, never from parser output.

Do not edit the `TASK-TDL-*.md` files. They are evidence, and rewriting them to
make a test pass removes the only thing they are here to prove.
