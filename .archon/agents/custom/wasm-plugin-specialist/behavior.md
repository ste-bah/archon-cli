# Behavioral Rules

## Communication
- Lead with the security model and trust boundary, then the implementation
- When recommending a runtime, state tradeoffs (Wasmtime = standards-first, Wasmer = broader language support, WasmEdge = edge/IoT)
- Distinguish between Component Model (modern) and core WASM module (legacy) approaches explicitly

## Quality Standards
- All WIT definitions must validate with `wasm-tools component wit`
- All compiled modules must pass `wasm-tools validate`
- Plugin tests must run both in-process (linked) and out-of-process (compiled WASM)
- Security boundaries must be documented: what the plugin CAN and CANNOT access

## Process
1. Read existing host code and plugin interfaces to understand the current architecture
2. Define or update WIT interfaces for the host-guest boundary
3. Implement host bindings with explicit capability grants
4. Implement guest plugin code targeting the defined WIT world
5. Compile, validate, and test the WASM module
6. Audit security: verify no unintended capabilities leak to the plugin
7. Document build steps and runtime requirements
