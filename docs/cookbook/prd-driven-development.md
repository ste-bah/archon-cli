# PRD-driven development

There are **two** routes from a PRD to running code. Both write under the same
two roots — `prds/` for PRD documents, `tasks/` for decomposed task sets — in
different subfolders, so they coexist without colliding. They are not
interchangeable: each has its own skills, its own templates, and its own
consumer.

## Choosing a pipeline

| | Skills chain | Workflow path |
|---|---|---|
| Skills | `/to-prd` → `/prd-to-spec` → `/spec-to-tasks` → `/archon-code` | `/workflow-prd` → `/workflow-prd-spec` |
| Templates | `ai-agent-prd.md`, `prdtospec.md` | `workflow-prd.md`, `workflow-prdtospec.md` |
| PRD lands at | `prds/<slug>/PRD.md` | `prds/PRD-<NAME>/PRD-<NAME>.md` |
| Tasks land at | `tasks/INDEX.md` + `tasks/phase<N>/task<M>.md` | `tasks/PRD-<NAME>/TASK-<DOMAIN>-<NNN>-<slug>.md` |
| Executed by | the 50-agent `/archon-code` pipeline | the workflow engine, as a generated run |

**Use the workflow path when you want the run to refuse to proceed on a
contradiction.** It gives you:

- a **dependency graph** — `depends_on` and `blocks` are both parsed and
  reconciled into one graph; a cycle, an unresolved reference, a task that
  blocks itself, or a pair declaring both directions is refused by name;
- **single-writer enforcement** — the write coordinator serialises tasks that
  target the same path, and a task opts a path out only by declaring
  `shared_append_target_files`;
- **per-task adversarial review** against declared `## Adversarial Review
  Notes`, with prior-run findings appended back into the task file;
- **requirement traceability** — `archon requirements trace` walks
  `REQ-<AREA>-<NNN>` from the PRD to the tasks claiming them to the commands
  and files that prove them, and `archon workflow lint --tasks` reports
  write conflicts, fake edges, and requirement coverage.

**Use the skills chain when you want the 50-agent pipeline** — test-first
implementation, six dev-flow gates per task, Sherlock review at gates 3 and 6,
phase reviewers. It is the richer execution model; it does not give you the
declared dependency graph or the requirement trace.

Do not mix them. `/spec-to-tasks` and `/archon-code` read `tasks/phase<N>/` and
will not find a workflow task set; the workflow engine walks
`tasks/PRD-<NAME>/` and will not find a phase tree.

---

# Pipeline 1 — the skills chain

The full PRD-to-code pipeline: from a feature description to a running implementation through four composable skills.

## Pipeline overview

```
/to-prd  →  /prd-to-spec  →  /spec-to-tasks  →  /archon-code
```

| Step | Skill | Output |
|------|-------|--------|
| 1. Generate PRD | `/to-prd "feature description"` | `prds/<slug>/PRD.md` |
| 2. Spec from PRD | `/prd-to-spec prds/<slug>/PRD.md` | `tasks/INDEX.md` + per-phase task files |
| 3. Refine tasks | `/spec-to-tasks` | Atomic, dev-flow-ready task files |
| 4. Implement | `/archon-code` | 50-agent pipeline execution |

## Fast path

Use `/compose-pipeline` to chain steps 1-3 in one command:

```
/compose-pipeline "add user authentication with OAuth2"
```

This runs `/to-prd` → `/prd-to-spec` → `/spec-to-tasks` sequentially, then hands off for manual `/archon-code`.

## Step-by-step

### 1. `/to-prd "feature description"`

Generates a Product Requirements Document using the `ai-agent-prd` template. The template covers:
- Problem statement and user stories
- Functional and non-functional requirements
- Architecture and data model
- Success metrics and acceptance criteria

Output lands at `prds/<slug>/PRD.md`. Review and refine before proceeding.

### 2. `/prd-to-spec <path to PRD>`

Converts the PRD into a phased task decomposition:
- `tasks/INDEX.md` — master index of all phases and tasks
- `tasks/phase<N>/task<M>.md` — per-task files with descriptions and dependencies

