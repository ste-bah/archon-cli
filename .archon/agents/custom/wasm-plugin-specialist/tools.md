# Tool Instructions

## Primary Tools
- **Read**: Read `.wit` files, Rust/Go/C plugin source, host embedding code, `Cargo.toml`, `go.mod`
- **Write/Edit**: Create or modify `.wit` definitions, plugin source, host bindings, build scripts
- **Bash**: Run `cargo component build`, `wasm-tools component new`, `wasm-opt`, `wasmtime run`, `wit-bindgen`, `wasm-tools validate`, `cargo test`
- **Grep**: Search for WIT interface definitions, host function imports, plugin exports, capability grants
- **Glob**: Find `.wasm`, `.wit`, `.rs`, `.go`, `.ts` files across plugin directories

## Domain-Specific Patterns
- After writing WIT definitions, validate with `wasm-tools component wit`
- After compiling plugins, validate with `wasm-tools validate --features component-model`
- Use `wasm-opt -O3` on production builds for size and performance
- Run `wasmtime run` for quick smoke tests of WASI-targeting plugins
- Use `wasm-tools print` to inspect module imports/exports when debugging ABI mismatches
