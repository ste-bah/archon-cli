//! Public-API drift guard for the `archon-memory` crate.
//!
//! Reference: project-tasks/archon-fixes/agentshit/phase-0-prereqs/TASK-AGS-011.md
//! Based on:  00-prd-analysis.md REQ-FOR-PRESERVE-D8 (d), NFR-ARCH-002
//!
//! See the sister test at
//! `crates/archon-core/tests/public_api_snapshot.rs` for the design
//! rationale (default mode = fixture sanity; drift mode =
//! `ARCHON_RUN_PUBLIC_API_DRIFT=1` shells out to `cargo public-api`).
//!
//! This test covers the FULL public surface of `archon-memory` (no
//! grep filter) — the crate is a stable-for-N-1 dependency of
//! `archon-core::agents::memory` and cannot gain or lose items
//! without an approved regen.

const FIXTURE: &str = include_str!(
    "../../../tests/fixtures/baseline/archon_memory_api.txt"
);

/// MemoryTrait is the preserve-D8 anchor for archon-memory —
/// REQ-FOR-PRESERVE-D8 asserts `save_agent_memory` uses
/// `&dyn archon_memory::access::MemoryTrait`, so removing this
/// trait from the public surface is an instant break.
const ANCHOR: &str = "archon_memory::access::MemoryTrait";

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
        "fixture exceeds 2000-line sanity cap"
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
        Ok(o) if o.status.success() => {
            String::from_utf8_lossy(&o.stdout).lines().next().unwrap_or("").to_string()
        }
        _ => {
            eprintln!("SKIP: cargo-public-api not on PATH");
            return;
        }
    };

    let output = std::process::Command::new("cargo")
        .args([
            "public-api",
            "--package",
            "archon-memory",
            "--simplified",
        ])
        .env("CARGO_BUILD_JOBS", "1")
        .output()
        .expect("failed to invoke cargo public-api");
    assert!(
        output.status.success(),
        "cargo public-api failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let body = String::from_utf8(output.stdout).expect("non-utf8 output");

    let mut rebuilt = format!("# {tool_version}\n{}", body.trim_end());
    if !rebuilt.ends_with('\n') {
        rebuilt.push('\n');
    }

    if rebuilt != FIXTURE {
        panic!(
            "archon_memory_api.txt drift detected. Run `bash scripts/regen-public-api.sh` \
             and review the diff before committing."
        );
    }
}
