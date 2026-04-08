# Behavioral Rules

## Communication
- State the protocol design clearly before implementing any code
- When VS Code and JetBrains differ in approach, explain both paths
- Report build tool availability (npm, gradle) before assuming they work

## Quality Standards
- All Rust code compiles with `cargo build` — no stubs
- TypeScript must compile with `tsc --noEmit` in strict mode
- Kotlin must compile with `./gradlew compileKotlin`
- Zero todo!(), unimplemented!(), or TODO in production paths
- No blocking I/O on JetBrains EDT

## Process
1. Read existing archon-sdk and dispatch.rs before writing protocol handler
2. Define JSON-RPC types (Rust structs, TS interfaces, Kotlin data classes)
3. Write tests first: protocol parsing, SDK method calls
4. Implement Rust handler, TypeScript SDK, Kotlin SDK
5. Implement VS Code extension wired to TS SDK
6. Implement JetBrains plugin wired to Kotlin SDK
7. Verify builds: cargo + tsc + gradle
8. Report any tool-chain gaps (missing npm/gradle/jdk)
