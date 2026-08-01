# Code Simplifier Plugin

An agent that simplifies and refines code for clarity, consistency, and maintainability while preserving functionality.

> Ported from the Claude Code `code-simplifier` plugin ([source](https://github.com/ste-bah/test-plugins-official/tree/main/plugins/code-simplifier), Apache-2.0).

## What It Does

Provides one subagent, `code-simplifier:code-simplifier`, that runs a simplification pass over recently modified code:

- Preserves exact functionality — it changes how code works, never what it does
- Applies the project's coding standards from ARCHON.md
- Reduces nesting, redundancy, and unnecessary abstractions; improves naming
- Avoids over-simplification (no nested ternaries, no dense one-liners, no cleverness at the cost of readability)
- Scopes itself to recently modified code unless told otherwise

## Usage

Ask Archon to spawn it with the Agent tool:

```
"Use the code-simplifier:code-simplifier agent to clean up the code we just wrote"
"Spawn code-simplifier:code-simplifier on src/parser/ — full-file scope"
```

It also works well as a routine post-change pass after a feature or fix lands.

## Installation

Project-local (recommended): copy `agents/` to `<project>/.archon/plugins/code-simplifier/agents/`.

Or user-global: copy `agents/` to `~/.archon/plugins/code-simplifier/agents/`.

Or run from the archon-cli repo root:

```
plugins/install.sh code-simplifier /path/to/project    # or: plugins\install.ps1 code-simplifier -ProjectDir <path>
```

Restart `archon` after installing (agents load at startup).

## Differences from the Claude plugin

- The upstream plugin has no README; this file was written for the Archon port.
- The single flat agent file (`agents/code-simplifier.md`) is split into Archon's 6-file agent directory (`agent.md`, `behavior.md`, `context.md`, `tools.md`, `memory-keys.json`, `meta.json`); all source content is preserved across the split.
- `model: opus` frontmatter is dropped — Archon's model selection differs, so the agent inherits the session model.
- The source tells the agent to follow "the established coding standards from" Claude Code's project-memory file; the port points it at `ARCHON.md`. The JS/React-specific example standards hard-coded upstream are kept in `context.md` as fallback defaults that ARCHON.md overrides.
- One typo fixed in the role text ("as a result your years" → "as a result of your years").
