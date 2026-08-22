# Hooks

A hook is a piece of work Archon runs at a named point in a session — a shell
command, an HTTP call, a named in-process function, or a sub-agent. Hooks are
configured on disk, keyed by event, and every hook for an event runs before the
runtime looks at any of their results.

Read the second section first. The hook system will run almost anything you
configure, but only a few events do anything with what a hook returns, and the
gap between "my hook ran" and "my hook changed something" is where the time
goes.

## Where hooks come from

Five sources are read at startup, in this order:

| # | Path | Source tag |
|---|---|---|
| 1 | `<project>/.archon/settings.json`, the `"hooks"` field | `project` |
| 2 | `<home>/.archon/hooks.toml` | `user` |
| 3 | `<project>/.archon/hooks.toml` | `project` |
| 4 | `<project>/.archon/hooks.local.toml` | `local` |
| 5 | `<home>/.archon/policy/hooks.toml` | `policy` |

Each path has a `.claude/` equivalent that is used only if the `.archon/` one
does not exist, and loading from it logs a deprecation warning
(`hooks/registry/load.rs`, `load_all`).

Load order is execution order. Afterwards the registry deduplicates **per event
by `(hook_type, command)`, keeping the last** (`hooks/registry/matching.rs`,
`deduplicate`). Two consequences follow that are easy to trip over:

- The matcher is **not** part of the dedupe key. The same command string
  registered under two different matchers collapses to one hook, and the one
  that survives is whichever was loaded last. If you want the same script to run
  for three tools, give each invocation a distinct command string — pass the
  tool name as an argument, for instance. The repository's own
  `.archon/hooks.toml` does exactly that, and says why.
- "Later source wins" is how a policy file overrides a project file, but it
  overrides by *replacing*, not by merging fields.

The source tag is not just provenance. When results are merged, a hook that
returns `permission_behavior: "allow"` has that dropped with a warning unless
its source tag is `policy` (`hooks/types.rs`, `AggregatedHookResult::merge`).
Any source may return `deny`.

### The TOML shape

```toml
[hooks.PreToolUse]
matchers = [
  { matcher = "Bash", hooks = [
    { type = "command", command = "scripts/check.sh", timeout = 10 }
  ]}
]
```

Event names are the `HookEvent` variant names in PascalCase. The twelve
`Before*`/`After*` runtime events additionally accept a snake_case alias
(`before_tool_call`, `after_agent_run`, and so on).

A missing file is silently skipped; a malformed one is an error that drops that
whole source (`hooks/toml_loader.rs`).

### Enabling and disabling without editing config

`/hooks list` prints every registered hook with a stable nine-character id
(`h` plus eight hex, a SHA-256 over the event, the type discriminant, the
command, and the matcher). `/hooks enable <id>` and `/hooks disable <id>` write
an `[overrides]` table into `<project>/.archon/hooks.local.toml`, and
`/hooks reload` re-reads all five sources.

An override is keyed by id, so it survives edits elsewhere in the file but not a
change to the hook's command — a new command is a new id, and the override no
longer applies to it.

Bare `/hooks` opens a browsable overlay of the same list: id, event, command,
source, and whether it is enabled. Enter on a row types the matching
`/hooks enable <id>` or `/hooks disable <id>` into the prompt and closes; you
press Enter again to run it. The overlay never writes `hooks.local.toml`
itself — the command does, and that is the one place that knows how.

The command column is there because the matcher usually is not distinguishing.
The first version of the overlay showed the matcher, and on a real project all
three hooks read `PostToolUse *` — three identical rows over three different
commands.

## What the runtime does with a hook's result

Every hook for an event runs and its result is merged into one
`AggregatedHookResult`. What happens to that aggregate depends entirely on the
event. Most events discard it.

