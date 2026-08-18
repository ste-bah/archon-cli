# Plan Mode trusted execution cookbook

Use Plan Mode when you want Archon to inspect a repository and agree an implementation path before working-tree mutations are allowed.

Plan Mode is a permission boundary, not just a prompt style. `Write`, `Edit`, and `Bash` are blocked by default. Read-only inspection and the explicit process-state controls `TaskCreate`, `TaskUpdate`, and `Agent` remain available, but agents and their tools stay subject to Plan Mode and preflight checks.

## Plan a cross-cutting change

Start an interactive session, then enter Plan Mode:

```text
/plan
```

Give Archon a concrete goal and constraints:

```text
Plan a safe migration from the legacy session cache to the durable store. Inspect the current call sites, preserve restart behavior, write tests before implementation, and include rollback steps. Do not modify files yet.
```

Useful follow-up commands:

```text
/plan show
/plan open
```

`/plan show` displays the current Plan Mode state. `/plan open` creates or opens the active editable plan document. Durable plan documents live at:

```text
.archon/plans/<plan-id>.md
```

Plan Mode interceptions are appended to:

```text
.archon/plan-audit/<session-id>.md
```

## Review and approve the structured plan

When Archon submits `ExitPlanMode`, the interactive approval prompt renders the stored plan title and numbered steps. Reply with one exact protocol value:

```text
approve
approve-edits
edit
reject: missing rollback coverage
```

- `approve` accepts the plan and restores the safely recorded pre-plan permission mode.
- `approve-edits` accepts the plan and restores `acceptEdits` explicitly.
- `edit` keeps the plan as a draft so it can be revised.
- `reject: <reason>` abandons the submitted plan and keeps Plan Mode active, so working-tree mutations remain blocked.

A normal approval does not invent `auto`. If no safe prior mode exists, Archon restores `default`. `bypassPermissions` is restored only when the host was explicitly started with dangerous bypass authorization; otherwise it is downgraded to `default`.

## Revise after review

If the plan is incomplete, answer:

```text
edit
```

Then state the required changes:

```text
Add a Windows test, separate schema migration from data backfill, and define rollback evidence for each phase.
```

Open the plan again if needed:

```text
/plan open
```

Archon remains in Plan Mode until a later structured submission is approved or you explicitly leave Plan Mode.

## Reject an unsafe plan

Use a reason that tells Archon what must change:

```text
reject: the plan deletes the old store before restart compatibility is verified
```

Rejection is terminal for that submitted plan and auditable. It does not grant mutation permission.

## Leave without structured approval

If you no longer want to use the structured plan, these user commands exit Plan Mode directly and restore `default`:

```text
/plan off
/plan exit
/plan done
```

These are explicit cancellation/exit controls. They do not approve the active structured plan.

## Run unattended sessions fail-closed

Interactive approval is unavailable in one-shot and other noninteractive sessions. The default noninteractive policy approves. For CI or unattended planning where missing human approval must stop execution, configure:

```toml
[context]
noninteractive_plan_approval = "reject"
```

Any value other than `"reject"` uses the approving default. Use the exact reject value when fail-closed behavior matters.

A separate model can handle requests while the agent is in Plan Mode:

```toml
[context]
plan_model = "claude-sonnet-4-6"
```

Remove `plan_model` to use the active session model.

## Resume durable plan work after restart

Approved plan steps materialize as plan-linked tasks. They persist in the plan store and rehydrate into `TaskList` when the same session and durable store are reopened. Unrelated manual tasks remain process-scoped.

After restart:

1. Resume the same Archon session.
2. Inspect the rehydrated task list.
3. Continue the approved steps rather than creating replacement task IDs.
4. Record the required evidence before marking a step complete.

Plan-step completion can require authoritative test evidence. A successful-looking text message is not evidence: Archon records the real Bash execution identity, command and output hashes, exit code, attempt, and signed summary. Missing or failed required evidence blocks completion.

## Review reconciliation at the end

Plan reconciliation survives conversation compaction because it is stored durably. It records:

- `completed` — the planned step was completed
- `omitted` — the planned step was not completed
- `deviated` — execution differed from the planned step
- `unplanned-extra` — a changed file fell outside the planned paths

Treat deviations and unplanned extras as review inputs, not automatic approval. Inspect them before declaring the implementation complete.

## Practical workflow

For a real feature change:

```text
/permissions default
/plan
Plan issue #245. Inspect API, storage, tests, and docs. Produce small reversible steps, name affected files, require tests for every implementation step, and include restart verification.
```

Then:

1. Read the stored steps in the approval prompt.
2. Reply `edit` if file coverage, rollback, or tests are incomplete.
3. Reply `approve-edits` when you want approved file edits without automatically approving Bash.
4. Execute the plan-linked tasks in order.
5. Require real test-run evidence before task completion.
6. Review reconciliation for omissions, deviations, and unplanned files.

## Related reference

- [Permissions](../reference/permissions.md)
- [Slash commands](../reference/slash-commands.md#plan-mode-lifecycle)
- [Configuration](../reference/config.md)
- [Tools](../reference/tools.md)
