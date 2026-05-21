# Plugins

Plugins are WebAssembly modules loaded with a `.archon-plugin/plugin.json` manifest. They can register tools, hooks, and slash commands when the manifest declares the matching structured capability and the operator grants it.

## Plugin Layout

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
