# Multi-agent handoffs

How to get work between agents without losing it, and how to make "leave no gaps"
something the run enforces rather than something you retype every session.

Background on why the board is shaped the way it is:
[Agent task board](../architecture/agent-task-board.md).

## The problem this solves

Ask archon to do something with subagents and one of them will find a problem
outside its own assignment. Before the board, that finding had exactly one
destination: the subagent's final message. The parent then summarised several
such messages into a paragraph, and the finding was gone.

You would read *"rate limiting added to the LLM client"*. What happened was
*"three of four providers wired; the fourth has no interception point"*. You find
out weeks later.

## The four tools

Every subagent has these, and so does the top-level agent.

| tool | use |
|---|---|
| `BoardRaise` | record something that must happen, or a note for whoever touches this area next |
| `BoardList` | see what is open for this run, filterable by status |
| `BoardClaim` | take an item before working it |
| `BoardResolve` | close it out as `resolved` or `declined`, with a reason |

Items are scoped to the run automatically — the run id is derived from the
session, so nothing has to be passed around.

### Evidence is required, not encouraged

`BoardRaise` refuses an item with no evidence, at the storage layer. An item that
says *"the config looks wrong"* is unactionable by whoever picks it up, and the
person best placed to say what they saw is the agent that just saw it.

Write file paths and line numbers. `providers/bedrock.rs:212 — send() builds and
dispatches in one call, no middleware seam` is worth keeping. *"Bedrock needs
work"* is not.

### Issues and notes are different

An **issue** is work that must happen; it outlives the run and must be closed.
A **note** is context the next agent in this area will want; it dies with the
run.

Conflate them and the board fills with *"looked at X, seemed fine"*, which is how
a tracker becomes a log nobody reads.

## Claims expire with the agent that holds them

`BoardClaim` is an atomic compare-and-set: exactly one caller wins a contested
item, and the loser is told who holds it rather than silently overwriting.

Claims are leased against **real agent liveness**, not a timeout. If the holder
dies mid-work, its claim is released automatically the next time anything lists
or claims on that run — no stale-claim heuristics, no "treat that as old state
unless there is evidence someone is working on it" instruction in your prompt.

You do not have to name the board tools — or `SendMessage`, or `Sleep` — when you
spawn an agent with an explicit `allowed_tools` list.
`archon_core::dispatch::ALWAYS_AVAILABLE_TOOLS` unions all six into a subagent's
tool set however that set was derived: from the request, from the agent
definition, or from the defaults. Most pipeline agents name their tools
explicitly, so without that the board would have been absent from exactly the
fan-outs it exists to coordinate.

The same list is retained by `ToolRegistry::filter_whitelist`, so a session
started under a restrictive agent definition cannot drop these either. That
matters because the whitelist *deletes* from the registry, and a subagent's
toolset is taken from that same registry by name — a tool dropped at session
start could not be restored downstream however loudly the spawn asked for it.

A denylist entry still wins, so this offers the tools rather than overriding a
deliberate refusal.

`SendMessage` joined the set for the same reason and was found the same way. #184
made a subagent's messages route properly and made team members addressable by
role, but no built-in agent definition names `SendMessage` — so an `explore` agent
asked to message a teammate correctly reported it had no such tool. An agent that
cannot be spoken to is not a teammate, and routing nothing can invoke is machinery
pretending to be a feature.

## What "done" means now

A workflow run **cannot report success while it has open issues**. At the final
barrier, every issue for the run must have reached one of:

- `resolved` — the work is done
- `promoted` — moved to GitHub because it outlives this run
- `declined` — with a recorded reason

Anything still open fails the run, and the failure names the items.

`escalated` deliberately does **not** count as drained. It is an unanswered
question, and letting it pass would ship one as an answer.

This is what turns *"leave no gaps"* from a hope into a property. You do not have
to ask for it.

### Declining needs a reason, and the store enforces it

You cannot decline an item with nothing behind it. The requirement lives in the
storage layer rather than only in the gate, so a caller that bypasses the gate
still cannot leave an unexplained decline.

## Reviewers can say the task itself is wrong

Workflow review has three outcomes, not two:

- **accepted**
- **gaps remain** — specific, evidenced, and returned for another round
- **assignment invalid** — this task as written cannot or should not be done

The third one matters more than it looks. With only pass and fail, an agent that
discovers its assignment was mis-scoped is marked as failing — which teaches it
to produce something that passes instead of telling you the assignment was wrong.

It is not a free escape hatch. A claim of `assignment_invalid` needs provenance
from a review branch (so the reducer holding the whole run cannot use it to
excuse work it could not reconcile), a named task, a stated reason, and
`file:line` evidence — a bare path will not do, because the line number is what
distinguishes *"I went and looked"* from *"I could not do it"*. A claim that
fails those checks is downgraded back to ordinary remediable work rather than
dropped.

## Watching it happen

`/web` starts the dashboard **inside** the running session, so it shows the
agents *this* session spawned rather than opening a second independent one. The
agent panel lists what is running, and the board view shows what has been raised
and what is blocked.

`archon web` as a standalone command still works, and the two surfaces differ in
one specific way rather than wholesale:

- **The board view works in both.** The board lives in the memory database, so
  any process that can open it sees the same rows. A standalone `archon web`
  shows the real board of whatever project it is pointed at.
- **The agent panel only works attached.** It reads in-process registries
  holding live `JoinHandle`s, which cannot cross a process boundary — so a
  separate process could never report the agents in your session, no matter what
  was recorded for it.

So reach for `/web` when you want to watch agents work, and standalone
`archon web` when you want to read the board without a session running.

## A worked example

> add rate limiting to the LLM client, use subagents, leave no gaps

1. Three subagents start. Each claims its work.
2. `provider-wiring` finishes three providers, then hits one with no interception
   point. It raises an issue with the file and line, and keeps going.
3. `limiter-core` and `tests` both notice the config has no `rate_limit` section.
   One item, two witnesses — the duplicate is merged rather than filed twice.
4. `provider-wiring` dies on a provider API error. Its claim is released
   automatically; no timeout, no manual intervention.
5. At the final barrier the run has three open issues. It **cannot** report
   success. Two are resolved by follow-up work; the interception-point one is
   promoted to GitHub because it needs a refactor beyond this run.
6. You are told: rate limiting is wired for three providers, the fourth is not
   and here is the issue number.

Step 6 is the thing you are buying. Without the board, step 2's finding is a
sentence in a summary you never see.
