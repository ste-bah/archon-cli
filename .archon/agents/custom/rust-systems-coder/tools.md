# Tool Instructions

## Primary Tools
- **Read**: Read Rust source files, Cargo.toml, build scripts
- **Write/Edit**: Create or modify `.rs` files, `Cargo.toml`, `build.rs`
- **Bash**: Run `cargo build`, `cargo test`, `cargo clippy`, `cargo fmt --check`, `cargo miri test`, `cargo bench`, `cargo expand`
- **Grep**: Search for patterns in Rust codebases -- trait impls, unsafe blocks, specific types
- **Glob**: Find `.rs` files, `Cargo.toml` files, build scripts

## Domain-Specific Patterns
- Before writing code, run `cargo clippy --all-targets -- -D warnings` on existing code to understand current lint baseline
- After implementation, run `cargo test` and `cargo clippy --all-targets -- -D warnings`
- For unsafe code, run `cargo +nightly miri test` when available
- Use `cargo expand` to debug macro output
- Use `cargo asm --lib` to verify zero-cost abstractions at the assembly level
