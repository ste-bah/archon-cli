# LSP integration

archon-cli speaks Language Server Protocol over stdio to any LSP server (rust-analyzer, pyright, typescript-language-server, gopls, clangd, etc.). All LSP operations are exposed through the single `LSP` tool.

## Supported operations

| Operation | Purpose |
|---|---|
| `goToDefinition` | Jump to symbol definition |
| `findReferences` | Find all references to a symbol |
| `hover` | Get hover info (types, docs) |
| `documentSymbol` | List symbols in a file |
| `workspaceSymbol` | Search symbols across the workspace |
| `goToImplementation` | Jump to trait/interface implementation |
| `prepareCallHierarchy` | Set up call hierarchy session |
| `incomingCalls` | List callers of a function |
| `outgoingCalls` | List functions called by a function |

When no language server is connected for the current file's language, the `LSP` tool returns empty results.

## Server auto-discovery

archon-cli detects the project language from file extensions and launches the appropriate LSP server. Default mappings:

| Extension | Server |
|---|---|
| `.rs` | rust-analyzer |
| `.py` | pyright-langserver |
| `.ts` / `.tsx` / `.js` / `.jsx` | typescript-language-server |
| `.go` | gopls |
| `.c` / `.cpp` / `.h` / `.hpp` | clangd |
| `.rb` | solargraph |

## Override via config

Drop a `<workdir>/.archon/lsp.toml`:

```toml
[servers.rust]
command = "rust-analyzer"
args = []
init_timeout_ms = 30000
request_timeout_ms = 10000

[servers.python]
command = "pyright-langserver"
args = ["--stdio"]

[servers.typescript]
command = "typescript-language-server"
args = ["--stdio"]
init_timeout_ms = 60000
```

Per-server fields:
- `command` — executable
- `args` — arguments
- `init_timeout_ms` — initialization timeout
- `request_timeout_ms` — per-request timeout
- `env` — environment variables (object)

## Diagnostics

LSP diagnostics are pushed in real time and surfaced via the `/insights` skill, which aggregates errors, warnings, and lint output across the session.

## Debugging

```bash
archon --debug lsp                # debug-level LSP transport
RUST_LOG=archon=trace archon      # full tracing including LSP frames
```

`/doctor` includes LSP server status and last error.

## See also

- [Tools](../reference/tools.md) — `LSP` and `CartographerScan`
- [IDE extensions](ide-extensions.md) — VS Code / JetBrains integration
