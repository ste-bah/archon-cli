use super::*;

#[test]
fn verify_success_records_command() {
    let repo = canonical_repo();
    let v = run_wave_verify(
        repo.path(),
        Some("printf ok"),
        1,
        &run_root_of(repo.path()),
        "impl",
    )
    .expect("verify");
    assert_eq!(v.exit, 0);
    assert_eq!(v.command.as_deref(), Some("printf ok"));
    assert_eq!(v.stdout_tail, "ok");
    if let Some(note) = &v.environment_note {
        assert!(note.contains("Filesystem read boundary:"));
        assert_eq!(
            archon_shell::read_boundary::observation_note(note.clone()),
            Some(note.clone())
        );
    }
    let persisted: serde_json::Value = serde_json::from_slice(
        &std::fs::read(
            run_root_of(repo.path()).join("write-coordination/stages/impl/tests/1.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        persisted["environment_note"],
        serde_json::to_value(v.environment_note).unwrap()
    );
}
