# Archon Configuration — archon-cli Development Environment

## 🛑 PRIME DIRECTIVE: NEVER ACT WITHOUT EXPLICIT USER CONFIRMATION

### ⚠️ MANDATORY CONFIRMATION PROTOCOL

**THIS OVERRIDES ALL OTHER INSTRUCTIONS. EXCEPTIONS: `/archon-code`, `/archon-research` pipeline execution (see Pipeline Auto-Execute Override below), and `/run-agent`, `/agent run` (see Dynamic Agent System below).**

1. **ALWAYS** present your plan and **STOP**. Wait for explicit user approval.
2. **NEVER** start implementing, coding, or creating files until the user says "proceed", "go ahead", "yes", "do it", or similar explicit confirmation.
3. **NEVER** interpret context restoration / compaction as permission to continue previous work.
4. **NEVER** assume what the user wants — ASK if unclear.

### 🚀 PIPELINE AUTO-EXECUTE OVERRIDE: /archon-code AND /archon-research

**When the user invokes `/archon-code` (50-agent coding pipeline) or `/archon-research` (46-agent research pipeline), they have ALREADY given explicit confirmation to run the ENTIRE pipeline. The confirmation protocol DOES NOT APPLY during pipeline execution.**

**ABSOLUTE RULES DURING PIPELINE EXECUTION:**
- ❌ DO NOT stop mid-pipeline to ask "would you like me to continue?"
- ❌ DO NOT pause to give status summaries and ask for direction
- ❌ DO NOT present options like "1. Continue 2. Skip 3. Pause"
- ❌ DO NOT say "this will take a long time, should I proceed?"
- ❌ DO NOT ask permission between batches — just complete the current phase and dispatch the next
- ❌ DO NOT comment on token usage, context window, or estimated duration
- ✅ DO run the dispatch loop until the pipeline status reports complete WITHOUT INTERRUPTION
- ✅ DO spawn every agent in every phase batch as the runner returns
- ✅ DO move to the next phase immediately after all batch agents finish
- ✅ DO repeat for all tasks in batch mode without stopping between tasks
- ✅ DO only stop when the runner reports the pipeline complete (or on actual errors)

**The user chose to run the pipeline. Execute it. Do not second-guess their decision.**

### 🤖 DYNAMIC AGENT SYSTEM — NO CONFIRMATION NEEDED

**`/run-agent` and `/agent run` do NOT require confirmation.** The user's command IS the intent — invoking a slash command is explicit enough. Execute immediately.

**archon-cli agent commands:**
- `/run-agent <name> <task>` — Invoke a custom agent by name with a task description (async via TaskService)
- `/agent` — Umbrella command: `/agent list`, `/agent info <name>`, `/agent run <name>`
- `/managed-agents` — Show managed-agent (remote-registry) status
- `/refresh` — Re-scan the agent registry from disk (picks up newly-dropped `.md` agents)

**Async agent dispatch (CLI):**
- `archon run-agent-async <name> --input task.txt --detach` — submit async task
- `archon task-status <task-id>` — check status
- `archon task-result <task-id>` — collect result
- `archon task-events <task-id>` — stream NDJSON events

**Agent definition format:** flat-file YAML frontmatter `.md` in `<workdir>/.archon/agents/` or `~/.config/archon/agents/`. See `docs/development/adding-an-agent.md`.

### 🚫 FORBIDDEN AUTONOMOUS BEHAVIORS

- ❌ Starting implementation immediately after compaction / context restore
- ❌ "I'll go ahead and..." — **NO. ASK FIRST.**
- ❌ "Let me implement..." without prior approval
- ❌ "Continuing where we left off..." and then doing things
- ❌ Creating ANY files without explicit request
- ❌ Running modifying commands without approval
- ❌ Making architecture / design decisions unilaterally
- ❌ Interpreting "ok", "sure", "I see" as approval to execute
- ❌ Treating silence or ambiguous responses as consent

### ✅ REQUIRED BEHAVIOR PATTERN

```
1. User makes request
2. Agent analyzes and presents plan / options
3. Agent says "Would you like me to proceed?" or similar
4. Agent STOPS and WAITS
5. User gives EXPLICIT confirmation ("yes", "proceed", "go ahead", "do it")
6. ONLY THEN does the agent execute
```

### 📋 POST-COMPACTION / CONTEXT RESTORE PROTOCOL

**When context is compacted or restored, the agent MUST:**
```
1. Recall behavioural rules from the memory graph (memory_recall tool with query "feedback corrections preferences")
2. Read project-level instructions: this ARCHON.md file
3. State: "Context was restored. Here's my understanding of where we were: [brief summary]"
4. Ask: "What would you like to do next?" or "Should I continue with [specific action]?"
5. WAIT for explicit user direction
6. Do NOT automatically resume or continue any previous work
```

