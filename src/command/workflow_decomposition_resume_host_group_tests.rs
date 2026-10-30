use super::*;

#[cfg(unix)]
#[tokio::test]
async fn fixed_resume_refusal_names_the_run_and_live_group_pid() {
    let project = fixture_project();
    let (store, run_id, _) =
        super::workflow_decomposition_drift_tests::launch_and_pause(project.path()).await;
    let mut child = archon_shell::spawn::command("sleep")
        .arg("30")
        .spawn()
        .expect("child");
    let pid = child.id();
    let start = archon_shell::process_tree::start_of(pid).expect("child identity");
    let ended = archon_shell::spawn::command("true")
        .spawn()
        .expect("ended group");
    let pgid = ended.id();
    ended.wait_with_output().expect("wait for ended group");
    let guard = crate::command::workflow_host_command_groups::record_group(
        &store
            .run_dir(&run_id)
            .join(crate::command::workflow_host_command_groups::GROUP_RECORDS_DIR),
        pgid,
        pgid,
        Some(pgid),
        None,
        "AC-X-001",
    )
    .expect("group record");
    guard.keep(Some(&[(pid, start)]));

    let error = resume_fixed_decomposition_with_factory(
        project.path(),
        &run_id,
        true,
        &launch_config(project.path()),
        &empty_env(),
        &PanicFactory {
            builds: AtomicUsize::new(0),
        },
    )
    .await
    .unwrap_err()
    .to_string();
    let _ = child.kill();
    let _ = child.wait();

    assert!(
        error.contains(&format!("run {run_id} cannot resume")),
        "{error}"
    );
    assert!(error.contains(&format!("pid {pid}")), "{error}");
    assert!(!error.contains("fixed decomposition"), "{error}");
}

#[cfg(unix)]
#[tokio::test]
async fn dry_run_refusal_is_run_neutral_and_preserves_records() {
    let project = fixture_project();
    let (store, run_id, _) =
        super::workflow_decomposition_drift_tests::launch_and_pause(project.path()).await;
    let mut child = archon_shell::spawn::command("sleep")
        .arg("30")
        .spawn()
        .expect("child");
    let pid = child.id();
    let start = archon_shell::process_tree::start_of(pid).expect("child identity");
    let ended = archon_shell::spawn::command("true")
        .spawn()
        .expect("ended group");
    let pgid = ended.id();
    ended.wait_with_output().expect("wait for ended group");
    let records = store
        .run_dir(&run_id)
        .join(crate::command::workflow_host_command_groups::GROUP_RECORDS_DIR);
    let guard = crate::command::workflow_host_command_groups::record_group(
        &records,
        pgid,
        pgid,
        Some(pgid),
        None,
        "AC-X-001",
    )
    .expect("group record");
    guard.keep(Some(&[(pid, start)]));
    let listing = |dir: &Path| {
        let mut files = std::fs::read_dir(dir)
            .expect("records")
            .map(|entry| {
                let path = entry.expect("entry").path();
                let bytes = std::fs::read(&path).expect("record bytes");
                (path, bytes)
            })
            .collect::<Vec<_>>();
        files.sort();
        files
    };
    let before = listing(&records);

    let error = crate::command::workflow_decompose::dry_run_fixed_decomposition(
        project.path(),
        &run_id,
        &launch_config(project.path()),
    )
    .await
    .unwrap_err()
    .to_string();
    let after = listing(&records);
    let _ = child.kill();
    let _ = child.wait();

    assert!(
        error.contains(&format!("run {run_id} cannot resume")),
        "{error}"
    );
    assert!(error.contains(&format!("pid {pid}")), "{error}");
    assert!(!error.contains("fixed decomposition"), "{error}");
    assert_eq!(
        before, after,
        "dry-run admission must not touch the records"
    );
}
