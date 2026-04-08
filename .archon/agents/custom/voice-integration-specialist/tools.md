# Tool Instructions

## Primary Tools
- **Read**: Read Rust source files, Cargo.toml, existing TUI/config code
- **Write/Edit**: Create audio capture modules, STT integration, config additions
- **Bash**: `cargo build`, `cargo test --test-threads=1`, `cargo clippy`
- **Grep**: Search for existing config patterns, TUI event handling, voice stubs
- **Glob**: Find audio-related files

## Domain-Specific Patterns
- Before implementing: read existing config.rs to understand ArchonConfig structure
- Before implementing: read TUI event loop to understand how to inject input
- Run `cargo build -p archon-tui` or `archon-core` to catch compile errors early
- Use `--test-threads=1` for ALL cargo test commands (WSL memory constraint — will OOM without this)
- For audio tests: mock the cpal stream with a Vec<f32> source — no real hardware needed in tests
- Check cpal version in Cargo.toml before writing API calls (0.15.x vs 0.16.x differ significantly)
- whisper-rs requires libclang; check if it's in the build environment before depending on it
