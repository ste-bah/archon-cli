# Plugins

Archon supports two kinds of plugins:

1. **Markdown plugin bundles** — directories of subagents, SKILL.md skills, and hook snippets. No compilation, no manifest; pieces are discovered from well-known paths. This is the right format for prompt-driven workflows (reviews, git automation, guided development).
2. **WASM plugins** — WebAssembly modules loaded with a `.archon-plugin/plugin.json` manifest. They can register tools, hooks, and slash commands when the manifest declares the matching structured capability and the operator grants it. This is the right format for plugins that need real code execution.

## Markdown Plugin Bundles

A bundle is a directory:

```text
.archon/plugins/
|-- my-bundle/
|   |-- agents/
|   |   `-- my-agent/          # 6-file agent format
|   |       |-- agent.md       # identity + ## INTENT (catalog description)
|   |       |-- behavior.md    # process rules
|   |       |-- context.md     # reference material
|   |       |-- tools.md       # ## Primary Tools allowlist + guidance
|   |       |-- memory-keys.json
|   |       `-- meta.json
|   |-- skills/                # copied into a skill root at install time
|   |-- scripts/               # hook scripts
|   `-- hooks/settings.snippet.json
```

- **Agents** under `<project>/.archon/plugins/<bundle>/agents/` and `~/.archon/plugins/<bundle>/agents/` are auto-discovered at startup and namespaced as `<bundle>:<agent>` (spawned via the Agent tool). Precedence: built-in < project plugin < user plugin < custom agents. Directories prefixed `_` are skipped.
- **Skills** are not auto-discovered from bundle dirs — install them by copying each `skills/<name>/` into a [skill root](../reference/skills.md) such as `<project>/.archon/skills/`.
- **Hooks** ship as a `settings.snippet.json` to merge manually into `.archon/settings.json` — see [Hooks](hooks.md). Never merged automatically.

A curated collection of bundles ported from the official Claude Code plugins lives in [`plugins/`](../../plugins/README.md) at the repo root, with `install.sh` / `install.ps1` helpers.

## WASM Plugin Layout

```text
.archon/plugins/
|-- my-plugin/
|   |-- .archon-plugin/
|   |   `-- plugin.json
|   `-- plugin.wasm
```

## Manifest Schema

`plugin.json`:

```json
{
  "name": "my-plugin",
  "version": "0.1.0",
  "description": "Example plugin",
  "author": "you@example.com",
  "capabilities": [
    { "kind": "ToolRegister" },
    { "kind": "ReadFs", "paths": ["/repo/docs"] },
    { "kind": "Network", "hosts": ["api.example.com"] }
  ],
  "required_host_functions": [
    "archon_log",
    "archon_register_tool",
    "archon_host_call"
  ]
}
```

Legacy string capability entries such as `"ReadFs"` are rejected with a migration error. Filesystem capabilities must list absolute paths. Network capabilities must enumerate concrete hosts; `"*"` is not valid in plugin manifests. A wildcard network grant is only available as an explicit high-risk operator approval outside the manifest path and is recorded as a load warning.

If a plugin declares a required host function Archon does not implement, loading fails with a manifest validation error.

## CLI

```bash
archon plugin list
archon plugin info <name>
```

In the TUI:

```text
/plugin
/plugin info <name>
/reload-plugins
```

## Discovery Paths

archon-cli searches for plugins in priority order:

1. `<workdir>/.archon/plugins/`
2. `~/.config/archon/plugins/`
3. `~/.local/share/archon/plugins/`

A plugin found at multiple paths uses the highest-priority location.

## Architecture

Plugins run inside the Archon WASM host with fuel and memory limits. A plugin that panics, exceeds its fuel budget, exceeds memory limits, or fails ABI negotiation is skipped without taking down the main Archon process.

The full plugin API surface lives in `crates/archon-plugin/`.

## See Also

- [Hooks](hooks.md) - event-driven shell commands
- [Skills](../reference/skills.md) - SKILL.md prompt workflows
- [Adding a tool](../development/adding-a-tool.md) - built-in tool implementation