### 3. `/spec-to-tasks`

Refines the task tree for dev-flow readiness:
- Verifies atomicity (single responsibility, testable, < 1 day)
- Splits coarse tasks, merges trivially small ones
- Ensures every task has acceptance criteria, dependencies, test plan, and files-to-modify
- Updates `tasks/INDEX.md`

### 4. `/archon-code`

Runs the 50-agent implementation pipeline against the refined task tree. Each task gets:
- Test-first implementation
- Sherlock adversarial review (Gate 3 + 6)
- Live smoke test (Gate 5)
- Dev-flow gate enforcement

## End-to-end TUI walkthrough

What the workflow actually looks like inside the TUI, from cold start to merged code. Assumes you've run `archon` and you're at the prompt.

### Step 0 — discuss the feature you want to build

Before invoking any skill, just talk to the agent. The richer your conversation context, the better the PRD `/to-prd` writes.

```
> I want to add OAuth2 token refresh to our API client. Tokens are stored in
> ~/.archon/.credentials.json. We need to lock the file during refresh so
> two concurrent CLI processes don't double-refresh and burn the refresh
> token. We also need a graceful fallback when the refresh endpoint is
> down — fall through to interactive re-login. Implementation should match
> the existing crate layout in crates/archon-llm/.

[archon] explores crates/archon-llm/, summarizes the existing token storage
[archon] asks two clarifying questions about lock granularity and timeout
> Per-process advisory lock. 30-second timeout. If we can't acquire, fail
> the request with a retriable error.
[archon] confirms understanding, summarises the requirements
```

The conversation history is the source material `/to-prd` will use.

### Step 1 — `/to-prd` (or alias `/prd`)

```
> /to-prd
```

What you see in the TUI:

```
[skill: to-prd] reading template ai-agent-prd...
[skill: to-prd] template loaded (8.2 KB), constructing PRD prompt
[agent] thinking through PRD structure...
[agent] writing prds/oauth2-token-refresh/PRD.md
[agent] PRD created at prds/oauth2-token-refresh/PRD.md
```

The skill does NOT write the file directly — it generates a prompt that asks the LLM to write the PRD using its `Write` tool. Review the PRD before going further:

```
> /open prds/oauth2-token-refresh/PRD.md
```

The PRD covers problem statement, user stories, functional + non-functional requirements, architecture sketch, data model, success metrics, acceptance criteria. Refine in-place if needed — the next step reads whatever's on disk.

### Step 2 — `/prd-to-spec <path>`

```
> /prd-to-spec prds/oauth2-token-refresh/PRD.md
```

Visible in the TUI:

```
[skill: prd-to-spec] reading template prdtospec...
[skill: prd-to-spec] reading PRD: prds/oauth2-token-refresh/PRD.md
[agent] decomposing PRD into 4 phases...
[agent] writing tasks/phase1/task1.md
[agent] writing tasks/phase1/task2.md
[agent] writing tasks/phase2/task1.md
[agent] writing tasks/phase2/task2.md
[agent] writing tasks/phase3/task1.md
[agent] writing tasks/phase4/task1.md
[agent] writing tasks/INDEX.md
[agent] summary: 6 tasks across 4 phases
```

The slash requires the positional path. If you forget it:

```
> /prd-to-spec
[skill: prd-to-spec] error: Usage: /prd-to-spec <path/to/PRD.md>
```

Aliases: `/decompose-prd` does the same thing.

### Step 3 — `/spec-to-tasks`

```
> /spec-to-tasks
```

```
[skill: spec-to-tasks] reading SKILL.md guidance...
[skill: spec-to-tasks] discovering tasks under tasks/
[agent] reviewing tasks/phase1/task1.md against atomicity criteria
[agent] task1.md OK (1-day, single deliverable, testable)
[agent] reviewing tasks/phase2/task1.md
[agent] task2.md TOO BIG — splitting into task2a (token-locking) and task2b (refresh-fallback)
[agent] reviewing tasks/phase3/task1.md
[agent] task3.md OK after adding test plan
[agent] updating tasks/INDEX.md to reflect refined tree
[agent] done. 7 tasks across 4 phases (was 6)
```

