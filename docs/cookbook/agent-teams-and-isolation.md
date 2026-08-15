# Agent teams, messaging, and isolation

How to run several agents on one job without them talking past each other,
overwriting each other, or finishing into silence.

Companion pages: [Multi-agent handoffs](multi-agent-handoffs.md) for the task
board, [Agent task board](../architecture/agent-task-board.md) for why the board
is shaped the way it is.

## The problem this solves

Spawning two agents was always easy. Everything after that was not.

A subagent that called `SendMessage` got its own request back as the tool result
and carried on believing it had sent something. An agent that finished told
nobody — the lead learned by asking. Two agents editing the same file discovered
it at merge time, or never. And an isolated agent's work landed on a branch in a
directory nobody looked at, so the only options were to trust it or go reading.

The pieces below are the fix. You can use any of them alone; they compose.

| what | where |
|---|---|
| talk to another agent | `SendMessage` |
| know when one finishes | status envelopes, automatic |
| say what you will write | `intended_writes` on the `Agent` call |
| keep writers apart | `isolation`, or automatic |
| see and merge the result | `/worktrees` |
| a named group with roles | `TeamCreate` / `TeamDelete`, `/agent` |

---

## Talking

Every subagent has `SendMessage`. You do not have to grant it — it is unioned
into the toolset alongside the board tools, whatever the agent's allowlist says.
An agent that cannot answer its lead is not a teammate.

```
SendMessage
  to           "reviewer"   role, agent name, agent id, or "lead"
  message      "src/auth.rs is ready for review"
  summary      "auth ready"          optional, one line
  message_type "text"                default
```

`to: "lead"` reaches the agent that spawned this one. The router resolves it from
the sender's own identity — a child cannot assert who its parent is, and there is
no author field on the message for a model to set.

Delivery happens at the recipient's next **tool round boundary**. Nothing is ever
injected into a request already in flight, so a busy agent finishes its thought
first. A queue caps at 64 messages and then refuses at the sender, which is the
honest failure: better than a message vanishing into a queue nobody drains.

Messages between team members arrive attributed:

```
<archon_team_message from="explore" to="general-purpose" type="chat">
please review src/auth.rs
</archon_team_message>
```

Without the `from`, a recipient cannot reply, which makes a mailbox a dead-drop.

### Asking one to stop

```
SendMessage  to "reviewer"  message_type "shutdown_request"  message "we are done here"
```

Cooperative: it trips a flag the agent checks at its next round, so it finishes
what it is doing rather than being killed mid-write.

`shutdown_response` and `plan_approval_response` carry `approve`, so sending one
*is* giving consent. They are honoured **only from the lead session**. A peer or
child sending one is refused and logged.

---

## Knowing when an agent finishes

Automatic. Every terminal state queues an envelope for the lead:

```
<archon_agent_status agent_id="3db484d4" name="reviewer" status="completed">
<result>Found two missing null checks in src/auth.rs:88 and :140.</result>
</archon_agent_status>
```

`status` is `completed`, `failed`, or `idle`. The `idle` one is the point: an
agent that outlives its auto-background timer is still running, and without this
a wedged agent and a busy one look identical.

A cancelled agent reports as `failed`. The completion path receives only
success-or-error, so it cannot tell the two apart and says so rather than
guessing.

---

## Declaring what you will write

Pass `intended_writes` when you spawn a writer:

```
Agent
  subagent_type   "general-purpose"
  prompt          "Add rate limiting to the Bedrock provider."
  intended_writes ["crates/archon-llm/src/providers/bedrock.rs"]
```

If a running agent has already claimed an overlapping path, the spawn returns a
warning naming it. It is a **warning, not a refusal** — declaring intent has to
be better than staying silent, and refusing the spawn would teach the opposite.

Claims are derived from liveness, not leases. There is no release call and no TTL:
a claim exists exactly as long as its holder is running. An agent that crashes
cannot leave a stale claim behind, because there is nothing to leave.

Overlaps use the same resource-key table the write planner uses, so
`src/a.rs` and `./src/a.rs` are one path, and a directory covers what is under it.

---

## Keeping writers apart

An agent can run in the shared tree or in its own git worktree.

| tier | `isolation` | what it gets |
|---|---|---|
| shared | `none` | your working tree, like any tool call |
| worktree | `worktree` | its own checkout and branch; **build commands refused** |
| worktree with builds | `worktree-with-builds` | its own checkout, branch, and scratch build dir |

