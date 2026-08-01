# Frontend Design Plugin

Generates distinctive, production-grade frontend interfaces that avoid generic AI aesthetics.

> Ported from the Claude Code `frontend-design` plugin ([source](https://github.com/ste-bah/test-plugins-official/tree/main/plugins/frontend-design), Apache-2.0).

## What It Does

Provides the `/frontend-design` skill for frontend work. Creates production-ready code with:

- Bold aesthetic choices
- Distinctive typography and color palettes
- High-impact animations and visual details
- Context-aware implementation

## Usage

Run the skill with your design brief appended (arguments are added to the end of the injected prompt):

```
/frontend-design Create a dashboard for a music streaming app
/frontend-design Build a landing page for an AI security startup
/frontend-design Design a settings panel with dark mode
```

Archon will choose a clear aesthetic direction and implement production code with meticulous attention to detail.

## Learn More

See the [Frontend Aesthetics Cookbook](https://github.com/anthropics/claude-cookbooks/blob/main/coding/prompting_for_frontend_aesthetics.ipynb) for detailed guidance on prompting for high-quality frontend design.

## Installation

Project-local (recommended): copy `skills/frontend-design/` (including `LICENSE.txt`) to `<project>/.archon/skills/frontend-design/`.

Or user-global: copy it to `~/.config/archon/skills/frontend-design/` (Windows: `%APPDATA%\archon\skills\frontend-design\`).

Or run from the archon-cli repo root:

```
plugins/install.sh frontend-design /path/to/project    # or: plugins\install.ps1 frontend-design -ProjectDir <path>
```

Restart `archon` after installing (skills load at startup).

## Differences from the Claude plugin

- In Claude Code the skill auto-triggers on any frontend work via its description; Archon injects skills on demand — run `/frontend-design` (optionally with the brief as arguments) before or alongside your frontend request.
- The skill body is model-agnostic design guidance and is ported nearly verbatim; the only additions are the Archon attribution line and expanded "use when" triggers in the frontmatter description.
- The skill-level `LICENSE.txt` referenced by the skill's `license:` frontmatter is kept next to SKILL.md; the plugin-level LICENSE is copied verbatim.

## Authors

Prithvi Rajasekaran (prithvi@anthropic.com)
Alexander Bricken (alexander@anthropic.com)
