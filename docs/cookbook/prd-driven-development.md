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