Tier 2 refuses `cargo build`, `npm run build` and friends *before* anything runs,
and refuses a chained command if **any** segment builds — `ls && cargo build`
builds. A single `cargo check` in a fresh worktree creates the cold `target/` the
tier exists to avoid: gigabytes, with no undoing it afterwards. If an agent needs
to build, give it tier 3 deliberately:

```
Agent
  subagent_type "general-purpose"
  isolation     "worktree-with-builds"
  prompt        "Fix the failing test in crates/archon-llm and run cargo test -p archon-llm."
```

The refusal is keyed on the **command text**, not the working directory. An
isolated agent still has the main checkout in scope, so it could `cd` out of its
worktree in a single bash line; a gate that trusted the cwd would be walked
around immediately.

You rarely set this by hand. In `config.toml`:

```toml
[subagent]
# off | overlap | always — when to isolate an agent that did not ask
auto_isolation     = "overlap"
# the ceiling; a request for more is clamped and logged
isolation_max_tier = "worktree"
```

`overlap` is the default and the useful one: isolate a write-capable agent
**only when its declared writes overlap a running agent's**. Isolation costs disk;
disjoint writers do not need it.

---

## Seeing and merging the result

An isolated agent's work is not in your tree until you say so. Nothing merges
automatically.

```bash
/worktrees
```

```
2 agent worktree(s):

  ● subagent-3db484d4
    branch 'archon/subagent-3db484d4' — 3 files changed, +48 -6, 2 ahead
    owner: running  age: 12m
    C:\Users\you\AppData\Roaming\archon\worktrees\subagent-3db484d4

  ○ subagent-9f21c0aa
    branch 'archon/subagent-9f21c0aa' — 1 file changed, +12 -0, 1 ahead, 4 behind main
    owner: finished  age: 1h
    C:\Users\you\AppData\Roaming\archon\worktrees\subagent-9f21c0aa
```

Worktrees live in the **user data directory** — `%APPDATA%\archon\worktrees\` on
Windows, `~/.local/share/archon/worktrees/` elsewhere — not inside the project.
They are named after the agent, not the session, so two agents in one session
cannot destroy each other's uncommitted work. One consequence worth knowing:
they accumulate across projects, and `/worktrees sizes` is how you find out how
much.

Diffstats are measured against the **merge base**, not the base branch tip. The
base moves while an agent works, and diffing against a moved tip attributes
everyone else's commits to this agent. `4 behind main` is the row whose merge is
about to be interesting.

| command | effect |
|---|---|
| `/worktrees` | the listing above (alias `/wt`) |
| `/worktrees sizes` | same, plus disk usage — walks every file, so opt in |
| `/worktrees merge <owner>` | integrate the branch, remove the worktree |
| `/worktrees discard <owner>` | throw the work away |
| `/worktrees keep <owner>` | leave it, branch and all |
| `/worktrees prune` | remove every **finished** agent's worktree |

`prune` filters on liveness, not age: a finished agent's worktree is reclaimable
now and a running agent's never is, whatever its age. Anything with uncommitted
work refuses and says which.

Every action on a live agent's worktree is refused while it is running.

---

## Teams

A team is a named group with roles. Use one when you want the agents to address
each other by role, and when you want a roster to look at.

```
TeamCreate
  name    "auth-review"
  members [ { role: "explore",         system_prompt: "You explore." },
            { role: "general-purpose", system_prompt: "You do small tasks." } ]
```

**The role must be a real `subagent_type`.** Spawning an agent whose
`subagent_type` matches a role seats it on that role; that is the whole binding,
and it is also the address teammates send to. A role no agent type matches is a
seat nothing will ever fill.

The roster lives at `<project>/.archon/teams/<team-id>/team.json` and is written
by the runtime, not by you. Seats fill when agents start and empty when they
reach any terminal state.

```bash
/agent
```

```
Team 'auth-review' (023d3b6f) — 1 of 2 seat(s) filled
  explore              vacant
  general-purpose      running     3db484d4-435d-4e8a-896c-90889b763a05
                         task: audit the auth module
                         writing: crates/archon-core/src/auth.rs