Each refined task file includes acceptance criteria, test plan, dependencies-by-task-id, and files-to-modify — the `/archon-code` pipeline reads these directly.

### Step 4a — fast path `/compose-pipeline`

If you trust the skills enough to chain them without intermediate review:

```
> /compose-pipeline "Add OAuth2 token refresh with file locking and interactive fallback"
```

Runs steps 1–3 back-to-back. Stops before `/archon-code` so you can still inspect the task tree before committing to a full pipeline run.

### Step 4b — implement with `/archon-code`

```
> /archon-code
```

Picks up the refined `tasks/` tree. Each task triggers a 50-agent run with:
- 6 dev-flow gates per task (tests-written-first → implementation → sherlock review → tests-passing → live smoke → final sherlock)
- Phase reviewers (Phases 1-6) gate progression
- Sherlock adversarial review at Gate 3 and Gate 6 — Sherlock independently re-reads the diff, treats it as guilty until proven innocent

See [god-code-pipeline.md](god-code-pipeline.md) for the full agent breakdown and TUI status commands.

### Task atomicity criteria

`/spec-to-tasks` checks each task against:

- **Single responsibility** — one clear deliverable
- **Testable** — can you write a test that verifies completion?
- **< 1 working day** — if it looks bigger, split it
- **No implicit dependencies** — dependencies must be listed explicitly by task ID

### Inspecting and resuming mid-pipeline

```
> /pipeline status                # current run id + phase + last completed agent
> /pipeline list                  # all sessions, resumeable + completed
> /pipeline verify <session-id>   # checks bundle hashes before trust/resume
> /pipeline inspect <session-id>  # shows manifest, state, and agent records
> /pipeline resume <session-id>   # continues from last completed gate
> /pipeline abort <session-id>    # marks the audited bundle aborted and keeps artifacts
```

If `/archon-code` crashes (rare) or you Ctrl-C deliberately, the resume path is
git-aware and verifier-gated: it refuses to continue if files have changed
under it or if bundle artifacts no longer match their hashes.

---

# Pipeline 2 — the workflow path

```
/workflow-prd  →  /workflow-prd-spec  →  archon workflow run
```

| Step | Skill | Output |
|------|-------|--------|
| 1. Generate PRD | `/workflow-prd "feature description"` | `prds/PRD-<NAME>/PRD-<NAME>.md` |
| 2. Decompose | `/workflow-prd-spec prds/PRD-<NAME>/PRD-<NAME>.md` | `tasks/PRD-<NAME>/TASK-<DOMAIN>-<NNN>-<slug>.md` |
| 3. Verify | `archon workflow lint` + `archon requirements trace` | coverage and topology report |
| 4. Execute | a generated run naming the task directory | workflow execution |

Aliases: `/wf-prd` and `/wf-prd-spec`.

## Step 1 — `/workflow-prd`

```
> /workflow-prd "trading data lake with a coverage matrix and native-interval enforcement"
```

Writes `prds/PRD-<NAME>/PRD-<NAME>.md`, where `<NAME>` is a
SCREAMING-KEBAB-CASE id ending in a three-digit sequence
(`TRADING-DATA-LAKE-AHDM-001`). That id is reused verbatim for the task
directory, so it is chosen once.

What this PRD format requires that `ai-agent-prd` does not:

- **Numbered sections.** Task files cite them in `source_sections:`, so
  numbering is contract, not presentation. Renumbering invalidates citations.
- **Requirement bullets in an exact grammar** — `- REQ-<AREA>-<NNN>: text` at
  column 0, `<AREA>` uppercase letters only, three digits, colon-space, one per
  line, never inside a fenced code block. IDs are extracted by regex, so an ID
  buried mid-paragraph is never extracted and silently does not exist.
- **Per-requirement severity**, as a trailing ``Violation severity: `error` —
  <what fails closed>.`` clause. Severity attached to validation *checks*
  instead has to be recovered by phrase-matching, which on the reference corpus
  classified 2 of 93 requirements.
- **A `## Hard Rules` section**, harvested verbatim into every agent prompt for
  the run.
