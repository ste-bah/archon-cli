# Setup wizard

`/setup-archon-skills` is an interactive 8-prompt configuration wizard. It walks through every archon-cli setting and optionally bootstraps the project directory.

## Invocation

Type `/setup-archon-skills` in the TUI. The agent becomes a guided configurator — it reads your existing config, prompts for each setting, and writes the result.

## Prompts

### 1. Authentication

OAuth (Claude.ai subscription) or API key. If OAuth, the wizard tells you to run `archon login`. If API key, it tells you to set `ANTHROPIC_API_KEY`.

### 2. Personality

MBTI type. Default: INTJ. Written to `[personality]` config section.

### 3. Theme

TUI theme. Default: derived from MBTI. Written to `[tui]` if an explicit choice is given.

### 4. Permission mode

One of: `default`, `acceptEdits`, `auto`, `dontAsk`, `bypassPermissions`. Written to `[permissions]`.

### 5. Embedding provider

One of: `auto`, `local`, `openai`. Written to `[memory]`.

### 6. Model and effort

Model: `claude-sonnet-4-6`, `claude-opus-4-7`, or `claude-haiku-4-5`. Effort: `high`, `medium`, or `low`. Written to `[api]`.

### 7. Output paths

Where `/to-prd` writes PRDs (default: `<workdir>/prds/`). Where task specs go (default: `<workdir>/tasks/`). Written to `[paths]`.

### 8. Project initialisation

Bootstraps `.archon/`, `prds/`, `tasks/` directories. Two paths:

- **Path A (binary install):** `curl -L https://raw.githubusercontent.com/ste-bah/archon-cli/main/scripts/archon-init.sh | bash`
- **Path B (building from source):** `bash scripts/archon-init.sh --target $(pwd)`

The wizard auto-detects which path applies by checking for `scripts/archon-init.sh` locally.

## Config file

All choices are written to `~/.config/archon/config.toml`. The wizard reads existing values before prompting so you know what you're changing.

## Idempotency

Safe to re-run. The wizard shows current values before asking for changes. Press Enter to accept the default / current value.

## See also

- [Configuration reference](../reference/config.md) — full config.toml schema
- [First run](../getting-started/first-run.md) — what happens on first launch
- [Quick start](../getting-started/quick-start.md) — 5-minute setup path
