# Skills and the method chain

Skills are written methods the model loads on demand. Since #187 it loads them
on its own judgement: every agent-invocable skill is listed in the system
prompt with its description, and the model invokes one via the `Skill` tool
when a task matches. You do not have to type the slash command.

You still can — `/tdd`, `/diagnose`, `/verify-done` all work by hand.

---

## Add a skill to a project

Drop a file in `.archon/skills/<name>/SKILL.md`. That is the whole interface —
no registration step and nothing to add to config. Restart the session to pick
it up: both the prompt catalogue and slash autocomplete are built at startup.

```markdown
---
name: release-notes
description: Use when a version is about to be tagged, or when asked what changed since the last release. Reads merged PRs and writes the notes in this project's house format.
---

# Release notes

## Process

1. `git log <last-tag>..HEAD --oneline` for the raw list
2. Group by user-visible change, not by commit
3. Lead each entry with what a user can now do, not what was refactored
4. Anything with no user-visible effect goes in a "Internals" section or nowhere
```

**The description is the trigger.** It is the only thing the model sees when
deciding whether to load the skill, so write the *situation* rather than the
topic. "Use when a version is about to be tagged" fires; "Release note helper"
does not.

Check it landed:

```bash
archon -p "what skills do you have for release work?"
```

---

## The chain

Five skills that name each other, so following one leads through the rest:

```
/grill-me            settle the design by interview
      ↓
/compose-pipeline    turn it into tasks
      ↓
/execute-plan        build it, one fresh subagent per task, each reviewed
      ↓
/verify-done         prove it works
      ↓
/land-branch         merge, PR, keep, or discard
```

You rarely invoke these by name. Ask for something substantial and the model
picks up `/grill-me` before it starts guessing at requirements, and
`/verify-done` before it tells you it is finished.

---

## Coding example: a feature, end to end

```
You: add rate limiting to the public API
```

What happens without you asking for it:

1. **`/grill-me`** — per-key or per-IP? What is the limit? What does a
   throttled caller get back, 429 or a queue? Answers come one question at a
   time, with a recommendation attached to each.
2. **`/compose-pipeline`** — the settled design becomes a task tree.
3. **`/execute-plan`** — each task gets a fresh subagent with only that task's
   brief. A separate agent reviews the diff against the task. Findings go back
   for up to five rounds, then the controller decides and records why.
4. **`/tdd`** inside each task — failing test first, watched fail, then the
   minimum code to pass.
5. **`/verify-done`** — the real test command runs, output compared against a
   prediction written *before* looking.
6. **`/land-branch`** — merge or PR.

If step 5 finds something it cannot close, it goes on the board as
`gaps_remain` and **the turn does not end** until it is fixed or declined with
a reason. See the gate below.

---

## Non-coding example: a decision document

Skills are not only for code.

```
You: we need to decide whether to move the ingest pipeline off Kafka
```

- **`/grill-with-docs`** — grills the proposal against the domain model already
  written down, sharpens the vocabulary, and offers an ADR when the decision is
  hard to reverse, surprising without context, and the result of a real
  trade-off. All three, or no ADR.
- **`/zoom-out`** when the discussion has descended into partition counts and
  lost the question.
- **`/verify-done`** still applies: the claim "we checked the throughput
  numbers" needs a source, not a recollection.

Other non-code fits: `/grill-me` before committing to a project plan,
`/write-a-skill` to capture a workflow you have now explained three times.

---

## The completion gate

`/verify-done` records what it could not close as a board item in
`gaps_remain`. While one exists for the run, the turn will not end — the
findings come back to the model as a repair prompt.

```
This turn cannot finish yet: a review left gaps that are still open.

- `itm-4f2a` — integration test for the throttled path never runs
  evidence: test asserts on a mock, not the middleware
  done when: the test exercises the real request path
```

Clear it with `BoardResolve` after fixing, or decline it with a reason if the
finding turns out not to be worth acting on. An unexplained open gap is what
keeps the turn from ending.

```toml
[skills]
completion_gate = "block"   # default; "warn" logs instead, "off" disables
```

**Why this is not just another instruction.** A skill can describe a
verification step; it cannot stop a model from declaring victory. The rule most
likely to be rationalised away is precisely the one standing between the model
and finishing. The gate is the part that is not advice.

The status is set by review flows only, so ordinary task tracking never trips
it. A session with no memory service has no board, and the gate allows the turn
rather than wedging it.

---

## Gotchas

**A description that only matches its own name never fires.** "Use when the
user says 'grill me'" cannot trigger autonomously. Lead with the situation and
put the phrasings second.

**Descriptor-only skills are not in the catalogue.** `/help`, `/cost`,
`/status` render in the TUI and never reach the model, so listing them would
cost prompt tokens for nothing. Only skills that emit a prompt are advertised.

**Agent definitions read a top-level `skills:` key.** Nested under
`capabilities:` it is descriptive metadata and is not loaded. Names that do not
resolve are dropped with a warning rather than presented to the model as
callable.

**Adding a skill does not cost you the prompt cache.** The catalogue sits in
the turn-variable section, after the cached prefix, precisely so a project
adding a `SKILL.md` does not invalidate every session's cache.

**Overriding a built-in needs no rebuild.** A project `.archon/skills/tdd/SKILL.md`
replaces the embedded body; resolution runs project → user config → user data →
embedded.

---

## See also

- [Skills reference](../reference/skills.md) — registry, override order, plugin skills
- [Agent teams and isolation](agent-teams-and-isolation.md) — worktrees, claims, teams
- [Hooks reference](../reference/hooks.md) — what each event's result is used for
