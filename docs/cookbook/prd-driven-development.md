# PRD-driven development

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

### Inspecting and resuming mid-pipeline

```
> /pipeline status                # current run id + phase + last completed agent
> /pipeline list                  # all sessions, resumeable + completed
> /pipeline resume <session-id>   # continues from last completed gate
> /pipeline abort <session-id>    # cleans up partial state, keeps ledger
```

If `/archon-code` crashes (rare) or you Ctrl-C deliberately, the resume path is git-aware: it refuses to continue if files have changed under it.

## Task atomicity criteria

`/spec-to-tasks` checks each task against:

- **Single responsibility** — one clear deliverable
- **Testable** — can you write a test that verifies completion?
- **< 1 working day** — if it looks bigger, split it
- **No implicit dependencies** — dependencies must be listed explicitly by task ID

## Project initialisation

Before running the pipeline, initialise the project:

```bash
# If building from source
bash scripts/archon-init.sh --target $(pwd)

# If using a binary install
curl -L https://raw.githubusercontent.com/ste-bah/archon-cli/main/scripts/archon-init.sh | bash
```

This creates `.archon/`, `prds/`, and `tasks/` directories.

## See also

- [Running god-code pipelines](god-code-pipeline.md) — `/archon-code` internals
- [Skills reference](../reference/skills.md) — full skill catalogue
- [Setup wizard](../operations/setup-wizard.md) — first-run configuration
