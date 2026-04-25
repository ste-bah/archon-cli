//! TC-ARCH-06 (US-ARCH-04, e2e): D10 architectural lint CI simulation.
//!
//! Injects a synthetic `.process_message().await` violation into a temp copy
//! of src/main.rs, runs arch-lint.sh against it, and asserts:
//! - Exit status is non-zero
//! - Output contains "spawn-everything-philosophy.md" (guideline reference)

use std::io::Write;
use std::process::Command;

#[ignore = "TDD test for AGS-106/107 arch-lint integration sim; tracked under #224 (CI cross-platform parity with P1.1 canary skip list)"]
#[test]
fn lint_catches_injected_violation() {
    let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let main_rs_path = repo_root.join("src/main.rs");
    let lint_script = repo_root.join("scripts/lint/arch-lint.sh");

    let original = std::fs::read_to_string(&main_rs_path).expect("failed to read src/main.rs");

    // Find the BEGIN INPUT_HANDLER marker line
    let begin_marker = "BEGIN INPUT_HANDLER";
    let marker_pos = original
        .find(begin_marker)
        .expect("BEGIN INPUT_HANDLER marker not found in src/main.rs");

    // Find end of the marker line
    let line_end = original[marker_pos..]
        .find('\n')
        .expect("no newline after marker")
        + marker_pos;

    // Inject a synthetic violation right after the marker line
    // (at handler body indentation = 12 spaces, which is handler scope)
    let violation = "            agent.process_message(&input).await; // SYNTHETIC VIOLATION\n";
    let mutated = format!(
        "{}\n{}{}",
        &original[..line_end],
        violation,
        &original[line_end + 1..]
    );

    // Write mutated file to a temp dir, with the same relative path structure
    // so arch-lint.sh can find it
    let tmp_dir = tempfile::TempDir::new().expect("failed to create temp dir");
    let tmp_src_dir = tmp_dir.path().join("src");
    std::fs::create_dir_all(&tmp_src_dir).expect("failed to create src dir");
    let tmp_main = tmp_src_dir.join("main.rs");
    {
        let mut f = std::fs::File::create(&tmp_main).expect("failed to create temp main.rs");
        f.write_all(mutated.as_bytes())
            .expect("failed to write temp main.rs");
    }

    // Copy other files that arch-lint.sh needs for Rule 2
    let agent_src = repo_root.join("crates/archon-core/src/agent.rs");
    if agent_src.exists() {
        let dst_dir = tmp_dir.path().join("crates/archon-core/src");
        std::fs::create_dir_all(&dst_dir).expect("failed to create agent.rs dir");
        std::fs::copy(&agent_src, dst_dir.join("agent.rs")).expect("failed to copy agent.rs");
    }

    // Copy the lint script and philosophy doc
    let dst_lint_dir = tmp_dir.path().join("scripts/lint");
    std::fs::create_dir_all(&dst_lint_dir).expect("failed to create lint dir");
    std::fs::copy(&lint_script, dst_lint_dir.join("arch-lint.sh"))
        .expect("failed to copy arch-lint.sh");

    let philosophy_doc = repo_root.join("docs/architecture/spawn-everything-philosophy.md");
    if philosophy_doc.exists() {
        let dst_doc_dir = tmp_dir.path().join("docs/architecture");
        std::fs::create_dir_all(&dst_doc_dir).expect("failed to create docs dir");
        std::fs::copy(
            &philosophy_doc,
            dst_doc_dir.join("spawn-everything-philosophy.md"),
        )
        .expect("failed to copy philosophy doc");
    }

    // Also copy tui app.rs for Rule 3 (it checks this file)
    let tui_app = repo_root.join("crates/archon-tui/src/app.rs");
    if tui_app.exists() {
        let dst_dir = tmp_dir.path().join("crates/archon-tui/src");
        std::fs::create_dir_all(&dst_dir).expect("failed to create tui dir");
        std::fs::copy(&tui_app, dst_dir.join("app.rs")).expect("failed to copy app.rs");
    }

    // Run arch-lint.sh from the temp dir
    let output = Command::new("bash")
        .arg("scripts/lint/arch-lint.sh")
        .current_dir(tmp_dir.path())
        .output()
        .expect("failed to execute arch-lint.sh");

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let combined = format!("{stdout}{stderr}");

    // Must exit non-zero
    assert!(
        !output.status.success(),
        "TC-ARCH-06: arch-lint.sh should have failed on injected violation but exited 0.\n\
         stdout: {stdout}\nstderr: {stderr}"
    );

    // Must reference the philosophy doc
    assert!(
        combined.contains("spawn-everything-philosophy.md"),
        "TC-ARCH-06: output should reference spawn-everything-philosophy.md.\n\
         stdout: {stdout}\nstderr: {stderr}"
    );
}
