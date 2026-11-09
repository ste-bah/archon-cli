use super::*;

#[test]
fn configured_tokens_match_only_exact_words_or_option_assignments() {
    let limits = VerifyLimits {
        refuse_tokens: vec!["--wide".into()],
        ..Default::default()
    };
    for command in ["check --wide", "check --wide=yes", "other --wide="] {
        assert!(
            matches!(vet(command, &limits), Err(RefusedToRun::WorkspaceWideCommand { token, .. }) if token == "--wide")
        );
    }
    for command in ["check --wider", "check prefix--wide", "check --wide-extra"] {
        assert!(vet(command, &limits).is_ok(), "{command}");
    }
    let limits = VerifyLimits {
        refuse_tokens: vec![],
        ..limits
    };
    for command in ["go test ./...", "cargo test", "check --wide=yes"] {
        assert!(vet(command, &limits).is_ok(), "{command}");
    }
}

#[test]
fn shell_syntax_is_refused_even_without_tokens() {
    let limits = VerifyLimits {
        refuse_tokens: vec![],
        ..Default::default()
    };
    for command in [
        "check | output",
        "check && next",
        "check > output",
        "check\nnext",
        "",
    ] {
        assert!(matches!(
            vet(command, &limits),
            Err(RefusedToRun::NotDirectlyExecutable { .. })
        ));
    }
}

#[test]
fn old_config_shapes_retain_shipped_refusal_tokens() {
    for text in [
        "",
        "[tools.cargo]\nbuild_jobs = 2",
        "[workflow]\nwrite_confinement = true",
    ] {
        let config: archon_core::config::ArchonConfig = toml::from_str(text).unwrap();
        let limits = VerifyLimits::from(&config.workflow.verify);
        assert!(vet("check --workspace", &limits).is_err());
        assert!(vet("check --all=true", &limits).is_err());
        assert!(vet("cargo test", &limits).is_ok());
    }
}

#[cfg(unix)]
mod processes {
    use super::*;
    use archon_workflow::acceptance_check_environment::{CheckPolicy, CommandEnvironment};
    use std::collections::BTreeMap;

    fn environment(root: &Path, extra: Option<&Path>) -> CommandEnvironment {
        let host = BTreeMap::from([
            ("PATH".into(), "/usr/bin:/bin".into()),
            ("HOME".into(), root.display().to_string()),
        ]);
        let mut policy = CheckPolicy::default_for(&host);
        if let Some(extra) = extra {
            policy
                .bound
                .insert("BUILD_LOCATION".into(), extra.display().to_string());
        }
        CommandEnvironment::from_host(&host, Some(&policy)).unwrap()
    }

