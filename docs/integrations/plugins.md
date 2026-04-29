# Plugins

Plugins are dynamically loaded Rust libraries (`.so` / `.dll` / `.dylib`) implementing the `archon_plugin::api` trait. They register new tools, hooks, skills, and slash commands at runtime.

## Plugin layout

```
.archon/plugins/
├── my-plugin/
│   ├── plugin.toml         # manifest
│   └── libmy_plugin.so     # compiled plugin
```

## Manifest schema

`plugin.toml`:

```toml
name = "my-plugin"
version = "0.1.0"
description = "Example plugin"
author = "you@example.com"
capabilities = ["tools", "skills", "hooks"]    # which surfaces this plugin extends

[[tools]]
name = "MyTool"
description = "Does something specific"
schema = "schemas/my-tool.json"
permission = "Risky"                            # Safe | Risky | Variable

[[skills]]
name = "my-plugin-skill"
description = "Workflow shortcut"
trigger = "/my-plugin"
template_path = "templates/my-skill.md"

[[hooks]]
event = "PostToolUse"
command = "scripts/post-hook.sh"
timeout = 30
```

## CLI

```bash
archon plugin list                # discovered plugins
archon plugin info <name>         # show manifest + status
```

In the TUI:
```
/plugin                           # list plugins
/plugin info <name>               # show details
/reload-plugins                   # re-scan plugin directories
```

## Discovery paths

archon-cli searches for plugins in (priority order):

1. `<workdir>/.archon/plugins/`
2. `~/.config/archon/plugins/`
3. `~/.local/share/archon/plugins/` (system-installed)

A plugin found at multiple paths uses the highest-priority location.

## Architecture

Plugins run **out-of-process** with crash isolation. The plugin host bridges tool calls and hook invocations via JSON-RPC over stdio. If a plugin panics or hangs, the host kills it and surfaces the error to the user without taking down the main archon process.

## Building a plugin

```toml
# Cargo.toml
[package]
name = "my-plugin"

[lib]
crate-type = ["cdylib"]

[dependencies]
archon-plugin-api = "0.1"
```

```rust
// src/lib.rs
use archon_plugin_api::*;

#[no_mangle]
pub extern "C" fn archon_plugin_register(host: &mut PluginHost) {
    host.register_tool(MyTool::new());
    host.register_skill(MySkill::new());
}
```

Compile with:
```bash
cargo build --release --crate-type cdylib
cp target/release/libmy_plugin.so ~/.config/archon/plugins/my-plugin/
```

The full plugin API surface lives in `crates/archon-plugin/`.

## Built-in plugin examples

The repo includes example plugins under `examples/plugins/` (when present) demonstrating tool and skill registration patterns.

## See also

- [Hooks](hooks.md) — event-driven shell commands (lighter-weight than plugins)
- [Skills](../reference/skills.md) — TOML-based skills (no compilation required)
- [Adding a tool](../development/adding-a-tool.md) — built-in tool implementation