| Event | Fields the runtime consumes |
|---|---|
| `PreToolUse` | `is_blocked` (tool call fails with the reason), `permission_behavior` (`deny` blocks; `allow` only from a `policy`-tagged hook; `ask` is logged and falls through to the normal flow), `updated_input` if it is a JSON object — the replacement is then re-validated against the tool's input schema. **`additional_context` is dropped.** |
| `PostToolUse` | `updated_mcp_tool_output` replaces the tool result; **`additional_context` is appended to the tool result**; `retry` re-runs the tool, up to 3 attempts; `prevent_continuation` + `stop_reason` end the turn after this call. |
| `FileChanged`, `CwdChanged` | `watch_paths` only. Everything else is discarded. |
| `SessionStart` | `watch_paths`, and **`additional_context` is injected into the system prompt** for the rest of the session, wrapped in `<hook-context>`. Re-fired by `/clear`. |
| `PostCompact` | **`additional_context`**, appended to whatever `SessionStart` established. This is what lets injected context survive a long session. |
| `PermissionRequest` | `updated_permissions` only. |
| `Elicitation` | `elicitation_action` + `elicitation_content` — the question is auto-answered without reaching the user. |
| `UserPromptSubmit` | Nothing. The call is made and the aggregate is dropped. |
| Everything else | Nothing. |

"Everything else" is most of the enum: `Setup`, `SessionEnd`, `Stop`,
`StopFailure`, `PostToolUseFailure`, `Notification`, `PreCompact`,
`ConfigChange`, `InstructionsLoaded`, `PermissionDenied`,
`SubagentStart`, `SubagentStop`, `TeammateIdle`, `TaskCreated`,
`TaskCompleted`, `ElicitationResult`, `WorktreeCreate`, `WorktreeRemove`, and
all twelve `Before*`/`After*` runtime lifecycle events. Those hooks are useful
for their side effects — writing a log, touching a file, posting to a service —
and for nothing else.

The runtime lifecycle events go further and say so out loud: if one of them
returns anything behaviour-changing (`is_blocked`, `updated_input`,
`updated_mcp_tool_output`, `updated_permissions`, `prevent_continuation`, or
`retry`), the call site logs `runtime lifecycle hook returned
behaviour-changing output; ignored` and continues
(`src/runtime/hooks.rs:92-105`, and the identical
`trace_ignored_runtime_hook_output` in
`crates/archon-core/src/agent/runtime_hooks.rs:158-174`). That warning in your
log is the system telling you the hook did nothing.

### PostToolUse is the only path back into the model

This is the single fact worth carrying away. `additional_context` from a
`PostToolUse` hook is joined with newline, appended to the tool result as

```
<original tool result>
---
[Hook Context]
<your text>
```

and that mutated result is what enters the transcript
(`crates/archon-core/src/agent/tool_postprocess_steps.rs`,
`apply_post_tool_aggregate`). The agent reads it on its next turn as an
observation about its own work, not as a message from a system it can argue
with.

Nothing else has this property. `PreToolUse` can stop a tool call and can
rewrite its input, but it cannot narrate — its `additional_context` is parsed,
merged into the aggregate, and then never read. If you want the model to *know*
something, the hook has to be on `PostToolUse`.

The corollary is that every byte a `PostToolUse` hook prints is transcript
budget spent on every matching tool call, forever. A self-check that prints
"looks fine" on each edit is a tax with no payer. Print only when there is
something to say.

## `matcher` does not filter

`HookMatcher.matcher` is parsed, is part of the hook id, and is shown by
`/hooks list` — and it is **never evaluated at execution time**. The code that
would apply it is a stub: `hooks/registry.rs:201-227` contains a placeholder
`if let` over a mapped empty string, a `let _ = matcher_str;` to silence the
unused warning, and three comment blocks explaining that the filter still needs
to be carried through `PendingHook`.

A hook configured with `matcher = "Bash"` therefore runs on **every** call of
that event, whatever the tool.

The filter that does work is `if_condition` on the individual `HookConfig`
(`hooks/condition.rs`, `evaluate`):

| Expression | Matches when |
|---|---|
| `""` or `"*"` | always |
| `"Read"` | `tool_name` equals `Read` exactly |
| `"Bash(git *)"` | `tool_name` is `Bash` **and** `tool_input.command` (or `tool_input.cmd`) matches the glob |

`*` is the only glob metacharacter; everything else is a literal. Both forms
read `tool_name` out of the event payload, so `if_condition` is only meaningful
on events whose payload carries one — the tool events do, and
`PostToolUse` deliberately includes `tool_input` as well so the `Tool(pattern)`
form has something to match against.

## Exit codes and stdout

For `command` and `agent` hooks (`hooks/executor.rs`, `interpret_exit_code`):

