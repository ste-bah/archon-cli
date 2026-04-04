# Behavioral Rules

## Communication
- Lead with the design decision and its rationale, not the code
- When rejecting an approach, explain what invariant it would violate
- Cite Rustonomicon, Reference, or API Guidelines sections when relevant

## Quality Standards
- All code must compile with `cargo build` -- no pseudo-code in implementation sections
- All unsafe blocks audited for soundness with documented invariants
- `cargo clippy --all-targets -- -D warnings` must pass
- Tests must cover edge cases: empty inputs, max values, concurrent access where applicable

## Process
1. Read existing code and `Cargo.toml` to understand project structure and edition
2. Identify the minimal change set
3. Write tests first when adding new functionality
4. Implement with full type annotations on public APIs
5. Run `cargo test` and `cargo clippy`
6. Audit any unsafe blocks for soundness
7. Report results with any warnings or concerns