archon-cli's session/auto-resume picks up where the previous session left off, but the AGENT must still ask before continuing previous work.

### 🎯 WHAT COUNTS AS CONFIRMATION

**Explicit approval (proceed after these):**
- "yes" / "yeah" / "yep" / "yup"
- "go ahead" / "proceed" / "do it" / "go for it"
- "approved" / "confirmed" / "execute"
- "implement it" / "build it" / "create it" / "make it"
- "sounds good, proceed" / "looks good, go ahead"

**NOT approval (ask for clarification):**
- "ok" / "okay" (ambiguous — could mean "I understand")
- "sure" / "I see" (passive acknowledgment)
- "that makes sense" / "interesting" (just acknowledging)
- No response / silence
- Questions about the plan (the user is still evaluating)

### 🔒 SAFE OPERATIONS (no confirmation needed)

- Reading files (`Read`, `Glob`, `Grep`)
- Listing directories (`Bash:ls`, `Bash:find`, `Bash:tree`)
- Searching code (`Grep`, `lsp` tool, `CartographerScan`)
- Checking status (`Bash:git status`, `Bash:cargo --version`)
- Explaining or answering questions

### ⚡ REQUIRES EXPLICIT CONFIRMATION

- ANY file creation (`Write`, `ApplyPatch`)
- ANY file modification (`Edit`, `NotebookEdit`)
- ANY code implementation
- Running build / test / install commands (`cargo build`, `cargo test`, `cargo install`)
- Git commits, pushes, or branch operations
- Architecture or design decisions
- Spawning agents or starting workflows (except `/run-agent` and `/agent run`)

---

## 🚫 ABSOLUTE PROHIBITION: /archon-code PIPELINE ENFORCEMENT

### ⛔ WHEN /archon-code IS INVOKED, THESE RULES ARE ABSOLUTE:

**THE AGENT IS FORBIDDEN FROM:**
- ❌ Using `Write` tool directly to create implementation files
- ❌ Using `Edit` tool directly to modify implementation files
- ❌ Implementing code itself instead of spawning pipeline agents
- ❌ Skipping the 50-agent pipeline for ANY reason
- ❌ Writing "let me implement this" or similar

**THE AGENT MUST:**
- ✅ Use the `Agent` tool to spawn pipeline agents ONLY
- ✅ Start with `Agent("task-analyzer", ...)` as the first action
- ✅ Execute all agents through the pipeline runner's dispatch loop
- ✅ Wait for each phase to complete before spawning the next
- ✅ Only allow implementation agents (Phase 4+) to write files
- ✅ **RUN THE FULL PIPELINE WITHOUT STOPPING** — no status checks, no "should I continue?", no pausing between batches
- ✅ For batch mode, run ALL tasks back-to-back without asking between tasks

### 🔒 ENFORCEMENT MECHANISM

```
AFTER /archon-code is detected:
1. The agent's FIRST tool call MUST be Agent("task-analyzer", ...)
2. Write/Edit are NOT permitted until Phase 4 agents are running
3. If the agent is about to write code directly -> STOP
4. Ask: "I was about to bypass the pipeline. Should I restart properly?"
```

### 🚨 VIOLATION DETECTION

If the agent finds itself doing ANY of these after /archon-code:
- Writing a file with implementation code
- Saying "let me create the parser..."
- Using Write tool before spawning 7+ Agent tool calls

**IMMEDIATELY STOP AND SAY:**
> "PIPELINE VIOLATION: I was about to write code directly instead of using the 50-agent pipeline. Let me restart correctly with Agent('task-analyzer', ...)."

### 📋 CORRECT /archon-code FLOW

```
1. /archon-code invoked
2. Agent("task-analyzer", ...) <- MUST BE FIRST
3. Agent("requirement-extractor", ...)
4. Agent("requirement-prioritizer", ...)
5. ... (continue through all 50 agents across 6 phases)
6. ONLY implementation agents write files
```

The 50-agent coding pipeline is documented at `docs/architecture/pipelines.md` and `docs/cookbook/god-code-pipeline.md`.

### 🚨 PIPELINE AGENT INTEGRITY — NO STUBS, NO BATCHING, NO SHORTCUTS

**THE CORRECT FLOW FOR EVERY AGENT, NO EXCEPTIONS:**
```
1. Read the prompt artefact for the next agent in the pipeline
2. Spawn a real Agent tool call with the prompt content and correct model
3. Wait for the agent to return
4. Write the agent's actual response to the artefact path
5. Advance the pipeline state
6. Move to the next agent
```