| Exit | Result |
|---|---|
| `0` | Success |
| `2` | Blocking. Trimmed stderr becomes the reason; if stderr is empty a generic message naming the command is used. |
| anything else | `NonBlockingError`, logged with the exit code and stderr. Execution continues. |

If stdout parses as a `HookResult` JSON object, its fields are overlaid on top
of that base result — each `Some`/non-empty field wins. The one exception is
`outcome`: on exit 2 the `Blocking` outcome is kept regardless of what stdout
claims, so a hook cannot un-block itself by printing JSON. Stdout that is not
valid `HookResult` JSON is not an error; it is logged at warn level and the
exit-code behaviour stands alone.

`prompt` hooks are different and simpler. Stdout is **not** JSON-parsed: on exit
0 the trimmed stdout becomes `additional_context` verbatim, empty output
produces no context at all, exit 2 blocks with stderr as the reason, and any
other code is a non-blocking error. This is the type to reach for when the hook
has something to tell the agent, because it removes the JSON envelope from a
script that only ever wanted to print a sentence.

`http` hooks POST the payload as JSON and interpret the response body the same
way (`hooks/http.rs`). A non-`https://` URL that is not localhost is rejected
before the request is made and the failure policy applies. `headers` values are
templates into which only the env vars named in `allowed_env_vars` are
interpolated, and the response body is capped at 64 KB. `function` hooks resolve
`command` against an in-process registry that ships with `noop` and
`block_all`; an unknown name warns and returns success.

## The process a command hook runs in

`hooks/executor_process.rs`, `run_command`:

- **stdin** receives the event payload as JSON, then the pipe is closed. This is
  the raw payload the call site built — the field set differs per event, and is
  not the `HookContext` type (that is for in-process callbacks only, despite
  what the doc comment on `HookContext` says).
- **Environment is cleared** and rebuilt from `archon_tools::bash::sanitized_env()`
  plus exactly three additions: `ARCHON_SESSION_ID`, `ARCHON_CWD`, and
  `ARCHON_HOOK_EVENT`. A hook cannot read an arbitrary variable out of the
  parent process; if it needs one, that is what `allowed_env_vars` is for on the
  HTTP path, and an explicit argument everywhere else.
- **Working directory** is the session's working directory.
- **Output is capped at 64 KiB total**, shared between stdout and stderr rather
  than 64 KiB each — the two drain tasks draw from one atomic budget. Whichever
  side overflows gets a `[hook output truncated at 65536 bytes]` marker, and the
  combined output stays inside the bound including the marker.
- **On timeout the whole process tree dies**, not just the shell: a process
  group killed with `SIGKILL` on Unix, a Job Object on Windows, with a 2-second
  deadline on reaping. A hook that backgrounds a child does not outlive its
  timeout.

## Timeouts and the aggregate budget

A single hook's `timeout` defaults to **60 seconds**. All hooks for one event
share an aggregate budget of **30 seconds**
(`HookExecutionConfig::aggregate_timeout_ms`), and each hook's timeout is
clamped down to whatever is left of it, with a floor of one second.

When the budget is exhausted, the remaining hooks are **not run**. Each one
still contributes a result — its failure policy applied with the reason
`aggregate timeout exhausted` — and `skipped_count` on the aggregate records how
many. The budget covers file-configured hooks, session-scoped hooks, and
in-process callbacks alike.

The test for that clamp spent four months passing on Windows without ever
starting a hook, because it asserted elapsed time rather than outcome — see
[postmortem 0002](../postmortem/0002-a-test-passed-on-the-one-platform-where-it-could-not-run.md).
The arithmetic now lives in `crates/archon-core/src/hooks/registry/budget.rs`
with unit tests that need no subprocess and no clock.

Failure policy is what happens when a hook cannot spawn, times out, or hits an
I/O error. It is set per hook with `on_failure = "allow" | "block"`. The default
depends on the event and there is only one gating event:

- `PreToolUse` defaults to **block**.
- Everything else defaults to **allow**.

That asymmetry is the reason to think about `on_failure` at all. A `PreToolUse`
guard that cannot run has failed open if it allows, which defeats the point of
having it; every other event is observational, and a broken logging hook should
not stop the session.

