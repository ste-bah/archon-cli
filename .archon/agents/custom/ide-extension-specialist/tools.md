# Tool Instructions

## Primary Tools
- **Read**: Read existing Rust, TypeScript, Kotlin, JSON source files
- **Write/Edit**: Create extension files, SDK code, Rust handlers, Gradle/npm configs
- **Bash**: `cargo build`, `cargo test --test-threads=1`, `npm run build`, `./gradlew build`
- **Grep**: Search for existing patterns, archon-sdk usage, dispatch.rs structure
- **Glob**: Find existing source files in crates/archon-sdk/, extensions/

## Domain-Specific Patterns
- Before implementing Rust: read `crates/archon-sdk/src/lib.rs` and `crates/archon-core/src/dispatch.rs`
- Before implementing TS/Kotlin SDK: read the protocol spec from task definition
- For Rust: use `--test-threads=1` for ALL cargo test (WSL memory constraint)
- For TypeScript: `npm run build` in the extension directory to check type errors
- For Kotlin: `./gradlew build` in the plugin directory to check compilation
- Verify npm/node available before using: `which node && node --version`
- VS Code extension: check `vscode` engine version in package.json matches target
- JetBrains: check `since-build`/`until-build` in plugin.xml for compatibility range
