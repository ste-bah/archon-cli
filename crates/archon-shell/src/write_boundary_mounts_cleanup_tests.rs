use super::*;

fn mounts() -> MountSeals {
    MountSeals {
        seals: ["/first", "/second"]
            .map(|name| Seal {
                path: CString::new(name).unwrap(),
                display: name.into(),
                readonly: true,
                directory: false,
                submounts: false,
            })
            .into(),
        uid: Vec::new(),
        gid: Vec::new(),
    }
}

#[test]
fn bind_failure_continues_sealing_and_attempts_all_cleanup_steps() {
    let mounts = mounts();
    let mut visited = Vec::new();
    let failure = mounts
        .apply_with(true, |step, _| {
            visited.push(step);
            if step == "bind" {
                unsafe {
                    *libc::__errno_location() = libc::EACCES;
                }
                -1
            } else {
                0
            }
        })
        .unwrap_err();
    assert_eq!(failure.step, "bind");
    assert_eq!(failure.errno, libc::EACCES);
    assert_eq!(visited.iter().filter(|step| **step == "bind").count(), 2);
    assert_eq!(&visited[visited.len() - 4..], &STEPS[10..]);
    assert_eq!(visited.last(), Some(&"re-enter working directory"));
}

#[test]
fn failed_reentry_preserves_first_failure_and_reports_inherited_cwd() {
    let mounts = mounts();
    let mut cwd_reentered = false;
    let mut binds = 0;
    let failure = mounts
        .apply_with_status(true, &mut cwd_reentered, |step, _| {
            if step == "bind" {
                binds += 1;
            }
            if (step == "bind" && binds == 2) || step == "re-enter working directory" {
                unsafe {
                    *libc::__errno_location() = libc::EACCES;
                }
                -1
            } else {
                0
            }
        })
        .unwrap_err();
    assert_eq!(failure.step, "bind");
    assert_eq!(failure.seal, 1);
    assert!(!cwd_reentered);
    let report = mounts.outcome_record(StatusRecord(
        [
            step_index(failure.step),
            failure.errno,
            failure.seal as i32,
            0,
        ],
        [2, 0, 0, 0],
    ));
    assert!(
        report.contains("applied seals: [read-only /first]"),
        "{report}"
    );
    assert!(
        report.contains("applied seals bypassable through the inherited working directory"),
        "{report}"
    );
    assert!(
        !mounts
            .outcome_record(StatusRecord(
                [step_index("bind"), libc::EACCES, 1, 1],
                [2, 0, 0, 0]
            ))
            .contains("inherited working directory")
    );
}

#[test]
fn empty_or_unstarted_seals_never_claim_inherited_cwd_bypass() {
    let mut empty = mounts();
    empty.seals.clear();
    for report in [
        empty.outcome_record(StatusRecord([0, 0, 0, 0], [0; 4])),
        mounts().outcome_record(StatusRecord([step_index("unshare"), 1, 0, 0], [3, 0, 0, 0])),
    ] {
        assert!(report.contains("applied seals: []"), "{report}");
        assert!(!report.contains("inherited working directory"), "{report}");
    }
}

#[test]
fn successful_real_outcome_omits_observation_note() {
    let outcome = mounts().outcome_record(StatusRecord([0, 0, 0, 1], [0; 4]));
    assert!(outcome.contains("applied seals:"));
    assert!(
        crate::read_boundary::observation_note(format!(
            "Filesystem read boundary: named-paths-denied; writes: {outcome}"
        ))
        .is_none()
    );
}

