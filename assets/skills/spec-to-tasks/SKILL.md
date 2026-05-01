---
name: spec-to-tasks
description: Refine the tasks/phase<N>/task<M>.md tree from /prd-to-spec into atomic, dev-flow-ready task files with verification checklists. Use after /prd-to-spec to validate and refine the generated task specs.
---

# Spec to Tasks

Refine the task tree created by `/prd-to-spec` into atomic, dev-flow-ready task files.

## Process

### 1. Survey the task tree

Read `tasks/INDEX.md` (created by `/prd-to-spec`) and the per-task files under `tasks/phase*/task*.md` using the Read tool. Build a mental model of the full decomposition.

### 2. Verify atomicity

For each task, check against these criteria:

- **Single responsibility** — one clear deliverable, not a grab-bag
- **Testable** — can you write a test that verifies completion?
- **< 1 working day** — if it looks bigger, split it
- **No implicit dependencies** — dependencies must be listed explicitly by task ID

### 3. Split or merge

- **Too coarse?** Split into sub-tasks: `tasks/phase<N>/task<M>a.md`, `task<M>b.md`
- **Trivially mergeable?** Two tasks with no value from separation → merge into one
- Use Edit/Write tools for both operations

### 4. Ensure completeness per file

Each FINAL task file must contain:

- **Title** — short, descriptive, imperative
- **Description** — what, not how
- **Acceptance criteria** — verifiable outcomes
- **Dependencies** — list of other task IDs this depends on
- **Test plan** — how to verify completion
- **Files to modify** — hint, not exhaustive

### 5. Update the index

Update `tasks/INDEX.md` to reflect the refined tree after all splits/merges.

### 6. Summarize

Print a one-line summary: N split, M merged, K refined. Do NOT print task contents.