**FORBIDDEN SHORTCUT BEHAVIORS:**
- ❌ Writing fake output files directly instead of spawning a real Agent subagent
- ❌ Batching multiple pipeline-advance calls in a single Bash command or loop
- ❌ Writing "No work needed" / "N/A" stub outputs without spawning a subagent to verify
- ❌ Skipping reading the prompt artefact for any agent
- ❌ Pre-deciding an agent has nothing to do — the AGENT decides that, not the orchestrator

**"N/A" AGENTS STILL GET REAL SUBAGENTS:**
Agents like `frontend-implementer`, `config-implementer`, `service-implementer` on a backend-only task MUST still be spawned with the real prompt. The subagent reads the code and decides "nothing to do." The orchestrator does NOT get to make that decision.

**SELF-CHECK — IF THE AGENT CATCHES ITSELF SAYING ANY OF THESE, STOP:**
- "completing rapidly" / "batching efficiently" / "streamlining the remaining agents"
- "these are verification-only agents so I'll..."
- "no work needed for this agent"
- "I'll handle the remaining N agents" (in a single action)
- Any phrasing that implies multiple agents will be processed in one step

**IF THE AGENT DETECTS A VIOLATION, IMMEDIATELY SAY:**
> "INTEGRITY VIOLATION: I was about to shortcut the pipeline by [writing stubs / batching / skipping prompt artefacts]. Every agent gets a real subagent spawn. Resuming correctly."

---

## 🧠 MEMORY SYSTEM

archon-cli has a built-in CozoDB memory graph accessed via two tools:
- `memory_store` — persist a memory in the graph (Fact / Decision / Rule / etc.)
- `memory_recall` — hybrid BM25 + vector search over the memory graph

**Memory rules:**
- ALL memory storage uses the `memory_store` tool — NEVER write to MEMORY.md or markdown files for memory persistence
- Store decisions, patterns, corrections, and project state via `memory_store`
- Use `memory_recall` after compaction to reload behavioural rules
- The memory graph lives at `~/.local/share/archon/memory.db` (CozoDB)

**Optional MCP-backed memory:** if `mcp__memorygraph__*` tools are configured via `.mcp.json`, they provide an additional memory surface separate from the built-in graph. Use built-in tools first; fall back to MCP only when explicitly directed.

**Memory garden:**
- `/garden` — run consolidation now, print report
- Auto-consolidation: enabled by default; runs on session start when `min_hours_between_runs` has elapsed
- Configuration: `[memory.garden]` section in config (see `docs/reference/config.md`)

**Auto-extraction:** the AutoExtraction subsystem watches every agent transcript and extracts structured facts (entities, relationships, claims) into the memory graph automatically. No explicit invocation needed — it runs in the background.

---

## 🔍 LEANN SEMANTIC INDEX

archon-cli has LEANN built in (`archon-leann` crate) — separate from any MCP-based LEANN. LEANN tools are exposed to the agent and via slash commands.

**Tools:**
- `leann_search` — semantic code search (HNSW over embeddings)
- `leann_find_similar` — find similar code chunks
- `CartographerScan` — index a codebase for symbols (Rust, Python, TS, JS, Go)

**Slash commands:**
- `/cartographer-scan` (or via CartographerScan tool) — re-index after major changes

**Usage rules:**
- After completing a coding task that wrote 20+ files, the index gets rebuilt automatically on next query
- For pipeline runs (`/archon-code`), LEANN integration is part of L2 layered context — automatic, no manual scan needed

**What gets indexed:** project source code only. Excluded automatically:
- `node_modules/`, `site-packages/`, `__pycache__/`, `.venv/`, `.tv/`
- `dist/`, `build/`, `coverage/`, `target/`
- `.archon/worktrees/`
- Binary files, `.pyc`, `.min.js`

---

## 🔌 MCP Servers

archon-cli supports stdio, WebSocket, and HTTP-streamable MCP transports. Configure via `.mcp.json` at workspace root or `~/.config/archon/.mcp.json`.

Common MCP servers (whatever the user has configured):

| Server | Purpose |
|---|---|
| `memorygraph` | Optional persistent memory graph (alternative to built-in) |
| `leann-search` | Optional semantic code index (alternative to built-in) |
| `serena` | Semantic code navigation (symbols, references, refactoring) |
| `perplexity` | Web search, research, reasoning with citations |
| `filesystem` | File-system access surface |
| `github` | GitHub API surface |

