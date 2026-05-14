# Installation

archon-cli is a 26-crate Cargo workspace. There are no precompiled binaries — clone, install Rust, build with `cargo build --release`. End-to-end build time is ~3-4 minutes on a modern laptop, longer on WSL2.

> **After install: TUI parity.** All examples in the rest of the docs show shell commands like `archon docs ingest ./path`. Inside the TUI those become `/docs ingest ./path` (drop the `archon` and prefix with `/`). Both forms work; both go through the same crates and write to the same persisted state. See [CLI and TUI Command Parity](../cookbook/real-world-evidence-engine.md#cli-and-tui-command-parity).

## Prerequisites

| Requirement | Minimum | Notes |
|---|---|---|
| Rust toolchain | 1.85+ | edition 2024 — older toolchains will not compile |
| `cargo` | bundled with Rust | comes from rustup |
| Git | any recent | for `git clone` and branch-aware sessions at runtime |
| Disk space | ~3 GB free | `target/` build artefacts dominate |
| RAM | 4 GB minimum, 8 GB+ recommended | linker phase peaks; WSL2 OOMs with parallel rustc on 4 GB |
| OS | Linux, macOS 12+, Windows 10/11 (native or WSL2) | |

## Install Rust

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
rustc --version    # verify: 1.85.0 or newer
```

## OS-specific build dependencies

archon-cli links against OpenSSL via `reqwest` (rustls is also enabled, but build-deps still need pkg-config + libssl headers on Linux for transitive crates).

> **Quick path: one-shot installer.** `scripts/install-system-deps.sh` detects your OS and installs the required build/doc-ingest packages at once: build deps + `pdftotext`/`pdfimages`/`pdftoppm` (poppler) for PDF ingest + `tesseract` for image OCR. Supports Ubuntu/Debian/WSL2, Fedora/RHEL/Rocky/Alma, Arch/Manjaro, openSUSE/SLE, Alpine, and macOS (Homebrew).
>
> ```bash
> sudo scripts/install-system-deps.sh                  # required build/PDF/OCR deps
> sudo scripts/install-system-deps.sh --with-docker    # add Docker sandbox deps
> sudo scripts/install-system-deps.sh --with-openshell # add Docker + OpenShell deps
> sudo scripts/install-system-deps.sh --with-sandbox   # add both sandbox extras
> scripts/install-system-deps.sh --with-openshell --setup-openshell-gateway
> scripts/install-system-deps.sh --check               # verify required deps
> scripts/install-system-deps.sh --check --with-sandbox
> scripts/install-system-deps.sh --dry-run --with-sandbox
> ```
>
> Sandbox backends remain opt-in after installation. Enable them in
> `[sandbox]` only after reviewing [Sandboxing](../security/sandboxing.md).
> The manual per-distro lists below are kept for reference and for unsupported distros.

### Ubuntu / Debian / WSL2-Ubuntu

```bash
sudo apt update
sudo apt install -y build-essential pkg-config libssl-dev git poppler-utils tesseract-ocr
```

### Fedora / RHEL / Rocky

```bash
sudo dnf install -y gcc pkg-config openssl-devel git poppler-utils tesseract
```

### Arch / Manjaro

```bash
sudo pacman -S --needed base-devel openssl pkg-config git poppler tesseract
```

### macOS

```bash
xcode-select --install
brew install poppler tesseract             # PDF text extraction + image OCR
# OpenSSL supplied by the system; no extra steps for default builds.
# If a transitive crate complains about OpenSSL:
brew install pkg-config openssl
export PKG_CONFIG_PATH="$(brew --prefix openssl)/lib/pkgconfig"
```

### Windows (native)

```powershell
winget install Rustlang.Rustup
winget install Microsoft.VisualStudio.2022.BuildTools
# Select "Desktop development with C++" during install
winget install Git.Git
# Optional Docker sandbox backend:
winget install Docker.DockerDesktop
```

### Windows (WSL2 — recommended)

```powershell
wsl --install -d Ubuntu
# Then follow Ubuntu/Debian setup above inside WSL
```

## Optional sandbox dependencies

Archon can route approved Bash commands through Docker, SSH, or OpenShell when
configured. These are not required for normal chat, docs, memory, providers, or
pipeline use.

```bash
# Linux/macOS from this repo clone:
sudo scripts/install-system-deps.sh --with-docker
sudo scripts/install-system-deps.sh --with-openshell

# Check without changing the machine:
scripts/install-system-deps.sh --check --with-sandbox
archon sandbox doctor --backend docker
archon sandbox doctor --backend openshell
```

Docker installs use the host package manager where possible (`docker.io`,
`moby-engine`/`docker-cli`, `docker`, or Docker Desktop on macOS). On Linux you
may still need to add your user to the `docker` group and start/enable the
daemon according to your distro policy.

OpenShell installs with NVIDIA's official install script:

```bash
curl -LsSf https://raw.githubusercontent.com/NVIDIA/OpenShell/main/install.sh | sh
```

OpenShell's local gateway path expects Docker to be available, so
`--with-openshell` also installs/checks Docker. The installer only enables
OpenShell on hosts covered by NVIDIA's current support matrix: Debian/Ubuntu
Linux on x86_64/aarch64, WSL2 Debian/Ubuntu on x86_64, and macOS Apple Silicon.
Use Docker or SSH sandboxing on other hosts unless you install and validate
OpenShell manually.

OpenShell gateway setup is per-user state. Run this as your normal user, not
with `sudo`, after Docker Desktop or Docker Engine is running:

```bash
scripts/install-system-deps.sh --with-openshell --setup-openshell-gateway
```

The setup step verifies Docker and OpenShell, refreshes OpenShell through the
official installer when gateway setup is requested, reuses an already-active
gateway, and starts/registers the local gateway service when needed. On macOS
that is the Homebrew service for `nvidia/openshell/openshell`; on Linux it is
the user `openshell-gateway` systemd service registered at
`https://127.0.0.1:17670`. Older CLI builds that still expose
`openshell gateway start` are handled as a fallback.

## Clone and build

```bash
git clone https://github.com/ste-bah/archon-cli
cd archon-cli
cargo build --release --bin archon
```

Release binary lands at `target/release/archon` (~66 MB).

For an incremental dev build (faster compile, larger binary, debug symbols):
```bash
cargo build --bin archon
./target/debug/archon --version
```

## WSL2 caveat — parallelism limit

If you are building inside WSL2, do NOT let cargo run rustc in parallel against the full 26-crate workspace. WSL2's memory pressure on multi-process compilation has caused OOM kills.

```bash
cargo build --release --bin archon -j1
```

Build time on WSL2 with `-j1` is ~3-4 minutes; without it can OOM the entire WSL2 VM. Native Linux and macOS do not need this flag.

## Install to PATH

```bash
# Linux/macOS
sudo cp target/release/archon /usr/local/bin/

# Or via cargo install (Linux/macOS/Windows)
cargo install --path .
# Installs to ~/.cargo/bin/archon — make sure ~/.cargo/bin is in PATH
```

## Initialise a blank project directory

Archon is usually run from the project root you want it to inspect. For a
blank project, create the directory first, initialise it, then launch Archon
from inside that directory:

```bash
mkdir -p ~/projects/my-archon-project
sh scripts/archon-init.sh \
  --target ~/projects/my-archon-project \
  --archon-cli-repo "$(pwd)"
cd ~/projects/my-archon-project
archon
```

Creates `.archon/`, `prds/`, `tasks/` and wires `.gitignore`. Safe to re-run.

The install scripts split responsibilities:

| Script | What it does | What it does not do |
|---|---|---|
| `scripts/install-system-deps.sh` | Installs/checks OS packages needed for building, PDF ingest, OCR, and optional sandbox backends | Does not initialise a project, write config, install provider credentials, or set up the web UI |
| `scripts/archon-init.sh` | Initialises an existing project directory with `.archon/`, `prds/`, `tasks/`, policy defaults, docs inboxes, and optional bundled assets | Does not create the target directory, install system packages, authenticate providers, or build the binary |

If you already have a project directory, skip `mkdir -p` and point
`--target` at that existing path. If you installed only a binary and do not
have this repository clone, download the init script instead:

```bash
cd ~/projects/my-archon-project
curl -L https://raw.githubusercontent.com/ste-bah/archon-cli/main/scripts/archon-init.sh -o archon-init.sh
chmod +x archon-init.sh
./archon-init.sh
```

Evidence Engine projects also get:

| Path | Purpose |
|---|---|
| `.archon/policy.toml` | Safe local-first policy defaults for OCR, VLM, retrieval, learning, and game-theory gates |
| `.archon/specs/` | Project routing/spec files; `gametheory.yaml` is copied when `--archon-cli-repo` points at this clone |
| `.archon/docs/inbox/` | Optional drop zone for PDFs, DOCX, Markdown, text, PNG/JPEG/TIFF before `archon docs ingest` |
| `.archon/evidence/` | Manual verification transcripts and evidence artifacts |

Add documents with the CLI, then inspect them from CLI or TUI:

```bash
archon docs ingest .archon/docs/inbox
archon docs status
archon docs inspect <document-id>
archon docs index --all
archon docs search "known phrase" --mode hybrid --debug
```

Inside the TUI:

```text
/docs open
/docs list
/docs inspect <document-id>
/docs provenance <chunk-or-artifact-id>
```

See [Project setup](project-setup.md) for full details — flags, scenarios, what's created vs not, exit codes, troubleshooting.

## Set up the web workbench

The browser workbench is embedded in the `archon` binary from `web/dist`.
Normal users do not need Node.js, Vite, or a separate web install. Launch it
from the project root after `archon-init.sh` so it can inspect that project's
`.archon/` state:

```bash
cd ~/projects/my-archon-project
archon web --port 8421 --bind-address 127.0.0.1
```

By default this opens `http://localhost:8421`. Use `--no-open` when running
under WSL, SSH, or a headless environment and open the URL manually.

The web workbench shows the same local state as the CLI/TUI: chat, uploads,
corpus/doc ingestion results, memory and learning rows, reasoning-quality
events, world-model data, pipelines, metrics, settings, and the evidence graph.
It does not create project scaffolding itself; run `archon-init.sh` first for a
blank project. See [Web workbench](../operations/web-workbench.md) for the tab
guide, data sources, action safety model, and troubleshooting.

Configure defaults in `~/.config/archon/config.toml` or
`<project>/.archon/config.toml`:

```toml
[web]
port = 8421
bind_address = "127.0.0.1"
open_browser = true
```

Binding to `127.0.0.1` is the safe default and does not require a token. Binding
to a non-loopback address causes Archon to create/use a bearer token; only do
that behind a trusted network boundary or reverse proxy. See
[Remote control](../operations/remote-control.md#web-ui) for the security
notes.

If you are developing the web UI itself, install Node 22+ and rebuild the
frontend before rebuilding the Rust binary:

```bash
cd web
npm install
npm run build
cd ..
cargo build --release --bin archon
```

For Windows native (PowerShell):
```powershell
$env:PATH += ";$PWD\target\release"
# Or copy archon.exe to a directory already in PATH
```

## Verify the build

```bash
archon --version
# Expected: archon 1.3.0 (<short-sha>)

archon --help                   # full subcommand listing
archon --list-themes            # 23 themes available
archon --list-output-styles     # 5 output styles available
```

## Run the test suite (optional)

```bash
# Native Linux/macOS — full parallelism
cargo test --workspace

# WSL2 — restrict parallelism
cargo test --workspace -j1 -- --test-threads=2

# Faster, prettier output via nextest
cargo install cargo-nextest
cargo nextest run --workspace -j1 -- --test-threads=2
```

## Common build problems

| Symptom | Cause | Fix |
|---|---|---|
| `error: package 'archon-cli-workspace' specifies edition 2024` | Rust < 1.85 | `rustup update stable` |
| `failed to resolve openssl` on Linux | missing `libssl-dev` | install OS build deps (above) |
| WSL2 build hangs then `Killed` / `signal: 9` | OOM during parallel rustc | rebuild with `cargo build --release -j1` |
| `linker 'cc' not found` | missing C toolchain | install `build-essential` (Linux) or Xcode CLI tools (macOS) |
| Long build (30+ min on first run) | full dependency graph fetch + compile | normal for first build; rebuilds use the incremental cache |
| `error: linking with cc failed` after a long compile | linker memory exhaustion | install `lld` and add `RUSTFLAGS='-Clink-arg=-fuse-ld=lld'`, or switch to `-j1` |
| rustc ICE on `petgraph::graphmap::NeighborsDirected::next` | stale dep cache | `cargo clean -p petgraph -p archon-pipeline` then rebuild |

## Build flags reference

| Flag | Purpose |
|---|---|
| `--release` | optimized build (slow compile, fast runtime, ~66 MB output) |
| `--bin archon` | only build the `archon` CLI binary — skips test/example targets and other workspace bins for faster iteration |
| `-j1` | restrict cargo to one rustc process (mandatory on WSL2 with <8 GB RAM) |
| `--offline` | reuse the local registry cache; do NOT fetch new crates |
| `--profile dev` | default debug build; preserves debug symbols, no optimization |

## Authentication

Anthropic credentials are resolved in this order:

### OAuth (recommended for Claude subscribers)

```bash
archon auth login --provider anthropic
```

PKCE OAuth flow in your browser, exchanges authorization code for tokens, stored at `~/.archon/.credentials.json`. Tokens refresh automatically with file locking to prevent race conditions across concurrent sessions. Re-run `archon auth login --provider anthropic` to re-authenticate; `archon auth logout --provider anthropic` (or `/auth logout --provider anthropic` in the TUI) signs out. The legacy `archon login` / `archon logout` commands still route to the Anthropic provider.

### API key

```bash
export ANTHROPIC_API_KEY="sk-ant-..."
# or ARCHON_API_KEY (alias)
```

### Pre-set bearer token

```bash
export ARCHON_OAUTH_TOKEN="..."
# or ANTHROPIC_AUTH_TOKEN (legacy alias)
```

The OAuth flow matches the original Claude Code client (`redirect_uri = http://localhost:{port}/callback`), so existing Claude Code tokens on the same machine work transparently.

### Codex OAuth

```bash
archon auth login --provider openai-codex
```

Codex credentials are stored in the same `~/.archon/.credentials.json` file under `openaiCodexOauth`. Set `[llm].provider = "openai-codex"` to use Codex for the TUI, tools, subagents, `/btw`, team runs, coding/research pipelines, and game-theory runs.

## VS Code Extension

The repo ships a VS Code extension under
[`extensions/vscode/`](../../extensions/vscode/) that wraps the
`archon ide-stdio` JSON-RPC backend as a chat panel. Build and install
from source per [`extensions/vscode/README.md`](../../extensions/vscode/README.md).
Marketplace publication is pending.

## Next steps

- [First run](first-run.md) — what data archon writes, where logs go
- [Configuration](../reference/config.md) — `~/.config/archon/config.toml` schema
- [Quick start](quick-start.md) — 5-minute path to first agent
