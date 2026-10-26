use super::*;

#[cfg(unix)]
#[tokio::test]
async fn restoring_after_an_ancestor_move_does_not_require_the_child_parent_snapshot() {
    let temp = tempfile::tempdir().unwrap();
    let site_path = temp.path().join("site");
    std::fs::create_dir_all(site_path.join("d")).unwrap();
    std::fs::write(site_path.join("d/f"), "original").unwrap();
    let root = super::super::super::mutation_fd::SiteRoot::capture(&site_path).unwrap();
    let nonce = "0123456789abcdef0123456789abcdef";
    let plan = MutationPlan {
        nonce: nonce.into(),
        inputs: vec![
            MutationInputPlan {
                named_path: "d".into(),
                site_copy_alias: false,
                move_path: PathBuf::from("d"),
                aside_path: PathBuf::from("d.archon-mutated-0123456789ab"),
            },
            MutationInputPlan {
                named_path: "d/f".into(),
                site_copy_alias: false,
                move_path: PathBuf::from("d/f"),
                aside_path: PathBuf::from("d/f.archon-mutated-0123456789ab"),
            },
        ],
    };
    let mut command_site = site(&site_path, false, 20);
    command_site.mutation_root = Some(root);
    command_site.mutation_plan = Some(&plan);
    let result = run(&command_site, "test -f d/f && test -d d; exit 1").await;
    let stdout = String::from_utf8_lossy(&result.stdout);
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(stdout.contains(": 0:restored"), "{stdout}");
    assert!(
        stderr.contains("archon-mutation-move-state-0123456789abcdef0123456789abcdef: 1:covered by moved ancestor 'd'"),
        "{stderr}"
    );
    assert!(
        stdout.contains(&format!("archon-mutation-restored-{nonce}")),
        "{stdout}"
    );
    assert_eq!(result.exit_code, Some(1));
    assert_eq!(
        std::fs::read_to_string(site_path.join("d/f")).unwrap(),
        "original"
    );

    let failed_temp = tempfile::tempdir().unwrap();
    let failed_site_path = failed_temp.path().join("site");
    std::fs::create_dir_all(failed_site_path.join("d")).unwrap();
    std::fs::write(failed_site_path.join("d/f"), "original").unwrap();
    std::fs::write(
        failed_site_path.join("d.archon-mutated-123456789012"),
        "occupied",
    )
    .unwrap();
    let failed_root =
        super::super::super::mutation_fd::SiteRoot::capture(&failed_site_path).unwrap();
    let failed_plan = MutationPlan {
        nonce: "12345678901234567890123456789012".into(),
        inputs: vec![
            MutationInputPlan {
                named_path: "d".into(),
                site_copy_alias: false,
                move_path: PathBuf::from("d"),
                aside_path: PathBuf::from("d.archon-mutated-123456789012"),
            },
            MutationInputPlan {
                named_path: "d/f".into(),
                site_copy_alias: false,
                move_path: PathBuf::from("d/f"),
                aside_path: PathBuf::from("d/f.archon-mutated-123456789012"),
            },
        ],
    };
    let mut failed_site = site(&failed_site_path, false, 20);
    failed_site.mutation_root = Some(failed_root);
    failed_site.mutation_plan = Some(&failed_plan);
    let failed_result = run(&failed_site, "test -f d/f; exit 1").await;
    let failed_stderr = String::from_utf8_lossy(&failed_result.stderr);
    assert!(
        failed_stderr.contains("archon-mutation-move-state-12345678901234567890123456789012: 0:"),
        "{failed_stderr}"
    );
    assert!(
        !failed_stderr.contains("archon-mutation-moved-12345678901234567890123456789012: 0"),
        "{failed_stderr}"
    );
    assert!(
        failed_stderr.contains("archon-mutation-moved-12345678901234567890123456789012: 1"),
        "{failed_stderr}"
    );
    assert_eq!(
        std::fs::read_to_string(failed_site_path.join("d/f")).unwrap(),
        "original"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn a_changed_input_parent_remains_unsafe_after_restore() {
    let temp = tempfile::tempdir().unwrap();
    let site_path = temp.path().join("site");
    std::fs::create_dir_all(site_path.join("d")).unwrap();
    std::fs::write(site_path.join("d/f"), "original").unwrap();
    let root = super::super::super::mutation_fd::SiteRoot::capture(&site_path).unwrap();
    let nonce = "fedcba9876543210fedcba9876543210";
    let plan = MutationPlan {
        nonce: nonce.into(),
        inputs: vec![MutationInputPlan {
            named_path: "d/f".into(),
            site_copy_alias: false,
            move_path: PathBuf::from("d/f"),
            aside_path: PathBuf::from("d/f.archon-mutated-fedcba987654"),
        }],
    };
    let mut site = site(&site_path, false, 20);
    site.mutation_root = Some(root);
    site.mutation_plan = Some(&plan);
    let result = run(&site, "mkdir d/extra; exit 1").await;
    let stdout = String::from_utf8_lossy(&result.stdout);
    assert!(stdout.contains(": 0:original-recreated"), "{stdout}");
    assert!(
        !stdout.contains(&format!("archon-mutation-restored-{nonce}")),
        "{stdout}"
    );
    assert_eq!(
        std::fs::read_to_string(site_path.join("d/f")).unwrap(),
        "original"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn a_missing_parent_snapshot_without_a_moved_ancestor_stays_unproven() {
    let temp = tempfile::tempdir().unwrap();
    let site_path = temp.path().join("site");
    std::fs::create_dir_all(site_path.join("d")).unwrap();
    std::fs::write(site_path.join("d/f"), "original").unwrap();
    let root = super::super::super::mutation_fd::SiteRoot::capture(&site_path).unwrap();
    let nonce = "abcdef0123456789abcdef0123456789";
    let plan = MutationPlan {
        nonce: nonce.into(),
        inputs: vec![MutationInputPlan {
            named_path: "d/f".into(),
            site_copy_alias: false,
            move_path: PathBuf::from("d/f"),
            aside_path: PathBuf::from("d/f.archon-mutated-abcdef012345"),
        }],
    };
    let moved = vec![super::super::super::mutation_fd::move_aside(
        &root,
        &plan.inputs[0].move_path,
        &plan.inputs[0].aside_path,
    )];
    let mut result = CheckResult {
        acceptance_id: "AC-X-001".into(),
        exit_code: Some(1),
        quota_walk_count: 0,
        stdout: Vec::new(),
        stderr: Vec::new(),
        environment_note: None,
        operational_error: None,
        fault: None,
        classification: None,
    };
    super::super::mutation::restore_plan(
        &mut result,
        &plan,
        &moved,
        &[Some(Err("too many open files".into()))],
        &root,
    );
    let stdout = String::from_utf8_lossy(&result.stdout);
    assert!(
        stdout.contains(": 0:unsafe-to-restore: parent snapshot failed: too many open files"),
        "{stdout}"
    );
    assert!(
        !stdout.contains(&format!("archon-mutation-restored-{nonce}")),
        "{stdout}"
    );
}

#[cfg(unix)]
#[test]
fn a_snapshot_error_with_a_recreated_input_records_unsafe_reason_and_recreation_markers() {
    let temp = tempfile::tempdir().unwrap();
    let site_path = temp.path().join("site");
    std::fs::create_dir_all(site_path.join("d")).unwrap();
    std::fs::write(site_path.join("d/f"), "original").unwrap();
    let root = super::super::super::mutation_fd::SiteRoot::capture(&site_path).unwrap();
    let nonce = "0123456789abcdef0123456789abcdef";
    let plan = MutationPlan {
        nonce: nonce.into(),
        inputs: vec![MutationInputPlan {
            named_path: "d/f".into(),
            site_copy_alias: false,
            move_path: PathBuf::from("d/f"),
            aside_path: PathBuf::from("d/f.archon-mutated-0123456789ab"),
        }],
    };
    let moved = vec![super::super::super::mutation_fd::move_aside(
        &root,
        &plan.inputs[0].move_path,
        &plan.inputs[0].aside_path,
    )];
    std::fs::write(site_path.join("d/f"), "recreated").unwrap();
    let mut result = CheckResult {
        acceptance_id: "AC-X-001".into(),
        exit_code: Some(0),
        quota_walk_count: 0,
        stdout: Vec::new(),
        stderr: Vec::new(),
        environment_note: None,
        operational_error: None,
        fault: None,
        classification: None,
    };

    super::super::mutation::restore_plan(
        &mut result,
        &plan,
        &moved,
        &[Some(Err("permission denied".into()))],
        &root,
    );

    let stdout = String::from_utf8_lossy(&result.stdout);
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(
        stdout.contains(&format!(
            "archon-mutation-state-{nonce}: 0:unsafe-to-restore: parent snapshot failed: permission denied"
        )),
        "{stdout}"
    );
    assert!(
        stderr.contains(&format!("archon-mutation-generated-{nonce}: 0")),
        "{stderr}"
    );
    assert!(
        stderr.contains(&format!("archon-mutation-recreated-{nonce}: 0")),
        "{stderr}"
    );
    assert_eq!(
        std::fs::read_to_string(site_path.join("d/f")).unwrap(),
        "recreated"
    );
}