    fn space(path: &Path) -> std::io::Result<u64> {
        if path.join("low").exists() {
            Ok(1024 * 1024)
        } else {
            Ok(u64::MAX)
        }
    }
    fn space_error(_: &Path) -> std::io::Result<u64> {
        Err(std::io::Error::other("fixture disk unavailable"))
    }
    fn limits() -> VerifyLimits {
        VerifyLimits {
            free_space: space,
            min_free_disk_mib: 2,
            timeout: Duration::from_secs(15),
            ..Default::default()
        }
    }
    fn run_script(root: &Path, text: &str, limits: &VerifyLimits, extra: Option<&Path>) -> Ran {
        run_in_environment(
            root,
            &["/bin/sh".into(), "-c".into(), text.into()],
            limits,
            &environment(root, extra),
        )
    }
    fn repository() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        let result = std::process::Command::new("git")
            .args(["init", "-q"])
            .arg(root.path())
            .output()
            .unwrap();
        assert!(result.status.success());
        root
    }
    fn assert_gone(root: &Path) {
        let pids = std::fs::read_to_string(root.join("pids")).unwrap();
        for pid in pids
            .split_whitespace()
            .map(|pid| pid.parse::<u32>().unwrap())
        {
            let deadline = Instant::now() + Duration::from_secs(2);
            while archon_shell::process_liveness::process_alive(pid) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(20));
            }
            assert!(
                !archon_shell::process_liveness::process_alive(pid),
                "survived: {pid}"
            );
        }
    }

    #[test]
    fn disk_floor_refuses_before_launch_in_each_watched_directory() {
        fn low_temporary(path: &Path) -> std::io::Result<u64> {
            if path == std::env::temp_dir() {
                Ok(1024 * 1024)
            } else {
                Ok(u64::MAX)
            }
        }
        for temporary in [false, true] {
            let root = repository();
            let expected = if temporary {
                std::env::temp_dir()
            } else {
                root.path().to_path_buf()
            };
            if !temporary {
                std::fs::write(root.path().join("low"), "").unwrap();
            }
            let limits = VerifyLimits {
                free_space: if temporary { low_temporary } else { space },
                ..limits()
            };
            let ran = run_script(root.path(), "touch launched", &limits, None);
            assert!(
                matches!(ran, Ran::DiskFloorBeforeLaunch(DiskFloor { path, free_mib: 1, floor_mib: 2 }) if path == expected.display().to_string())
            );
            assert!(!root.path().join("launched").exists());
        }
    }

    #[test]
    fn low_search_list_components_do_not_refuse_a_real_process() {
        for order in [0, 1, 2] {
            let root = repository();
            let lookup = tempfile::tempdir().unwrap();
            std::fs::write(lookup.path().join("low"), "").unwrap();
            let entries = match order {
                0 => [lookup.path(), root.path()],
                1 => [root.path(), lookup.path()],
                _ => [lookup.path(), lookup.path()],
            };
            let value = std::path::PathBuf::from(std::env::join_paths(entries).unwrap());
            let ran = run_script(root.path(), "touch launched", &limits(), Some(&value));
            assert!(
                matches!(ran, Ran::Finished { success: true, .. }),
                "{ran:?}"
            );
            assert!(root.path().join("launched").exists());
        }
    }

    #[test]
    fn disk_probe_errors_are_recorded_before_launch() {
        let root = repository();
        let ran = run_script(
            root.path(),
            "touch launched",
            &VerifyLimits {
                free_space: space_error,
                ..limits()
            },
            None,
        );
        assert!(
            matches!(ran, Ran::NotLaunchable { reason } if reason.contains("fixture disk unavailable"))
        );
        assert!(!root.path().join("launched").exists());
    }

    #[test]
    fn disk_probe_error_during_execution_kills_the_tree() {
        fn probe(path: &Path) -> std::io::Result<u64> {
            if path.join("low").exists() {
                space_error(path)
            } else {
                Ok(u64::MAX)
            }
        }
        let root = repository();
        let ran = run_script(
            root.path(),
            "echo $$ > pids; sleep 60 & echo $! >> pids; touch low; wait",
            &VerifyLimits {
                free_space: probe,
                ..limits()
            },
            None,
        );
        assert!(
            matches!(ran, Ran::NotLaunchable { reason } if reason.contains("fixture disk unavailable"))
        );
        assert_gone(root.path());
    }

    #[test]
    fn mid_run_floor_kills_the_leader_and_its_grandchild() {
        thread_local! { static WRITTEN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
        fn temporary_after_write(path: &Path) -> std::io::Result<u64> {
            if path != std::env::temp_dir() {
                WRITTEN.set(path.join("low").exists());
            }
            if path == std::env::temp_dir() && WRITTEN.get() {
                Ok(1024 * 1024)
            } else {
                Ok(u64::MAX)
            }
        }
        for temporary in [false, true] {
            let root = repository();
            let started = Instant::now();
            let limits = VerifyLimits {
                free_space: if temporary {
                    temporary_after_write
                } else {
                    space
                },
                ..limits()
            };
            let ran = run_script(
                root.path(),
                "echo $$ > pids; sleep 60 & echo $! >> pids; touch low; wait",
                &limits,
                None,
            );
            assert!(
                matches!(ran, Ran::DiskFloor(DiskFloor { path, .. }) if path == if temporary { std::env::temp_dir() } else { root.path().to_path_buf() }.display().to_string())
            );
            assert_gone(root.path());
            assert!(started.elapsed() < Duration::from_secs(10));
        }
    }

    #[test]
    fn one_second_deadline_kills_the_leader_and_its_grandchild() {
        for text in [
            "echo $$ > pids; sleep 60 & echo $! >> pids; wait",
            "echo $$ > pids; sh -c 'echo $$ >> pids; sleep 60 & echo $! >> pids; wait' & wait",
            "trap '' TERM; echo $$ > pids; sleep 60 & echo $! >> pids; wait",
        ] {
            let root = repository();
            let started = Instant::now();
            let ran = run_script(
                root.path(),
                text,
                &VerifyLimits {
                    timeout: Duration::from_secs(1),
                    ..limits()
                },
                None,
            );
            assert!(matches!(ran, Ran::TimedOut { seconds: 1 }), "{ran:?}");
            assert_gone(root.path());
            assert!(started.elapsed() < Duration::from_secs(10));
        }
    }

    #[test]
    fn a_running_baseline_stopped_by_the_floor_is_inconclusive() {
        use super::super::super::tests::{plan_for, repo_with_committed_file};
        use archon_knowledge::traceability::{FalsificationOutcome, Inconclusive};
        let (root, path) = repo_with_committed_file("fn a() {}\nfn b() {}\n");
        let original = std::fs::read(&path).unwrap();
        std::fs::write(
            root.path().join("run"),
            "echo $$ > pids; sleep 60 & echo $! >> pids; touch low; wait",
        )
        .unwrap();
        let plan = plan_for(root.path(), "/bin/sh run");
        let host = BTreeMap::from([
            ("PATH".into(), "/usr/bin:/bin".into()),
            ("HOME".into(), root.path().display().to_string()),
        ]);
        let policy = CheckPolicy::default_for(&host);
        let outcome =
            super::super::super::attempt_with_policy(root.path(), &plan, Some(&policy), &limits())
                .unwrap();
        assert!(
            matches!(
                outcome,
                FalsificationOutcome::Inconclusive(Inconclusive::DiskFloor {
                    free_mib: 1,
                    floor_mib: 2,
                    ..
                })
            ),
            "{outcome:?}"
        );
        assert_eq!(std::fs::read(path).unwrap(), original);
        assert_gone(root.path());
    }

    #[test]
    fn successful_leader_still_reaps_background_descendants() {
        let root = repository();
        let ran = run_script(
            root.path(),
            "echo $$ > pids; sleep 60 & echo $! >> pids; exit 0",
            &limits(),
            None,
        );
        assert!(
            matches!(ran, Ran::Finished { success: true, .. }),
            "{ran:?}"
        );
        assert_gone(root.path());
    }
}

