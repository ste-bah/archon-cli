//! TASK-AGS-100: D10 Architectural Philosophy Document + CI Lint Scaffold
//!
//! Validates:
//! - docs/architecture/spawn-everything-philosophy.md exists and contains the
//!   three D10 rules and the main.rs:3743 smoking-gun reference
//! - scripts/lint/arch-lint.sh exists, is executable, and exits 0 in its
//!   commented-out (scaffold) state
//! - .github/workflows/ci.yml wires an `arch-lint` job
//! - CONTRIBUTING.md references the philosophy document

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    // tests/ lives at the workspace root
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(path: impl AsRef<Path>) -> String {
    let p = repo_root().join(path.as_ref());
    fs::read_to_string(&p).unwrap_or_else(|e| panic!("failed to read {}: {e}", p.display()))
}

#[test]
fn philosophy_doc_exists_and_has_three_rules() {
    let body = read("docs/architecture/spawn-everything-philosophy.md");
    // Rule 1: no .await >100ms in main event handler
    assert!(
        body.contains("no .await >100ms"),
        "philosophy doc must contain verbatim rule 1: 'no .await >100ms'"
    );
    // Rule 2: producer channels are unbounded
    assert!(
        body.contains("producer channels are unbounded")
            || body.contains("producer channels unbounded"),
        "philosophy doc must state rule 2: producer channels unbounded"
    );
    // Rule 3: tools own task lifecycle
    assert!(
        body.contains("tools own task lifecycle"),
        "philosophy doc must state rule 3: tools own task lifecycle"
    );
}

#[test]
fn philosophy_doc_references_smoking_gun() {
    let body = read("docs/architecture/spawn-everything-philosophy.md");
    assert!(
        body.contains("main.rs:3743"),
        "philosophy doc must reference the main.rs:3743 smoking gun"
    );
}

#[test]
fn arch_lint_script_exists_and_is_executable() {
    let script = repo_root().join("scripts/lint/arch-lint.sh");
    assert!(script.exists(), "scripts/lint/arch-lint.sh must exist");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let meta = fs::metadata(&script).expect("stat arch-lint.sh");
        let mode = meta.permissions().mode();
        assert!(
            mode & 0o111 != 0,
            "scripts/lint/arch-lint.sh must be executable (mode={:o})",
            mode
        );
    }
}

#[test]
fn arch_lint_script_exits_zero_in_scaffold_state() {
    let script = repo_root().join("scripts/lint/arch-lint.sh");
    let status = Command::new("bash")
        .arg(&script)
        .current_dir(repo_root())
        .status()
        .expect("failed to run arch-lint.sh");
    assert!(
        status.success(),
        "arch-lint.sh scaffold must exit 0 (activation deferred to TASK-AGS-110)"
    );
}

#[test]
fn ci_workflow_wires_arch_lint_job() {
    let body = read(".github/workflows/ci.yml");
    assert!(
        body.contains("arch-lint"),
        ".github/workflows/ci.yml must declare an arch-lint job"
    );
    assert!(
        body.contains("scripts/lint/arch-lint.sh"),
        ".github/workflows/ci.yml arch-lint job must invoke scripts/lint/arch-lint.sh"
    );
}

#[test]
fn contributing_md_links_to_philosophy() {
    let body = read("CONTRIBUTING.md");
    assert!(
        body.contains("spawn-everything-philosophy.md"),
        "CONTRIBUTING.md must link to docs/architecture/spawn-everything-philosophy.md"
    );
}

#[ignore = "TDD test for unimplemented AGS-110 arch-lint activation; tracked under #224 (CI cross-platform parity with P1.1 canary skip list)"]
#[test]
fn arch_lint_script_has_pattern_scaffold() {
    // TASK-AGS-110 will uncomment the activation lines; this test guarantees
    // the scaffold structure exists so the later task is a pure uncomment.
    let body = read("scripts/lint/arch-lint.sh");
    assert!(
        body.contains("agent.process_message") || body.contains("agent\\.process_message"),
        "arch-lint.sh must contain the forbidden-pattern scaffold for TASK-AGS-110"
    );
    assert!(
        body.contains("spawn-everything-philosophy.md"),
        "arch-lint.sh must reference the philosophy doc in its failure message"
    );
}