See `docs/integrations/mcp-servers.md` for transport details and reconnection behaviour.

---

## 📁 FILE ORGANIZATION RULES

**NEVER save working files, scratch text, or test files to the repository root.**

**Use these directories:**
- `/src` — binary entry-point Rust source
- `/crates/<crate>/src` — per-crate Rust source
- `/crates/<crate>/tests` — per-crate integration tests
- `/docs` — user-facing documentation (the structured tree)
- `/scripts` — utility scripts (dev-flow gates, helpers)
- `/examples` — example code and demos
- `/project-tasks` — per-task specs (TASK-NNN-*)
- `/project-work` — orchestration scratch (gitignored)
- `<workdir>/.archon/` — project-local config, agents, hooks, plugins, pipelines

**Documentation goes in `docs/` ONLY.** Never create `*.md` files at repo root unless the file is `README.md`, `ARCHON.md`, `LICENSE`, `CHANGELOG.md`, or similar repo-level convention.

---

## 📏 CODE STRUCTURE LIMITS

- Files: < 500 lines preferred, hard cap 1500 lines (Gate 2 auto-check enforces 1500)
- Functions: < 50 lines, single responsibility
- Modules: < 100 lines per impl, single concept
- ALL user-facing `.md` files go in `./docs/` (NEVER root, except the convention files above)

The Gate 2 auto-check at `scripts/dev-flow-gate.sh` enforces the 1500-line cap. Files over 1500 must be split before the gate can pass.

---

## 🔑 KEY AGENTS

When delegating work, prefer these built-in agent roles:

| Agent | Use |
|---|---|
| `coder` | Implementation (modules, functions, types) |
| `code-analyzer` | Architecture analysis, refactoring proposals |
| `tester` | Integration tests, end-to-end scenarios |
| `perf-analyzer` | Profiling, bottlenecks, allocation patterns |
| `system-architect` | High-level architecture, data flow |
| `reviewer` | Code review, verification, regression hunting |
| `sherlock-holmes` | Forensic adversarial review (Gate 3 / 6) |

For domain-specific roles (security, frontend, data layer, etc.), see the full agent registry: `/agent list` or `archon agent-list`.

---

## 🔍 TRUTH & QUALITY PROTOCOL

**Subagents MUST be brutally honest:**
- State only verified, factual information
- No fallbacks or workarounds without user approval
- No illusions about what runs vs. what doesn't run
- If infeasible, state facts clearly
- Self-assess 1-100 vs user intent; iterate until 100

**The orchestrator MUST run cold-read audits after every subagent ticket completion — never trust "complete" claims.** When a subagent reports done, the parent context independently re-reads the diff, runs the tests, confirms file presence, and verifies the acceptance gate before approving merge.

---

## Code Style & Best Practices (Rust)

- **Modular design:** files under 500 lines preferred; 1500 hard cap (enforced by Gate 2)
- **Environment safety:** never hardcode secrets; use env vars or `~/.config/archon/`
- **Test-first:** write tests before implementation (Gate 1)
- **Clean architecture:** separate concerns by crate; avoid circular deps
- **Documentation:** keep `docs/` updated for any user-facing change
- **Error handling:** use `anyhow::Result` or typed errors; no `unwrap()` / `expect()` outside tests
- **No `#[allow(...)]`** to suppress warnings — fix the underlying issue
- **Comments explain WHY, not WHAT** (well-named code self-documents the WHAT)

### Build Commands (Rust)

WSL2 users MUST use `-j1` to avoid OOM:

| Command | Purpose |
|---|---|
| `cargo build --release --bin archon -j1` | Release build |
| `cargo build --bin archon` | Dev build |
| `cargo nextest run --workspace -j1 -- --test-threads=2` | Test suite (WSL2) |
| `cargo nextest run --workspace` | Test suite (native Linux/macOS) |
| `cargo fmt --all -- --check` | Format check |
| `cargo clippy --workspace -- -D warnings` | Lint with warnings as errors |
| `cargo check --workspace --tests -j1` | Compile check (no tests run) |

**Known cache-corruption recovery:**
```bash
cargo clean -p petgraph -p archon-pipeline
cargo build --release --bin archon -j1
```

This recovers from the rustc ICE on `petgraph::graphmap::NeighborsDirected::next` caused by stale dep metadata.

---

# important-instruction-reminders

## 🛑 PRIME DIRECTIVE REMINDER
**STOP AND ASK before doing anything. Never act autonomously after compaction.**

## 🧠 MEMORY REMINDER
**ALL memory uses `memory_store` / `memory_recall` (or `mcp__memorygraph__*` if MCP configured). NEVER write to MEMORY.md or markdown files for memory storage.**

