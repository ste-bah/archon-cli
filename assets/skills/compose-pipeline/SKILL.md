---
name: compose-pipeline
description: Sequential one-command run of /to-prd → /prd-to-spec → /spec-to-tasks, then hands off to the user for manual /archon-code invocation. Use when you want the full PRD-driven development loop in one command.
---

# Compose Pipeline

Chain three skills sequentially via the Skill tool, then hand off to the user for the final coding step.

## Process

### Step 1 — Confirm with user

Before starting, confirm with the user:

> "About to run /to-prd → /prd-to-spec → /spec-to-tasks on '<feature>'. After that finishes, you'll manually invoke /archon-code on the refined task tree. Proceed?"

Wait for explicit confirmation.

### Step 2 — /to-prd

Invoke `Skill(action=invoke, name=to-prd, args=[<feature>])` and wait for the result.

The PRD will be written to `prds/<slug>/PRD.md`. Note the path.

### Step 3 — /prd-to-spec

Invoke `Skill(action=invoke, name=prd-to-spec, args=[<prd path from step 2>])` and wait.

### Step 4 — /spec-to-tasks

Invoke `Skill(action=invoke, name=spec-to-tasks, args=[])` and wait.

### Step 5 — Hand off

After the third skill returns, print:

```
Composition phase finished. Refined task tree at tasks/INDEX.md.
To run the 50-agent coding pipeline, invoke this manually:

    /archon-code

(Skipping auto-invocation: the Skill tool cannot reach /archon-code,
which is a separate command-registry primary. Manual invocation also
ensures you confirm the expensive run step explicitly.)
```

## Limitations

The Skill tool resolves names against `register_builtins()` only (the SkillRegistry). `/archon-code` lives in the broader command registry, so the Skill tool returns `"skill 'archon-code' not found"` if invoked. A future Phase 3.5 ticket can add an `ArchonCodeSkill` shim that registers in `SkillRegistry` and dispatches into the existing pipeline runner.