- **A decomposition section and a traceability table**, so every requirement
  has a named home before any task is written.

## Step 2 — `/workflow-prd-spec <path to PRD>`

```
> /workflow-prd-spec prds/PRD-TRADING-DATA-LAKE-AHDM-001/PRD-TRADING-DATA-LAKE-AHDM-001.md
```

Writes a **flat** directory at `tasks/PRD-<NAME>/`. Discovery is a single
non-recursive read for `TASK-*.md`, so a task file one level deeper is not
found at all — not warned about, not partially loaded.

What this task format requires that `prdtospec` does not:

- **A fenced ```yaml block, first in the file, with ten keys present** —
  `task_id`, `title`, `complexity`, `status`, `depends_on`, `blocks`,
  `implements`, `required_env_keys`, `required_tools`,
  `deliverable_contracts`. Presence, not non-emptiness: `[]` is a valid and
  meaningful declaration, and a missing key is refused naming the file and the
  key.
- **A `task_id` equal to the id in the filename.** A mismatch is refused naming
  both.
- **`## Focused Tests` as runnable commands.** A bullet counts only when it
  contains a backticked span whose first token is a known runner (`cargo`,
  `pytest`, `npm`, `archon`, …). `cargo test -p archon-trading
  registry_migration` proves something; "Registry schema migration test."
  cannot. This is the single highest-value rule: on the reference 17-task
  corpus every entry was prose, and `archon requirements trace` reported
  **0 of 93 requirements satisfied** despite the tasks and requirements
  covering each other exactly.
- **`## Files Expected to Change` as real backtick-quoted paths.** Backticked
  spans are what gets lifted; a section with none yields no anchors and the
  task's requirements cannot promote. "Likely anchors: …" hedges repeated
  across tasks make the section useless as a dataflow signal.
- **`implements: [REQ-...]` always declared**, as a single-line flow sequence,
  `[]` for an audit or review task. Enables both coverage checks: every cited
  ID must exist in the PRD, and every requirement must be claimed by some task.
- **Instance bindings for templated artifact paths.** A path containing
  `<...>` needs `instance_source_path`, `instance_source_records_field`,
  `instance_artifact_field`, and a `min_instances` floor. `min_instances: 0` is
  vacuous — zero matches satisfy it. A typed verifier takes one concrete path
  and cannot be combined with a template.
- **Distinct `kind` for create versus append** on the same path —
  `x_registry` creates, `x_registry_entry` appends.

## Step 3 — verify

```
archon workflow lint --tasks tasks/PRD-<NAME>/
archon requirements trace --prd prds/PRD-<NAME>/PRD-<NAME>.md --tasks tasks/PRD-<NAME>/
```

The lint's `## requirement coverage` section resolves the PRD from the task
directory. It tries the §3.1 adjacent layout first (`<parent>/<id>.md`), then
the `prds/` root this pipeline writes to — `prds/<id>/<id>.md`, `prds/<id>.md`,
and `prds/<id>/PRD.md`. If none exists it names every path it tried and skips
rather than guessing, because a coverage report computed against the wrong
document is worse than none.

`archon requirements trace` takes both paths explicitly, and is the
authoritative traceability check — the lint's coverage section is a convenience.

A requirement is reported satisfied only when a declared verifier command
actually ran and passed **and** the trace shows that run read the anchored
file. Both halves come from the task file: the command from `## Focused
Tests`, the anchor from `## Files Expected to Change`.

## Step 4 — run the workflow

```bash
archon workflow run --decomposed --live --yes \
  "implement the decomposed PRD in /abs/path/to/repo/tasks/PRD-TRADING-DATA-LAKE-AHDM-001"
```

The task text is not decoration — it is how the run finds its work. Two things
must both be true of it, and if either is missing the run refuses rather than
proceeding with an empty task graph.

**1. It must look like a decomposed-PRD run.** The planner only loads a task
universe when the text contains one of three markers, matched
case-insensitively: `decomposed prd`, `task-*.md`, or `/tasks/prd-`. Because
this pipeline writes to `tasks/PRD-<NAME>/`, naming the directory satisfies the
third by construction — but only if you spell the path with forward slashes.