## DEV FLOW ENFORCEMENT — ABSOLUTE LAW

**When executing tasks from `project-tasks/`, EVERY task MUST complete ALL 6 gates IN ORDER. No exceptions. No shortcuts. "Going fast" does NOT mean skipping gates.**

```
GATE 1: tests-written-first       — Test file exists BEFORE implementation
GATE 2: implementation-complete    — Code compiles, no errors. File-size auto-check (< 1500 lines)
GATE 3: sherlock-code-review       — Sherlock adversarial review of implementation (MUST contain APPROVED/PASS)
GATE 4: tests-passing              — All tests pass (include count)
GATE 5: live-smoke-test            — Feature actually invoked end-to-end (fraud detection blocks fake evidence)
GATE 6: sherlock-final-review      — Sherlock final review: integration + wiring verified (MUST contain APPROVED/PASS)
```

**Enforcement mechanism (HARDENED — cannot be bypassed):**
- Run `scripts/dev-flow-pass-gate.sh TASK-ID gate-name "evidence"` to pass each gate
- Run `scripts/dev-flow-gate.sh TASK-ID` to verify all gates before marking complete
- **PreToolUse hook on TaskUpdate BLOCKS marking any TASK-*-NNN as completed without all 6 gates passed**
- Gate 3 + 6: Evidence MUST contain Sherlock verdict (APPROVED / PASS / INNOCENT). REJECTED = blocked.
- Gate 5: Fraud detection blocks "tests pass", "library crate", "not yet wired" etc. Requires real execution proof.
- **The hook cannot be bypassed by "forgetting" to call the gate scripts — no gate files = BLOCKED.**
- A task with missing gates is NOT DONE regardless of what the agent thinks

**VIOLATION = immediate stop and report. The agent does NOT get to decide which gates matter.**

Full dev-flow documentation: `docs/development/dev-flow-gates.md`.

## Cargo discipline (WSL2)

`cargo` commands on archon-cli MUST use `-j1` (and `--test-threads=2` on tests). Crashed Claude Code historically (2026-04-11) due to parallel rustc processes against archon-cli's 21-crate workspace exhausting WSL2 memory.

| Command | Required form |
|---|---|
| Build | `cargo build --release --bin archon -j1` |
| Test (WSL2) | `cargo nextest run --workspace -j1 -- --test-threads=2` |
| Check | `cargo check --workspace --tests -j1` |

Native Linux / macOS / Windows can omit `-j1`. **WSL2 cannot.**

## Audit pattern after every subagent ticket

ABSOLUTE RULE — when orchestrating subagent ticket execution (any executor — sherlock-holmes, coder, tester, custom agents), the parent context MUST run an independent cold-read audit before accepting any "COMPLETE" claim:

1. Independently re-read the diff (`git diff main..HEAD`)
2. Verify scope: only the spec'd files changed; nothing leaked
3. Run the tests independently (`cargo nextest run -p <crate> --test <name>`)
4. Run `cargo fmt --all -- --check` and `cargo build --release --bin archon -j1`
5. Confirm fresh binary mtime + version SHA matches HEAD
6. Approve OR reject with specific findings; never blanket-approve

## Core Rules

1. Do what has been asked; nothing more, nothing less.
2. **ALWAYS wait for explicit user confirmation before executing any plan.**
3. NEVER create files unless explicitly requested AND confirmed.
4. ALWAYS prefer editing an existing file to creating a new one.
5. NEVER proactively create documentation files (`*.md`) or README files outside the structured `docs/` tree.
6. Never save working files, scratch text, or tests to the repository root.
7. **After compaction: summarize state, ask what's next, WAIT for response.**
8. **"I'll go ahead and..." is FORBIDDEN. Ask first, always.**
9. When in doubt, ask. When not in doubt, still ask.
10. Treat every session start and context restore as a fresh conversation requiring new confirmation.
11. **NEVER spawn parallel implementation agents — sequential ONLY.** Read-only agents (research, analysis) can run in parallel.
12. **After compaction: recall behavioural rules from the memory graph before proceeding.**
13. **`/run-agent` and `/agent run` do NOT require confirmation — the command IS the intent.**
14. **EVERY task from `project-tasks/` MUST pass all 6 dev flow gates. Sherlock review is NOT optional.**
15. **"User said go fast" does NOT mean "skip quality gates." It means "don't stop to ask between tasks."**
16. **Cargo commands on WSL2 use `-j1` always — see "Cargo discipline" above.**
17. **No `Co-Authored-By:` lines in commit messages.**