#[test]
fn rejected_path_preserves_valid_seals() {
    if let Err(reason) = MountSeals::probe() {
        assert!(
            std::env::var_os("CI").is_none(),
            "CI requires mount seals: {reason}"
        );
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let valid = root.path().join("valid");
    let link = root.path().join("link");
    std::fs::write(&valid, "protected").unwrap();
    std::os::unix::fs::symlink(&valid, &link).unwrap();
    for paths in [
        vec![link.clone(), valid.clone()],
        vec![valid.clone(), link.clone()],
    ] {
        let (prepared, notes) = MountSeals::build_available(&paths).unwrap();
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(notes[0].contains("link"), "{notes:?}");
        let real = valid.canonicalize().unwrap();
        assert!(
            prepared
                .seals
                .iter()
                .any(|seal| seal.readonly && seal.path.as_bytes() == real.as_os_str().as_bytes())
        );
    }
}

#[test]
fn failed_path_does_not_drop_later_seal_and_bitmap_names_every_failure() {
    for failing in [vec![0], vec![0, 1]] {
        let mounts = mounts();
        let mut failed = [0; 4];
        let mut binds = 0;
        let mut cwd = false;
        let failure = mounts
            .apply_record(true, &mut cwd, &mut failed, |step, _| {
                if step == "bind" {
                    let index = binds;
                    binds += 1;
                    if failing.contains(&index) {
                        unsafe {
                            *libc::__errno_location() = libc::EACCES;
                        }
                        return -1;
                    }
                }
                0
            })
            .unwrap_err();
        assert_eq!(binds, 2);
        assert!(cwd);
        assert_eq!(failed[0], if failing.len() == 1 { 1 } else { 3 });
        let report = mounts.outcome_record(StatusRecord(
            [
                step_index(failure.step),
                failure.errno,
                failure.seal as i32,
                i32::from(cwd),
            ],
            failed,
        ));
        assert!(report.contains("/first not sealed"), "{report}");
        if failing.len() == 1 {
            assert!(
                report.contains("applied seals: [read-only /second]"),
                "{report}"
            );
            assert!(!report.contains("/second not sealed"), "{report}");
        } else {
            assert!(report.contains("/second not sealed"), "{report}");
        }
    }
}

#[test]
fn available_paths_read_topology_once_and_probe_the_combined_set_once() {
    let root = tempfile::tempdir().unwrap();
    let paths = [root.path().join("first"), root.path().join("second")];
    for path in &paths {
        std::fs::write(path, "protected").unwrap();
    }
    let mut probes = 0;
    let (prepared, notes) = MountSeals::test_available(
        &paths,
        || Ok(Vec::new()),
        |mounts| {
            probes += 1;
            for path in &paths {
                assert!(
                    mounts
                        .seals
                        .iter()
                        .any(|seal| seal.readonly && seal.display == path.canonicalize().unwrap())
                );
            }
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(probes, 1);
    assert!(notes.is_empty());
    assert!(prepared.seals.iter().filter(|seal| seal.readonly).count() >= 2);
    let error = MountSeals::test_available(
        &paths,
        || Err("unavailable".into()),
        |_| panic!("no topology"),
    )
    .unwrap_err();
    for path in &paths {
        assert!(error.contains(&path.display().to_string()), "{error}");
    }
}

#[test]
fn failed_combined_preflight_diagnoses_paths_then_checks_retained_set() {
    let root = tempfile::tempdir().unwrap();
    let paths = [root.path().join("first"), root.path().join("second")];
    for path in &paths {
        std::fs::write(path, "protected").unwrap();
    }
    let mut probes = 0;
    let (prepared, notes) = MountSeals::test_available(
        &paths,
        || Ok(Vec::new()),
        |_| {
            probes += 1;
            if probes <= 2 {
                Err("injected".into())
            } else {
                Ok(())
            }
        },
    )
    .unwrap();
    assert_eq!(probes, 4);
    assert_eq!(notes.len(), 1);
    assert!(notes[0].contains("first"));
    assert!(
        prepared
            .seals
            .iter()
            .any(|seal| seal.readonly && seal.display == paths[1].canonicalize().unwrap())
    );
    assert!(
        !prepared
            .seals
            .iter()
            .any(|seal| seal.readonly && seal.display == paths[0].canonicalize().unwrap())
    );
}

#[test]
fn status_capacity_rejection_retains_valid_paths_and_names_rejected_paths() {
    let root = tempfile::tempdir().unwrap();
    let paths: Vec<_> = (0..300)
        .map(|i| root.path().join(format!("protected-{i}")))
        .collect();
    for path in &paths {
        std::fs::write(path, "protected").unwrap();
    }
    let (prepared, notes) =
        MountSeals::test_available(&paths, || Ok(Vec::new()), |_| Ok(())).unwrap();
    assert_eq!(prepared.seals.len(), 256);
    assert!(!notes.is_empty());
    for path in paths {
        let real = path.canonicalize().unwrap();
        let retained = prepared
            .seals
            .iter()
            .any(|seal| seal.readonly && seal.display == real);
        let named = notes
            .iter()
            .any(|note| note.starts_with(&format!("{} not sealed:", path.display())));
        assert_ne!(retained, named, "{}", path.display());
    }
}
