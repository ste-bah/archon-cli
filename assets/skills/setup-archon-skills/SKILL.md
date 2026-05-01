---
name: setup-archon-skills
description: Interactive first-run wizard walking the user through 8 configuration prompts and optionally bootstrapping the project. Use on first launch or when reconfiguring archon-cli.
---

# Setup Archon Skills

Interactive configuration wizard. The agent walks the user through 8 prompts naturally in conversation. The agent uses Read/Write/Bash tools to inspect and update configuration — the skill only provides the instructions.

## Process

### 0. Read existing config

Use the Read tool to inspect `~/.config/archon/config.toml` if it exists. Report current values so the user knows what they're changing.

### 1. Authentication

Ask: "Use OAuth (Claude.ai subscription) or API key?"

- If OAuth: tell them to run `archon login`
- If API key: tell them to set `ANTHROPIC_API_KEY` environment variable

### 2. Personality

Ask: "MBTI type for the agent? (default: INTJ)"

Write to `[personality]` section.

### 3. Theme

Ask: "Theme? (default: derived from MBTI)"

Write to `[tui]` section if explicit choice given.

### 4. Permission mode

Ask: "Default permission mode? (default | acceptEdits | auto | dontAsk | bypassPermissions)"

Write to `[permissions]` section.

### 5. Embedding provider

Ask: "Embedding provider? (auto | local | openai)"

Write to `[memory]` section.

### 6. Model

Ask: "Default model? (claude-sonnet-4-6 | claude-opus-4-7 | claude-haiku-4-5)"

Ask: "Effort level? (high | medium | low)"

Write to `[api]` section.

### 7. Output paths

Ask: "Where should /to-prd write PRDs? (default: <workdir>/prds/)"

Ask: "Where should task specs go? (default: <workdir>/tasks/)"

Write to `[paths]` section, or accept defaults.

### 8. Project initialisation

Ask: "Initialise this project for archon-cli now? (creates .archon/, prds/, tasks/)"

If yes:
- Check whether `./scripts/archon-init.sh` exists (Path B: building from source)
  - If yes, run: `bash scripts/archon-init.sh --target $(pwd)` via the Bash tool
  - If no (Path A: binary-only install), tell the user to run:
    ```
    curl -L https://raw.githubusercontent.com/ste-bah/archon-cli/main/scripts/archon-init.sh | bash
    ```
    and re-launch archon afterward. Do NOT auto-execute curl-pipe-bash.

### 9. Summary

Print a final summary of all choices and where they were saved.
