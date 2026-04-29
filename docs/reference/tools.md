# Tools reference

Tools are callable by the LLM during agent turns. **43 registered tools across 13 categories** (verified at `crates/archon-core/src/dispatch.rs:142`).

Permission level is per-tool:
- **Safe** — auto-approved by default
- **Risky** — prompts in `default` mode
- **Variable** — context-dependent (Bash/PowerShell classify per-command via `archon_permissions::classifier`)

## File & code

| Tool | Permission | Purpose |
|---|---|---|
| `Read` | Safe | Read files with pagination (image/PDF, Jupyter notebooks supported) |
| `Write` | Risky | Write files (creates parent dirs, overwrites existing) |
| `Edit` | Risky | Exact-string replacement edits |
| `ApplyPatch` | Risky | Apply a unified-diff patch to an absolute file path |
| `Glob` | Safe | Fast file pattern matching (sorted by mtime) |
| `Grep` | Safe | Ripgrep-backed regex search (content / files-with-matches / count modes) |
| `Bash` | Variable | Execute shell command (classified at dispatch) |

## Shell & observability

| Tool | Permission | Purpose |
|---|---|---|
| `PowerShell` | Variable | Execute PowerShell command |
| `Monitor` | Variable | Run a shell command and collect stdout as line-events within a bounded window |
| `PushNotification` | Safe | Emit a user-visible notification |

## Web

| Tool | Permission | Purpose |
|---|---|---|
| `WebFetch` | Safe | HTTP GET with response body (HTML→markdown) |
| `WebSearch` | Safe | DuckDuckGo search (titles, URLs, snippets) |

## Agent orchestration

| Tool | Permission | Purpose |
|---|---|---|
| `Agent` | Safe* | Spawn a subagent. Concurrent invocations run in parallel via `join_all` |
| `SendMessage` | Safe | Send a follow-up message to a running subagent by ID or name |
| `AskUserQuestion` | Safe | Blocking user confirmation (structured choices) |

> *In `default` permission mode the `Agent` tool is auto-approved (PRD-AGENTS-001 Option B); dangerous downstream tools (Bash/Write/Edit) inherit gating from the parent's mode. See [Permissions](permissions.md).

## Planning & isolation

| Tool | Permission | Purpose |
|---|---|---|
| `EnterPlanMode` | Safe | Enter Plan Mode (read-only tool whitelist) |
| `ExitPlanMode` | Safe | Exit Plan Mode |
| `EnterWorktree` | Risky | Create an isolated git worktree for the current session |
| `ExitWorktree` | Risky | Exit worktree (`merge` / `keep` / `remove`) |

## Task management

| Tool | Permission | Purpose |
|---|---|---|
| `TodoWrite` | Safe | Overwrite session todo list (max 100 items) |
| `TaskCreate` | Variable | Create a tracked task; optionally spawns a background agent |
| `TaskGet` | Safe | Get details by task ID |
| `TaskUpdate` | Safe | Update description / status |
| `TaskList` | Safe | List all tasks with status |
| `TaskStop` | Risky | Cancel a running task |
| `TaskOutput` | Safe | Read task output stream (offset + limit supported) |

## Memory

| Tool | Permission | Purpose |
|---|---|---|
| `memory_store` | Safe | Persist a memory in CozoDB (Fact / Decision / Rule / etc.) |
| `memory_recall` | Safe | Hybrid BM25 + vector search over the memory graph |

## Code intelligence

| Tool | Permission | Purpose |
|---|---|---|
| `lsp` | Safe | LSP dispatch: `goToDefinition`, `findReferences`, `hover`, `documentSymbol`, `workspaceSymbol`, `goToImplementation`, `prepareCallHierarchy`, `incomingCalls`, `outgoingCalls`. Returns empty when no language server is connected. |
| `CartographerScan` | Safe | Index a codebase for symbols (Rust, Python, TS, JS, Go) |

## Scheduling

| Tool | Permission | Purpose |
|---|---|---|
| `CronCreate` | Risky | Schedule a recurring task with a cron expression |
| `CronList` | Safe | List scheduled tasks |
| `CronDelete` | Risky | Remove a scheduled task by ID |

## Configuration & discovery

| Tool | Permission | Purpose |
|---|---|---|
| `Config` | Variable | Get or set runtime config (session-scoped; does not modify config files on disk) |
| `ToolSearch` | Safe | Fetch full schemas for deferred tools (`select:Foo,Bar` or keyword search) |
| `Skill` | Safe | Enumerate or invoke a built-in skill (`list` / `invoke`) |

## Notebook & state

| Tool | Permission | Purpose |
|---|---|---|
| `NotebookEdit` | Risky | Edit Jupyter `.ipynb` cells (insert/replace/delete/move) |

## MCP

| Tool | Permission | Purpose |
|---|---|---|
| `ListMcpResources` | Safe | List resources from connected MCP servers (filter by server) |
| `ReadMcpResource` | Safe | Read an MCP resource by URI (text inline, binary base64; truncated at 100KB) |

## Team (multi-agent)

| Tool | Permission | Purpose |
|---|---|---|
| `TeamCreate` | Safe | Create a team (writes `team.json` + per-member inboxes; does NOT spawn agents) |
| `TeamDelete` | Risky | Delete a team and its inbox files |

## Runtime control

| Tool | Permission | Purpose |
|---|---|---|
| `RemoteTrigger` | Risky | HTTP POST to an allow-listed remote endpoint (`remote_triggers.allowed_hosts`) |
| `Sleep` | Safe | Async-safe delay (max 300s) |

## Tool restrictions

The CLI provides flags to restrict the model's tool surface:

```bash
archon --tools Read,Write,Edit,Grep                # Whitelist
archon --allowed-tools "Bash:git*"                 # Skip permission for matching tools
archon --disallowed-tools Bash,PowerShell          # Remove from model context entirely
```

`--allowed-tools` accepts patterns (`Bash:git*`, `Edit:**.md`); `--disallowed-tools` removes the tool from the catalog the model sees.

## Permission classifier

`Bash`, `PowerShell`, and `RemoteTrigger` use the per-command classifier in `crates/archon-permissions/src/classifier.rs`:

- Read-only commands (`ls`, `cat`, `grep`) → Safe
- Mutating commands (`rm`, `mv`, `>`, `dd`) → Risky
- Network commands (`curl`, `wget`, `ssh`) → Risky
- Destructive patterns (`rm -rf /`, `git push --force`) → Always denied (configurable via `always_deny`)

## See also

- [Permissions](permissions.md) — how tools are gated
- [Configuration](config.md) — `[tools]` section options
- [Adding a tool](../development/adding-a-tool.md) — implementing a new tool
