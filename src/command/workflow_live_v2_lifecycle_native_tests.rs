//! Unit-test entry points bypass application startup; register the real native CLI.
use archon_workflow::v2::deliverable_contract::{self, ContractRoots};
use std::path::PathBuf;

pub(super) fn initialize() {
    let program = std::env::var_os("CARGO_BIN_EXE_archon")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::current_exe()
                .expect("test executable")
                .parent()
                .and_then(|path| path.parent())
                .expect("test executable under build output")
                .join(format!("archon{}", std::env::consts::EXE_SUFFIX))
        });
    assert!(
        program.is_file(),
        "native verifier executable is missing at {}; build --bin archon first",
        program.display()
    );
    // D-S7-10 uses one native kernel. Match main's startup without changing
    // the generated command or replacing its verdict in the canned agent.
    deliverable_contract::set_verifier_program(Ok(program));
}

#[test]
fn native_verifier_fixture_executes_passing_and_failing_floor_commands() {
    use std::io::Write;
    use std::process::{Command, Stdio};

    initialize();
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("present.json"), "{\"value\":1}").unwrap();
    std::fs::write(directory.path().join("empty.json"), "").unwrap();
    let roots = ContractRoots::project_only(directory.path().to_string_lossy());
    for (artifact, passed) in [
        ("present.json", true),
        ("missing.json", false),
        ("empty.json", false),
    ] {
        let command = deliverable_contract::verification_command(
            &roots,
            &serde_json::json!({
                "kind":"evidence", "artifact_path":artifact,
                "artifact_status":{"exists":!passed,"bytes":0}
            }),
        );
        let mut child = Command::new(archon_shell::resolve_posix_shell())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(command.as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert_eq!(output.status.success(), passed, "{output:?}");
        let verdict: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        if passed {
            assert_eq!(verdict["status"], "declared_deliverable_present");
        } else {
            assert_eq!(verdict["status"], "failed");
            assert!(
                verdict["failures"][0]
                    .as_str()
                    .unwrap()
                    .contains("missing or empty")
            );
        }
    }
}
