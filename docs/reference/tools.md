# Tools reference

Tools are callable by the LLM during agent turns. The core registry is built in
`crates/archon-core/src/dispatch.rs`, and session setup conditionally adds
tools such as LEANN, memory, and verbosity controls when those subsystems are
enabled. Use `/tools` or `ToolSearch` for the live tool surface in a running
session.

Permission level is per-tool:
- **Safe** — auto-approved by default
- **Risky** — prompts in `default` mode
- **Variable** — context-dependent (Bash/PowerShell classify per-command via `archon_permissions::classifier`)

## File & code

| Tool | Permission | Purpose |
|---|---|---|
| `Read` | Safe | Read files with pagination (image/PDF, Jupyter notebooks supported) |
| `Write` | Risky | Write small/new files. Refuses large full-file rewrites of existing files; use `LargeEdit*` for those. |
| `Edit` | Risky | Exact-string replacement edits |
| `LargeEditBegin` | Risky | Start a transactional staged edit for an existing large file |
| `LargeEditInsertAfter` | Risky | Insert a content chunk after an anchor line in the staged file |
| `LargeEditReplaceSection` | Risky | Replace a staged section by start anchor and optional end anchor |
| `LargeEditDeleteSection` | Risky | Delete a staged section by start anchor and optional end anchor |
| `LargeEditCommit` | Risky | Atomically commit the staged file if the original file hash still matches |
| `LargeEditAbort` | Risky | Remove a staged large edit without changing the target file |
| `ApplyPatch` | Risky | Apply a unified-diff patch to an absolute file path |
| `Glob` | Safe | Fast file pattern matching (sorted by mtime) |
| `Grep` | Safe | Ripgrep-backed regex search (content / files-with-matches / count modes) |
| `Bash` | Variable | Execute shell command (classified at dispatch) |

Large file edits should follow this protocol:

```text
LargeEditBegin -> LargeEditReplaceSection/InsertAfter/DeleteSection -> LargeEditCommit
```

`LargeEditCommit` compares the target file's current hash to the hash captured
by `LargeEditBegin`. If another process changed the file, commit fails and the
staged copy remains available for review or `LargeEditAbort`.

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
| `AgentCatalog` | Safe | List, search, and inspect available subagent types. Used to discover long-tail agents that are not declared inline in the `Agent` tool description |
| `SendMessage` | Safe | Send a follow-up message to a running subagent by ID or name |
| `AskUserQuestion` | Safe | Blocking user confirmation (structured choices) |

> *In `default` permission mode the `Agent` tool is auto-approved (PRD-AGENTS-001 Option B); dangerous downstream tools (Bash/Write/Edit) inherit gating from the parent's mode. See [Permissions](permissions.md).

For mutating subagent tasks, pass `expected_target_files` with the files that
must change. Foreground subagents are snapshot-hashed before launch and the
Agent result fails if those files are unchanged after completion. Explicit
background subagents reject `expected_target_files` because Archon cannot
verify the mutation before returning.

## Plan Mode safe tools

Plan Mode uses the canonical production **Plan-safe allowlist** in
`archon_tools::plan_mode::PLAN_MODE_SAFE_TOOLS`. Every tool outside this exact
set is denied while Plan Mode is active; future exceptions require an explicit
production allowlist or configuration change. Plan Mode blocks working-tree mutations by default while retaining explicit process-state controls: `TaskCreate`, `TaskUpdate`, and `Agent`. Agent model/tool actions remain subject to Plan Mode and preflight boundaries. The retained tools are:

| Tool | Plan Mode effect |
|---|---|
| `Read` | Read file, image, PDF, or notebook content without changing it. |
| `Glob` | Find paths matching a pattern without changing the working tree. |
| `Grep` | Search file content without changing it. |
| `AskUserQuestion` | Request structured user input; it does not mutate project files. |
| `EnterPlanMode` | Request Plan Mode entry; only the trusted runtime may execute it. |
| `ExitPlanMode` | Submit the plan for approval; approval materializes plan-linked tasks, while rejection or revision keeps mutations blocked. |
| `TaskCreate` | Create a tracked task. With a `prompt`, it may run or spawn a subagent; that subagent's tools remain subject to its own permission gates. |
| `TaskUpdate` | Update tracked-task metadata or status; it does not edit project files. |
| `TaskGet` | Read one tracked task. |
| `TaskList` | List tracked tasks. |
| `DocList` | List the compact document inventory. |
| `DocGet` | Read compact document metadata. |
| `DocStatus` | Read document processing or indexing status. |
| `DocSearch` | Search indexed document chunks. |
| `DocAnswer` | Answer from document evidence with citations. |
| `DocProvenance` | Trace document or chunk provenance. |
| `DocInspect` | Inspect document pages, chunks, OCR runs, and provenance. |
| `DocModelStatus` | Read embedding backend and vector status. |
| `GameTheoryStatus` | Read persisted game-theory run status. |
| `GameTheoryListAgents` | List curated game-theory specialists. |
| `GameTheoryInspect` | Inspect a game-theory run, fingerprint, routing, specialist, section, or report artifact. |
| `LearningStatus` | Read governed-learning status and proposal counts. |
| `LearningInspect` | Inspect a learning event, behaviour proposal, or manifest. |
| `BehaviourProposals` | List pending behaviour proposals; it neither approves nor rolls them back. |
| `Agent` | Spawn a subagent subject to its own permission gates; subagents cannot enter or exit Plan Mode. |

The allowlist includes no mutation-capable tools: `Write`, `Edit`, `Bash`,
`BehaviourApprove`, `BehaviourRollback`, and all other tools outside the
canonical set are denied; mutating tools remain denied. Other task and agent controls remain denied unless explicitly allowlisted by a future production change. This includes `TaskStop`,
`TaskOutput`, `AgentCatalog`, and `SendMessage`; they are not Plan-safe merely
because they may be observational in some contexts.

