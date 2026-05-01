# Installation

archon-cli is a 21-crate Cargo workspace. There are no precompiled binaries — clone, install Rust, build with `cargo build --release`. End-to-end build time is ~3-4 minutes on a modern laptop, longer on WSL2.

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

### Ubuntu / Debian / WSL2-Ubuntu

```bash
sudo apt update
sudo apt install -y build-essential pkg-config libssl-dev git
```

### Fedora / RHEL / Rocky

```bash
sudo dnf install -y gcc pkg-config openssl-devel git
```

### Arch / Manjaro

```bash
sudo pacman -S --needed base-devel openssl pkg-config git
```

### macOS

```bash
xcode-select --install
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
```

### Windows (WSL2 — recommended)

```powershell
wsl --install -d Ubuntu
# Then follow Ubuntu/Debian setup above inside WSL
```

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

If you are building inside WSL2, do NOT let cargo run rustc in parallel against the full 21-crate workspace. WSL2's memory pressure on multi-process compilation has caused OOM kills.

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

## Initialise a project

```bash
sh scripts/archon-init.sh --target /path/to/your/project
# Or with the curated assets from this clone:
sh scripts/archon-init.sh --target /path/to/your/project --archon-cli-repo $(pwd)
```

Creates `.archon/`, `prds/`, `tasks/` and wires `.gitignore`. Safe to re-run.

See [Project setup](project-setup.md) for full details — flags, scenarios, what's created vs not, exit codes, troubleshooting.

For Windows native (PowerShell):
```powershell
$env:PATH += ";$PWD\target\release"
# Or copy archon.exe to a directory already in PATH
```

## Verify the build

```bash
archon --version
# Expected: archon 0.1.28 (<short-sha>)

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

Three methods, tried in this order:

### OAuth (recommended for Claude subscribers)

```bash
archon login
```

PKCE OAuth flow in your browser, exchanges authorization code for tokens, stored at `~/.config/archon/oauth.json`. Tokens refresh automatically with file locking to prevent race conditions across concurrent sessions. Re-run `archon login` to re-authenticate; `archon logout` (or `/logout` in the TUI) signs out.

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

## Next steps

- [First run](first-run.md) — what data archon writes, where logs go
- [Configuration](../reference/config.md) — `~/.config/archon/config.toml` schema
- [Quick start](quick-start.md) — 5-minute path to first agent
