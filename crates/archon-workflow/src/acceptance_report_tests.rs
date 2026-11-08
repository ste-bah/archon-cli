use super::*;

#[test]
fn successful_verify_keeps_only_incomplete_boundary_report() {
    let root = tempfile::tempdir().unwrap();
    let contained = "Filesystem read boundary: named-paths-denied; operator config/autostart writes: named-paths-denied";
    let incomplete = "Filesystem read boundary: not-contained";
    for boundary in [contained, incomplete] {
        let report = run_verify_with_command(root.path(), Some("printf done"), None, |_| {
            (
                archon_shell::spawn::command(shell_program()),
                boundary.into(),
            )
        })
        .unwrap()
        .unwrap();
        assert_eq!(report.exit_code, Some(0));
        assert_eq!(report.stdout, "done");
        assert_eq!(
            report.environment_note,
            if boundary == contained {
                None
            } else {
                Some(incomplete.into())
            }
        );
    }
}