## Planning & isolation

| Tool | Permission | Purpose |
|---|---|---|
| `EnterWorktree` | Risky | Create an isolated git worktree for the current session |
| `ExitWorktree` | Risky | Exit worktree (`merge` / `keep` / `remove`) |

## Plan Mode completion evidence

Approved plan steps can carry required evidence kinds. A task cannot complete unless its recorded evidence passes `check_required_evidence`; evidence requirements block completion rather than treating a terminal task state as proof. Durable reconciliation classifies plan work as completed, omitted, or deviated and records changed unplanned paths as unplanned-extra.


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

## Evidence documents

| Tool | Permission | Purpose |
|---|---|---|
| `DocIngest` | Risky | Ingest a file or directory into the document evidence store |
| `DocList` | Safe | List a compact document inventory |
| `DocGet` | Safe | Show compact document metadata by id |
| `DocStatus` | Safe | Show document processing/indexing status |
| `DocSearch` | Safe | Search chunks with exact, semantic, or hybrid retrieval |
| `DocAnswer` | Safe | Answer from document evidence with citations |
| `DocProvenance` | Safe | Trace document/chunk provenance |
| `DocInspect` | Safe | Inspect pages, chunks, OCR runs, and provenance for a document |
| `DocModelStatus` | Safe | Show embedding backend/vector status |

`DocList`, `DocGet`, `DocSearch`, and `DocAnswer` run through Archon's
in-process document runtime during agent turns, so normal chat retrieval does
not spawn a fresh `archon` child process for every question. `DocList` is capped
to a compact inventory by default; agents should use `DocSearch` for content
questions instead of dumping the corpus into the model context.

## Game theory

| Tool | Permission | Purpose |
|---|---|---|
| `GameTheoryRun` | Risky | Run the game-theory pipeline and persist run state |
| `GameTheoryStatus` | Safe | Read persisted run status |
| `GameTheoryListAgents` | Safe | List curated game-theory specialists |
| `GameTheorySpecimens` | Safe | List or ingest the known-fingerprint specimen library |
| `GameTheoryInspect` | Safe | Inspect run, fingerprint, routing, specialist, section, or report artefacts |
| `GameTheoryReplay` | Risky | Replay routing, reclassification, or a specialist from stored state |
| `GameTheoryClassify` | Risky | Persist a Tier 1 fingerprint for a situation |
| `GameTheoryCallSpecialist` | Risky | Re-run one specialist against a stored fingerprint |

## Governed learning

| Tool | Permission | Purpose |
|---|---|---|
| `LearningStatus` | Safe | Show governed-learning status and proposal counts |
| `LearningInspect` | Safe | Inspect a learning event, behaviour proposal, or manifest |
| `BehaviourProposals` | Safe | List pending behaviour proposals |
| `BehaviourApprove` | Risky | Approve and apply a proposal |
| `BehaviourRollback` | Risky | Roll back to a previous manifest version |

## Code intelligence

| Tool | Permission | Purpose |
|---|---|---|
| `lsp` | Safe | LSP dispatch: `goToDefinition`, `findReferences`, `hover`, `documentSymbol`, `workspaceSymbol`, `goToImplementation`, `prepareCallHierarchy`, `incomingCalls`, `outgoingCalls`. Returns empty when no language server is connected. |
| `CartographerScan` | Safe | Index a codebase for symbols (Rust, Python, TS, JS, Go, Java) |
| `LeannSearch` | Safe | Semantic code search via HNSW over embeddings. Conditionally registered when the LEANN index is available at startup. |
| `LeannFindSimilar` | Safe | Find similar code chunks. Conditionally registered when the LEANN index is available. |
| `JavaToolchain` | Safe (`detect`, `report`) / Risky (`compile`, `analyze`, `test`) | Drive a Java project's Gradle or Maven build and read its analysis reports. See [Java support](../integrations/java.md). |

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
| `TeamCreate` | Safe | Establish the session's team: declare the roles it wants. Does not spawn agents |
| `TeamDelete` | Risky | Shut the team down — `shutdown_request` to every member, then remove it |

A team's roster lives at `<project>/.archon/teams/<team-id>/team.json`. Spawning an
agent whose `subagent_type` matches a declared role seats it on the team; the seat
is vacated when the agent reaches any terminal state. Members address each other by
role with `SendMessage`, and what they receive is attributed to the sender's role.
`/agent` shows the roster with each member's status, task and declared writes, and
`archon team list` shows every team on the project.

`TeamDelete` waits up to 60s for members to stop. A member that does not stop leaves
the team intact and is named in the refusal — a half-deleted team is worse than one
that is still there. One team per session: creating a second while members are
running is refused.

## Runtime control

| Tool | Permission | Purpose |
|---|---|---|
| `RemoteTrigger` | Risky | HTTP POST to an allow-listed remote endpoint (`remote_triggers.allowed_hosts`) |
| `SessionSearch` | Safe | Search past sessions by text, directory, branch or date; returns ids, names, timestamps and the matching excerpt |
| `Sleep` | Safe | Async-safe delay (max 300s) |

`SessionSearch` and `/sessions` read the same store and resolve its path the
same way — `ARCHON_SESSION_DB_PATH`, then `session.db_path`, then the default
location. They differ only in shape: the command renders a table for a person,
the tool returns structured rows for the model. Results are capped at 50 and the
whole payload is bounded; when rows are dropped to stay inside that bound the
response says how many, because a silently shortened list reads as a complete
one.

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