**2. The path must be absolute.** Path tokens are scraped from the text and
only absolute ones are considered — `/…` on Unix, `C:\…` or `C:/…` on Windows.
A relative `tasks/PRD-<NAME>/` yields no roots, and the run fails with
*"generated decomposed PRD workflow requires local TASK-\*.md evidence before
planning"*. That refusal is deliberate: silently planning nothing looks
identical to planning correctly right up until the run reports success having
done nothing.

Nothing relates the task directory to the PRD's location, so `prds/` and
`tasks/` being separate roots costs nothing here.

The flags:

| Flag | Effect |
|------|--------|
| `--decomposed` | Select the decomposed-PRD lifecycle, which plans directly from the declared task graph. Without it you get the v3 authored-script lifecycle, which has a model author a `workflow.js` and executes that. Both read the task universe — see below. |
| `--live` | **Required.** Use the configured provider. |
| `--yes` | Approve a non-interactive live run. Omit it inside the TUI, where you approve at the prompt. |
| `--resume-from <RUN_ID>` | Resume a prior run, reusing its accepted and no-op calls. |

### Previewing a run

**`--live` is not optional on `run`.** Omitting it does not give you a
deterministic smoke run — that path was removed. The binary refuses with
*"legacy deterministic workflow execution was removed by the workflow runtime
rescue; workflows run through the live V2 runtime."*

**`archon workflow plan` behaves differently with and without `--live`, and
only one of the two is a preview:**

- **Without `--live`** it emits a fixed four-stage heuristic scaffold —
  `discover` → `review` → `synthesize` → `quality`. That output is
  byte-identical for any task text and with or without `--decomposed`; only
  `name:` and `task:` echo what you passed. It does not read your task files.
  Use it to see the generic stage shape, not to judge what a run will do.
- **With `--live`** it goes through the provider planner and produces a
  task-specific spec. This is the real preview, and it costs one planner call
  rather than a whole run.

So preview with `archon workflow plan --live <task text>` before committing to
a run.

### What `--decomposed` does and does not change

It selects the lifecycle. It does **not** control whether your task files are
read: `extract_task_universe_for_generated_run` is gated on the *task text*
markers below, not on the flag, so a task text naming a decomposed PRD
directory loads the task universe either way. The v3 lifecycle then receives
the task ids, paths and per-file content fingerprints in its authoring brief
and validates its output against them.

The difference is how the work is planned — straight from the declared graph,
versus a model-authored script informed by that graph.

To watch a run that is already going, or to pick one up afterwards:

```bash
archon workflow status <run-id>
archon workflow continue <run-id>
```

## Ordering-only dependencies

A dependency where the upstream task produces no artifact the downstream task
consumes is a sequencing edge, and the lint reports it as such. That is
information, not a defect. **Do not fabricate a deliverable contract to silence
it** — an invented artifact path turns a truthful ordering edge into a false
dataflow claim and creates a gate nothing produces.

## Reference corpus

`tests/fixtures/prd-trading-data-lake-ahdm-001/` holds a real 17-task set in
this format, with `expected-parse.json` pinning what the parser reads from it.
It is the worked example — including its two known defects, prose focused tests
and repeated anchor lists, which are exactly what the template above exists to
prevent.

---

# Both pipelines

## Project initialisation

Before running the pipeline, initialise the project:

```bash
# If building from source
bash scripts/archon-init.sh --target $(pwd)

# If using a binary install
curl -L https://raw.githubusercontent.com/ste-bah/archon-cli/main/scripts/archon-init.sh | bash
```

This creates `.archon/`, `prds/`, and `tasks/` directories. Both pipelines use
those same two roots — the skills chain in `prds/<slug>/` and `tasks/phase<N>/`,
the workflow path in `prds/PRD-<NAME>/` and `tasks/PRD-<NAME>/`.

## See also

- [Running god-code pipelines](god-code-pipeline.md) — `/archon-code` internals
- [Skills reference](../reference/skills.md) — full skill catalogue
- [Setup wizard](../operations/setup-wizard.md) — first-run configuration
