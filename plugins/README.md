# Archon Plugin Collection

Archon ports of the official Claude Code plugins ([source](https://github.com/ste-bah/test-plugins-official/tree/main/plugins), Apache-2.0). Each plugin is a markdown bundle of skills (slash commands), subagents, and/or hooks — no compilation, no WASM, no manifest. Copy the pieces into place (or run the installer) and restart `archon`.

## Plugins

| Plugin | Provides | What it does |
|---|---|---|
| [code-review](code-review/) | `/code-review` | Multi-agent review of your branch or a GitHub PR with confidence-scored findings |
| [pr-review-toolkit](pr-review-toolkit/) | `/review-pr` + 6 agents | Specialized review agents: comments, tests, silent failures, type design, simplification, general review |
| [feature-dev](feature-dev/) | `/feature-dev` + 3 agents | Guided feature development: explore → clarify → architect → implement → review |
| [commit-commands](commit-commands/) | `/commit`, `/commit-push-pr`, `/clean-gone` | Git workflow accelerators |
| [archon-md-management](archon-md-management/) | `/revise-archon-md`, `/archon-md-improver` | Audit and improve your project's ARCHON.md |
| [code-simplifier](code-simplifier/) | `code-simplifier:code-simplifier` agent | Post-change simplification pass preserving behavior |
| [frontend-design](frontend-design/) | `/frontend-design` | Distinctive, production-grade web UI design guidance |
| [hookify](hookify/) | `/hookify` + agent + hooks | Create hook rules from natural language (or conversation analysis) |
| [security-guidance](security-guidance/) | hooks | Passive warnings when editing security-sensitive files |
| [ralph-loop](ralph-loop/) | `/ralph-loop`, `/cancel-ralph` + Stop hook | Autonomous iteration loop that keeps working until done |

## Install

From the repo root:

```bash
# project-local (recommended): installs into <project>/.archon/
plugins/install.sh code-review /path/to/project

# user-global: installs into ~/.archon/plugins/ and your user skill root
plugins/install.sh --user code-review

# everything
plugins/install.sh all /path/to/project
```

On Windows (PowerShell):

```powershell
plugins\install.ps1 code-review -ProjectDir C:\path\to\project
plugins\install.ps1 code-review -User
plugins\install.ps1 all -ProjectDir C:\path\to\project
```

Restart `archon` afterwards — skills and agents are discovered at startup.

### What goes where

| Bundle piece | Project install | User install |
|---|---|---|
| `agents/` | `<project>/.archon/plugins/<plugin>/agents/` | `~/.archon/plugins/<plugin>/agents/` |
| `skills/<name>/` | `<project>/.archon/skills/<name>/` | `~/.config/archon/skills/<name>/` (Windows: `%APPDATA%\archon\skills\<name>\`) |
| `scripts/` | `<project>/.archon/plugins/<plugin>/scripts/` | `~/.archon/plugins/<plugin>/scripts/` |
| `hooks/settings.snippet.json` | merged into `<project>/.archon/settings.json` | merged into `~/.archon/settings.json` |

Hooks are **enabled by default at install** (matching Claude Code, where installing a plugin activates its bundled hooks). The merge is idempotent and preserves any hooks you already have. Pass `--no-hooks` (sh) / `-NoHooks` (PowerShell) to install without enabling them — the installer then prints the snippet for manual merging. Hook scripts run shell commands on lifecycle events, so review `scripts/` before installing if that matters in your environment. Hook scripts use `jq` to parse event JSON — install it if you don't have it (`winget install jqlang.jq` / `apt install jq`).

### Invoking what you installed

- **Skills** are slash commands: `/code-review`, `/feature-dev focus on the auth module`, … Arguments are appended to the skill prompt.
- **Agents** are namespaced `<plugin>:<agent>` (e.g. `pr-review-toolkit:silent-failure-hunter`) and are spawned by the model via the Agent tool — the bundled skills reference them by these names. You can also ask for one directly: "use the feature-dev:code-explorer agent to map the payment flow".

## How discovery works

- Agents: Archon scans `<project>/.archon/plugins/*/agents/*` and `~/.archon/plugins/*/agents/*` at startup (project wins over user only through the normal precedence chain; a custom agent of the same key beats both). Each agent dir uses Archon's 6-file format: `agent.md`, `behavior.md`, `context.md`, `tools.md`, `memory-keys.json`, `meta.json`.
- Skills: Archon scans `<project>/.archon/skills/` then user skill roots; `<name>/SKILL.md` subdir layout wins over flat files. Only `name:` and `description:` frontmatter keys are read.
- Hooks: loaded from `config.toml` or `.archon/settings.json` — see [docs/integrations/hooks.md](../docs/integrations/hooks.md).

## Porting conventions

These bundles were ported from the Claude Code originals with mechanical rules: commands and skills → `SKILL.md` skills; flat-frontmatter agents → 6-file agent dirs; `hooks.json` → `.archon/settings.json` snippets; `CLAUDE.md` → `ARCHON.md`; Task tool → Agent tool with `plugin:agent` names; `${CLAUDE_PLUGIN_ROOT}` → `.archon/plugins/<plugin>`. Every intentional deviation is listed in each plugin's README under "Differences from the Claude plugin". Original LICENSE files (Apache-2.0) ship with each plugin.

## Licensing

Archon itself is MIT-licensed, but the contents of this `plugins/` directory are derived from Apache-2.0 sources and remain **Apache-2.0** (the license does not permit relicensing derivatives as MIT). Each plugin directory carries its upstream LICENSE verbatim, attribution links in every ported file, and change documentation in its README — which together satisfy Apache-2.0's redistribution conditions. The collection-level tooling written from scratch for Archon (`install.sh`, `install.ps1`, `merge-hooks.js`, this README) is MIT like the rest of the repo.

## Not ported (yet)

- **LSP plugins** (`*-lsp`) — thin wrappers around Claude Code's LSP client config; Archon has its own `lsp` tool.
- **claude-code-setup, agent-sdk-dev, plugin-dev, mcp-server-dev, skill-creator** — Claude-Code-specific tooling; Archon covers the equivalents with `/write-a-skill`, `/setup-archon-skills`, and its own docs.
- **explanatory/learning output styles** — depend on Claude Code output styles, no Archon equivalent.
- **session-report** — Archon has `/insights` and `/stats` built in.
- **claude-security, code-modernization, cwc-makers, math-olympiad, playground, project-artifact, receipts, mcp-tunnels, ralph-* extras** — candidates for a second tranche; ask if you want one prioritized.
