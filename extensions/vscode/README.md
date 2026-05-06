# Archon VS Code extension

JSON-RPC bridge between VS Code and a local archon-cli binary.
Talks to `archon ide-stdio` over stdin/stdout by default, or to a
running Archon WebSocket server.

## Install From Source

VS Code Marketplace publication is pending.

```bash
git clone https://github.com/ste-bah/archon-cli
cd archon-cli

CARGO_BUILD_JOBS=2 cargo build --release -p archon-cli-workspace -j1 --bin archon
sudo cp target/release/archon /usr/local/bin/archon

cd extensions/vscode
npm install
npm run package
code --install-extension archon-vscode-0.1.50.vsix
```

## Configuration

Open VS Code Settings and search for "Archon".

| Setting | Default | Purpose |
|---|---|---|
| `archon.connectionMode` | `stdio` | `stdio` or `websocket` |
| `archon.binaryPath` | `archon` | Path to the archon binary in stdio mode |
| `archon.websocketUrl` | `ws://localhost:8420/ws/ide` | Endpoint for the headless server |

## Commands

The extension contributes six commands:

- `Archon: Open Chat`
- `Ask Archon`
- `Archon: Explain Code`
- `Archon: Fix This Error`
- `Archon: Generate Tests`
- `Archon: Reconnect`

Right-click selected code in the editor to access the selection-aware
commands from the context menu.

## Development

Packaging uses `@vscode/vsce`, which requires Node.js 20 or newer.

```bash
npm install
npm run typecheck
npm run build
npm test
npm run package
```

Open `extensions/vscode/` in VS Code and press F5 to launch an Extension
Development Host. Reload inside that window to pick up changes.

## Marketplace Status

Not published yet. Install from source using the steps above.

See [docs/integrations/ide-extensions.md](https://github.com/ste-bah/archon-cli/blob/main/docs/integrations/ide-extensions.md)
for the JSON-RPC protocol used by the extension.