`async = true` spawns the command in the background and returns success
immediately — but only when the effective failure policy is `allow`, and only
for plain `command` hooks. A hook whose failure blocks must finish before the
guarded operation proceeds, so `async` is silently ignored for it.

## Other execution details worth knowing

- **`once = true`** fires the hook at most once per process, keyed on
  `event:source:command`. It is tracked in a set, not removed from the registry,
  and the key does not include the hook type or matcher.
- **Agent hooks are serialised and recursion-guarded.** Only one `agent` hook
  runs at a time (a global mutex), and while one is running, *all* hook
  execution on that thread is skipped and `execute_hooks` returns an empty
  aggregate. Without this, an agent hook that used a tool would fire the tool
  hooks that fire the agent hook.
- **Session-scoped hooks** can be registered at runtime for one session id and
  are cleared automatically when `SessionEnd` fires for it. They run after the
  file-configured hooks and carry no source authority, so they can never grant
  `allow`.
- **A panicking or timing-out in-process callback is treated as success.**

## In-process callbacks are not external hooks

Plugins and extensions can register a Rust callback for an event instead of a
command. The two see different things.

A command or HTTP hook receives the **full event payload** as built by the call
site. An in-process callback receives a `HookContext`, which is a fixed, much
narrower struct: event, session id, cwd, timestamp, permission mode,
conversation turn, and the tool name / input / output when the payload had them.
Anything else in the payload is not visible to it.

One field is worth calling out because it was recently added:
`HookContext.agent_id` is now populated from the payload's `subagent_id`
(`hooks/registry.rs`, at the end of `execute_hooks`). Before that a
`SubagentStop` callback could tell that *a* subagent had stopped but not which
one — command hooks could always read `subagent_id` straight out of the JSON.

Function hooks (`type = "function"`) build their own, even thinner context: no
`tool_output` and no `agent_id`.

## Declared but not wired

Two `HookConfig` fields parse and are never read by anything:

- **`async_rewake`** — intended to re-wake the agent after a background hook
  finishes. There is no such mechanism; the field appears only in its own
  declaration and in test fixtures.
- **`status_message`** — the `HookConfig` field is dead. Status messages that
  do reach the TUI log come from `HookResult.status_message`, which a hook
  returns on stdout.

Also incomplete: `permission_behavior: "ask"` from a `PreToolUse` hook is logged
and then falls through to the normal permission flow, rather than forcing a
prompt.

## A worked example

This is the repository's own `.archon/hooks.toml`, which wires a per-edit file
size check:

```toml
[hooks.PostToolUse]
matchers = [
  { hooks = [
    { type = "prompt", command = "bash scripts/self-check-file.sh Edit", if_condition = "Edit", timeout = 5 },
    { type = "prompt", command = "bash scripts/self-check-file.sh Write", if_condition = "Write", timeout = 5 },
    { type = "prompt", command = "bash scripts/self-check-file.sh NotebookEdit", if_condition = "NotebookEdit", timeout = 5 },
  ] },
]
```

Every choice in it is forced by something above:

- **`PostToolUse`**, because the whole point is for the agent to read the
  finding. On any other event the script would run and its output would go
  nowhere.
- **`type = "prompt"`**, because the script wants to print a sentence on
  violation and nothing on success. A `command` hook would have to emit
  `{"additional_context": "..."}` JSON to achieve the same thing.
- **`if_condition`, not `matcher`**, because `matcher` does not filter. Without
  these three conditions the check would run after every `Bash`, `Read`, and
  `Grep` call as well.
- **Three distinct command strings**, because dedupe is on `(type, command)`.
  Three entries reading `bash scripts/self-check-file.sh` would collapse into
  one, and only the last-loaded would survive.
- **`timeout = 5`**, because the default is 60. The check takes roughly 0.4
  seconds; anything near 60 means something is wrong and the turn should not be
  waiting on it.
- **`on_failure` left at its default**, which for a non-`PreToolUse` event is
  `allow`. A self-check must never block a turn.

The script itself exits 0 unconditionally and prints only when the invariant is
actually broken.

## See also

- [Configuration](config.md) — config file precedence
- [Slash commands](slash-commands.md) — `/hooks`
- [Plugins](../integrations/plugins.md) — heavier extensions that register tools and skills
