//! Public-API drift guard for `archon_core::agents::memory::*`.
//!
//! Reference: project-tasks/archon-fixes/agentshit/phase-0-prereqs/TASK-AGS-011.md
//! Based on:  00-prd-analysis.md REQ-FOR-PRESERVE-D8 (d), NFR-ARCH-002 (backward compat N-1)
//!
//! ## What this test enforces
//!
//! The sub-surface `archon_core::agents::memory::*` is a
//! PRESERVE-D8 frozen API — phase-1..9 refactors may add items, but
//! must never rename, remove, or change the signature of an existing
//! public item without an accompanying approved regen of the
//! `tests/fixtures/baseline/agents_memory_api.txt` snapshot.
//!
//! ## How it works
//!
//! Two gated modes:
//!
//! 1. DEFAULT (unset env): `include_str!` the committed fixture,
//!    assert it is non-empty, starts with a `# cargo-public-api ...`
//!    header line, and contains at least one preserve-D8 anchor item
//!    (`save_agent_memory`). This runs on every `cargo test` — no
//!    nightly toolchain, no `cargo-public-api` binary, no cargo-in-
//!    cargo lock contention.
//!
//! 2. DRIFT MODE (`ARCHON_RUN_PUBLIC_API_DRIFT=1`): additionally
//!    shells out to `cargo public-api --package archon-core
//!    --simplified`, filters to the `archon_core::agents::memory::`
//!    sub-tree, prefixes the tool-version header, and asserts
//!    byte-identical match against the fixture. Requires nightly +
//!    cargo-public-api on PATH — panics with a "run
//!    scripts/regen-public-api.sh" hint on drift.
//!
//! Drift mode is opt-in because running `cargo public-api` from
//! inside a `cargo test` run means a nested cargo build against the
//! workspace target dir, which deadlocks on WSL2 without careful
//! isolation. Phase-1..9 CI runs will set the env var with a
//! dedicated `CARGO_TARGET_DIR` override.

const FIXTURE: &str = include_str!("../../../tests/fixtures/baseline/agents_memory_api.txt");

/// Must be present in the snapshot — `save_agent_memory` is the
/// documented preserve-D8 anchor (REQ-FOR-PRESERVE-D8 §entry-point).
const ANCHOR: &str = "archon_core::agents::memory::save_agent_memory";

#[test]
fn test_fixture_non_empty_and_headered() {
    assert!(!FIXTURE.is_empty(), "fixture file is empty");
    let first = FIXTURE.lines().next().expect("no first line");
    assert!(
        first.starts_with("# cargo-public-api "),
        "fixture must start with '# cargo-public-api <version>' header, got: {first:?}. \
         Run scripts/regen-public-api.sh."
    );
    assert!(
        FIXTURE.contains(ANCHOR),
        "fixture missing preserve-D8 anchor {ANCHOR}. \
         Run scripts/regen-public-api.sh."
    );
    assert!(
        FIXTURE.lines().count() <= 2000,
        "fixture exceeds 2000-line sanity cap — something is wrong with the filter"
    );
}

#[test]
fn test_public_api_drift() {
    if std::env::var_os("ARCHON_RUN_PUBLIC_API_DRIFT").is_none() {
        eprintln!(
            "SKIP test_public_api_drift: set ARCHON_RUN_PUBLIC_API_DRIFT=1 to \
             run live drift check (requires nightly + cargo-public-api)."
        );
        return;
    }

    let tool_version = match std::process::Command::new("cargo-public-api")
        .arg("--version")
        .output()
    {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout)
            .lines()
            .next()
            .unwrap_or("")
            .to_string(),
        _ => {
            eprintln!("SKIP: cargo-public-api not on PATH");
            return;
        }
    };

    let output = std::process::Command::new("cargo")
        .args(["public-api", "--package", "archon-core", "--simplified"])
        .env("CARGO_BUILD_JOBS", "1")
        .output()
        .expect("failed to invoke cargo public-api");
    assert!(
        output.status.success(),
        "cargo public-api failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let raw = String::from_utf8(output.stdout).expect("non-utf8 output");
    let filtered: String = raw
        .lines()
        .filter(|l| l.contains("archon_core::agents::memory::"))
        .collect::<Vec<_>>()
        .join("\n");

    let mut rebuilt = format!("# {tool_version}\n{filtered}");
    if !rebuilt.ends_with('\n') {
        rebuilt.push('\n');
    }

    if rebuilt != FIXTURE {
        panic!(
            "agents_memory_api.txt drift detected. Run `bash scripts/regen-public-api.sh` \
             and review the diff before committing."
        );
    }
}