#[test]
fn disk_limits_before_mutation_refuse_and_during_mutation_never_prove_a_kill() {
    use archon_knowledge::traceability::{FalsificationOutcome, Inconclusive, ProofLevel};
    let floor = DiskFloor {
        path: "volume".into(),
        free_mib: 1,
        floor_mib: 2,
    };
    let refusal = floor.clone().refusal();
    assert!(matches!(
        refusal,
        RefusedToRun::DiskFloor {
            free_mib: 1,
            floor_mib: 2,
            ..
        }
    ));
    assert!(refusal.describe().contains("1 MiB"));
    let outcome = super::super::classify_result("unknown", Ran::DiskFloor(floor));
    assert_eq!(
        outcome,
        FalsificationOutcome::Inconclusive(Inconclusive::DiskFloor {
            path: "volume".into(),
            free_mib: 1,
            floor_mib: 2,
        })
    );
    assert_eq!(
        outcome.level_after(ProofLevel::Exercised),
        ProofLevel::Exercised
    );
    let encoded = serde_json::to_string(&outcome).unwrap();
    assert_eq!(
        serde_json::from_str::<FalsificationOutcome>(&encoded).unwrap(),
        outcome
    );
}

#[test]
fn only_working_root_and_process_temporary_directory_are_watched() {
    use super::super::limits::watched;
    let root = tempfile::tempdir().unwrap();
    let paths = watched(root.path());
    assert_eq!(paths, vec![root.path().to_path_buf(), std::env::temp_dir()]);
    assert_eq!(watched(&std::env::temp_dir()), vec![std::env::temp_dir()]);
}

#[test]
fn process_test_defaults_never_probe_the_host_disk() {
    let limits = VerifyLimits::for_test();
    let root = tempfile::tempdir().unwrap();
    for name in ["absent", "also-absent/nested", "missing-output"] {
        let path = root.path().join(name);
        assert!(!path.exists());
        assert_eq!((limits.free_space)(&path).unwrap(), u64::MAX);
    }
}

#[cfg(unix)]
#[test]
fn low_unrelated_environment_directories_do_not_refuse_launch() {
    use archon_workflow::acceptance_check_environment::CommandEnvironment;
    let root = tempfile::tempdir().unwrap();
    let readonly = tempfile::tempdir().unwrap();
    fn probe(path: &Path) -> std::io::Result<u64> {
        if path.join("readonly").exists() {
            Ok(0)
        } else {
            Ok(u64::MAX)
        }
    }
    std::fs::write(readonly.path().join("readonly"), "").unwrap();
    for key in [
        "HOME",
        "JAVA_HOME",
        "SSL_CERT_DIR",
        "GOROOT",
        "PATH",
        "BUILD_LOCATION",
    ] {
        let mut host = std::collections::BTreeMap::from([
            ("PATH".into(), "/usr/bin:/bin".into()),
            ("HOME".into(), root.path().display().to_string()),
        ]);
        host.insert(key.into(), readonly.path().display().to_string());
        let mut policy =
            archon_workflow::acceptance_check_environment::CheckPolicy::default_for(&host);
        policy
            .bound
            .insert(key.into(), readonly.path().display().to_string());
        let environment = CommandEnvironment::from_host(&host, Some(&policy)).unwrap();
        let ran = run_in_environment(
            root.path(),
            &["/bin/sh".into(), "-c".into(), "exit 0".into()],
            &VerifyLimits {
                free_space: probe,
                ..VerifyLimits::for_test()
            },
            &environment,
        );
        assert!(
            matches!(ran, Ran::Finished { success: true, .. }),
            "{key}: {ran:?}"
        );
    }
}

#[test]
fn launch_failure_names_the_declared_command() {
    let root = tempfile::tempdir().unwrap();
    let ran = run(
        root.path(),
        &["missing-verifier-executable".into(), "--selected".into()],
        Duration::from_secs(1),
    );
    assert!(
        matches!(ran, Ran::NotLaunchable { reason } if reason.contains("`missing-verifier-executable --selected`"))
    );
}
