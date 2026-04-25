# tests/perf/

Criterion benchmark entry points for the archon-cli workspace.

Each file here is a criterion harness target registered in `Cargo.toml` under
`[[bench]]`. Keep benches small and focused on one hot path; do not place
production logic here. Results are written to `target/criterion/`.
