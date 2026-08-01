# ARCHON.md Management Plugin

Tools to maintain and improve ARCHON.md files - audit quality, capture session learnings, and keep project memory current.

> Ported from the Claude Code `claude-md-management` plugin ([source](https://github.com/ste-bah/test-plugins-official/tree/main/plugins/claude-md-management), Apache-2.0). The upstream plugin manages Claude Code's project-memory file; this port manages `ARCHON.md`, Archon's equivalent project-instructions file (same role, same format).

## What It Does

Two complementary skills for different purposes:

| | /archon-md-improver | /revise-archon-md |
|---|---|---|
| **Purpose** | Keep ARCHON.md aligned with codebase | Capture session learnings |
| **Triggered by** | Codebase changes | End of session |
| **Use when** | Periodic maintenance | Session revealed missing context |

## Usage

### Skill: /archon-md-improver

Audits ARCHON.md files against current codebase state:

```
/archon-md-improver
/archon-md-improver focus on the packages/api module
```

Outputs a quality report (scored per file), then proposes targeted updates and applies them with your approval. Arguments you type after the skill name are appended to the end of the prompt.

### Skill: /revise-archon-md

Captures learnings from the current session:

```
/revise-archon-md
```

Reflects on the session, drafts concise additions, shows proposed diffs, and only edits files you approve.

## Installation

Project-local (recommended):

1. Copy `skills/archon-md-improver/` (including `references/`) to `<project>/.archon/skills/archon-md-improver/`.
2. Copy `skills/revise-archon-md/` to `<project>/.archon/skills/revise-archon-md/`.

Or user-global: copy each `skills/<skill>/` dir to `~/.config/archon/skills/` (Windows: `%APPDATA%\archon\skills\`).

Or run from the archon-cli repo root:

```
plugins/install.sh archon-md-management /path/to/project    # or: plugins\install.ps1 archon-md-management -ProjectDir <path>
```

Restart `archon` after installing (skills load at startup).

## Differences from the Claude plugin

- Renamed `claude-md-management` → `archon-md-management`. Every reference to Claude Code's project-memory file is adapted to `ARCHON.md`; the companion files are adapted the same way (`.archon.md`, `.archon.local.md`, and `~/.archon/ARCHON.md` for user-global defaults).
- The upstream `/revise-claude-md` command became the `revise-archon-md` skill; the upstream `claude-md-improver` skill became `archon-md-improver`. In Claude Code the improver auto-triggers from its description ("audit my project memory"); in Archon both are invoked as slash commands.
- The example screenshots (`claude-md-improver-example.png`, `revise-claude-md-example.png`) were not ported; their image references are removed from this README.
- The upstream "`#` key shortcut" user tip (Claude Code's mid-session auto-memorize key) has no Archon equivalent; it is replaced with a pointer to `/revise-archon-md`.
- The `allowed-tools: Read, Edit, Glob` frontmatter on the revise command is dropped (Archon ignores it); the tool constraint is stated in the skill body instead.
- `references/` links are rewritten from relative markdown links to install-relative paths (`.archon/skills/archon-md-improver/references/...`), since Archon injects only SKILL.md. A pointer to `references/update-guidelines.md` was added in Phase 4 of the improver (upstream ships the file but never links it).

## Author

Isabella He (isabella@anthropic.com) — original Claude Code plugin.