Tip: members address each other by role with SendMessage.
```

`archon team list` shows every team on the project from outside a session.

### Shutting one down

```
TeamDelete  team_id "023d3b6f"
```

A handshake, not a delete. It sends `shutdown_request` to every seated member,
waits for them to leave the roster — leaving is the acknowledgement, because an
agent vacates its seat from the same hook that reports it complete — and removes
the team last.

A member that does not stop within 60 seconds leaves the team **intact** and is
named in the refusal. A half-deleted team is worse than one that is still there.

One team per session: creating a second while members are running is refused,
because switching would strand them on a roster nothing reads.

---

## Recipes

### Non-coding: a two-role research team

Two agents, one gathering and one checking, talking directly instead of routing
every exchange through you.

```
Create a team called market-scan with two members:
  role explore, system_prompt "You gather sources and quote them."
  role general-purpose, system_prompt "You check claims against sources."

Then spawn an explore agent to find the three most cited papers on X, and a
general-purpose agent to verify each citation the explore agent sends it.
Tell the explore agent to SendMessage each finding to general-purpose as it
goes, rather than waiting until the end.
```

Why it is better than one agent: the checker sees each claim while the gatherer
is still working, so a bad source is caught before ten more are built on it.
Check progress with `/agent`; the roster shows who is running and what they hold.

### Non-coding: a long document, split by section

```
Spawn three general-purpose agents in the background, one per section, each with
intended_writes naming only its own output file. Have each SendMessage to lead
when its section is done.
```

`intended_writes` on separate files means no overlap, so nothing is isolated and
nothing needs merging — the cheap path, taken automatically. The status
envelopes tell you when each lands without polling.

### Coding: parallel work on one module

```
Spawn two agents:
  one with intended_writes ["crates/archon-llm/src/providers/bedrock.rs"]
  one with intended_writes ["crates/archon-llm/src/providers/vertex.rs"]
```

Disjoint, so both run in the shared tree. Now try:

```
  one with intended_writes ["crates/archon-llm/src/providers/"]
  one with intended_writes ["crates/archon-llm/src/providers/bedrock.rs"]
```

The second spawn warns about the overlap, and with `auto_isolation = "overlap"`
it gets its own worktree. When it finishes:

```bash
/worktrees
/worktrees merge subagent-9f21c0aa
```

A conflicting merge is refused and the branch is left alone, so nothing is lost
while you resolve it.

### Coding: a reviewer that can push back

```
Create a team with roles general-purpose and explore.
Spawn general-purpose to implement the change with intended_writes naming the
files. Spawn explore to review whatever general-purpose messages it, and to
SendMessage concerns straight back rather than to me.
```

The implementer gets review comments at its next tool round and can act on them
in the same run. Without member-to-member messaging this is two sequential runs
with you as the relay.

### Coding: shutting a runaway down

```
/agent                                   # find the role and id
```
```
SendMessage to "general-purpose" message_type "shutdown_request" message "stop, wrong approach"
```

Cooperative, so it finishes the file it is writing. If it holds a worktree, its
work is still on its branch — `/worktrees` will show it, and you can read the
diffstat before deciding to merge or discard.

---

## What gets learned

Coordination outcomes feed the world model, so the advisor can eventually answer
"will these two conflict?" before you spawn them.

`/worktrees merge` and `discard` each write one trace row carrying what was known
at spawn — declared writes, whether they overlapped, which isolation tier was
granted — against what happened: files actually changed, and whether the merge
conflicted.

Merge results are **ground truth**. A git merge either conflicted or it did not;
no labeler judged it. That makes `merge_conflict` the one signal here a model can
train against directly, and it is served through the same guardrail path that
already runs per turn.

Nothing is collected unless you merge or discard through `/worktrees`.

---

## Gotchas

**A role that is not a real agent type never fills.** `TeamCreate` will happily
declare `role: "reviewer"`, but seating binds on `subagent_type`, and there is no
built-in `reviewer`. Use `/agent` to see what exists, or write a custom agent
definition with that name.

**The team is per session.** It lives in the process that created it. A second
`archon` session sees the `team.json` on disk via `archon team list` but has no
active team of its own, so its spawns seat nowhere.

**Tier 2 refuses builds by design.** If an agent reports that `cargo test` was
refused, it was isolated without build permission. Give it
`isolation: "worktree-with-builds"` if it genuinely needs to build, and expect
the disk cost.

**Worktrees are not cleaned up for you when they contain work.** That is
deliberate — the alternative discards unreviewed changes. `/worktrees prune`
removes the finished, empty ones; the rest wait for you.

**Overlap warnings are advisory.** Two agents can still write the same file if
you ignore the warning and isolation is off. The claim tells you; it does not
stop you.
