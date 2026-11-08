use super::*;
use crate::write_boundary::landlock::LandlockSandbox;

#[test]
fn syscall_failures_report_the_exact_step_errno_and_path() {
    let mounts = MountSeals {
        seals: vec![Seal {
            path: CString::new("/protected/config").unwrap(),
            display: "/protected/config".into(),
            readonly: true,
            directory: true,
            submounts: false,
        }],
        uid: b"1000 1000 1\n".to_vec(),
        gid: b"1000 1000 1\n".to_vec(),
    };
    for step in STEPS {
        let mut visited = Vec::new();
        let failure = mounts
            .apply_with(true, |name, _syscall| {
                visited.push(name);
                if name == *step {
                    // SAFETY: thread-local errno, just like a failed syscall.
                    unsafe {
                        *libc::__errno_location() = libc::EPERM;
                    }
                    -1
                } else {
                    0
                }
            })
            .unwrap_err();
        if matches!(*step, "inspect working directory" | "unshare") {
            assert_eq!(visited.last(), Some(step));
        } else {
            assert_eq!(visited.last(), Some(&"re-enter working directory"));
            for cleanup in &STEPS[10..] {
                assert!(visited.contains(cleanup), "{step}: {visited:?}");
            }
        }
        assert_eq!(failure.step, *step);
        let message = failure.message(&mounts);
        let expected = if namespace_step(step) {
            format!("private mount namespace unavailable ({step}, errno 1)")
        } else if diagnostics::process_step(step) {
            format!("private seal operation {step} failed (errno 1)")
        } else {
            format!(
                "private seal operation {step} failed (errno 1); cannot protect /protected/config"
            )
        };
        assert_eq!(message, expected);
    }
}

#[test]
fn remount_preserves_source_flags_including_relatime() {
    assert_eq!(
        remount_flags(libc::ST_SYNCHRONOUS | libc::ST_MANDLOCK),
        libc::MS_STRICTATIME
    );
    assert_eq!(
        remount_flags(libc::ST_NOSUID | libc::ST_NODEV | libc::ST_NOEXEC | libc::ST_RELATIME),
        libc::MS_NOSUID | libc::MS_NODEV | libc::MS_NOEXEC | libc::MS_RELATIME
    );
    assert_eq!(
        remount_flags(libc::ST_NOATIME | libc::ST_NODIRATIME),
        libc::MS_NOATIME | libc::MS_NODIRATIME
    );
}

#[test]
fn missing_seal_inside_a_grant_refuses_but_outside_needs_no_mount() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let missing = root.join("missing-config");
    assert_eq!(
        MountSeals::build(std::slice::from_ref(&missing), std::slice::from_ref(&root)).unwrap_err(),
        format!(
            "Linux cannot write-seal {} inside a writable folder: the protected path does not exist",
            missing.display()
        )
    );
    let mounts = MountSeals::build(&[missing], &[root.join("writable")]).unwrap();
    assert!(mounts.seals.is_empty());
}

#[test]
fn private_mounts_seal_files_and_leave_ancestor_and_temp_creation_working() {
    const CHILD: &str = "ARCHON_MOUNT_SEAL_CHILD";
    if let Some(root) = std::env::var_os(CHILD) {
        let root = PathBuf::from(root);
        for path in [
            ".git/config",
            "../startup.rc",
            "../autostart/new",
            "/proc/self/cwd/.git/config",
        ] {
            assert_eq!(
                std::fs::write(path, "forged").unwrap_err().raw_os_error(),
                Some(libc::EROFS),
                "{path}"
            );
        }
        for name in [
            "repo/.git/config",
            "linked/.git",
            "operator.conf",
            "included.conf",
            "startup.rc",
        ] {
            let path = root.join(name);
            assert_eq!(std::fs::read(&path).unwrap(), b"original");
            assert_eq!(
                std::fs::write(&path, "forged").unwrap_err().raw_os_error(),
                Some(libc::EROFS)
            );
            assert!(matches!(
                std::fs::remove_file(&path).unwrap_err().raw_os_error(),
                Some(libc::EBUSY | libc::EROFS)
            ));
            let replacement = path.with_extension("replacement");
            std::fs::write(&replacement, "forged").unwrap();
            assert!(matches!(
                std::fs::rename(&replacement, &path)
                    .unwrap_err()
                    .raw_os_error(),
                Some(libc::EBUSY | libc::EROFS)
            ));
            std::fs::remove_file(replacement).unwrap();
        }
        assert_eq!(
            std::fs::rename(root.join("repo/.git"), root.join("repo/replaced"))
                .unwrap_err()
                .raw_os_error(),
            Some(libc::EBUSY)
        );
        assert_eq!(
            std::fs::write(root.join("autostart/new"), "forged")
                .unwrap_err()
                .raw_os_error(),
            Some(libc::EROFS)
        );
        for name in [
            "repo/new",
            "repo/.git/index.lock",
            "repo/target/output",
            "temp/new",
            "path-bin/new",
            "ancestor-new",
        ] {
            let file = root.join(name);
            std::fs::write(&file, "created").unwrap();
            std::fs::rename(&file, file.with_extension("moved")).unwrap();
            std::fs::create_dir(file.with_extension("dir")).unwrap();
        }
        let mount = CString::new(root.join("repo/.git/config").as_os_str().as_bytes()).unwrap();
        assert_eq!(
            unsafe { libc::umount2(mount.as_ptr(), libc::MNT_DETACH) },
            -1
        );
        assert_eq!(io::Error::last_os_error().raw_os_error(), Some(libc::EPERM));
        // Exercise the real system temp root too, including /tmp on Linux.
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("created"), "ok").unwrap();
        let direct = tempfile::Builder::new()
            .prefix("mount-seal-")
            .tempdir_in("/tmp")
            .unwrap();
        std::fs::write(direct.path().join("created"), "ok").unwrap();
        std::fs::write(root.join("child-ran"), "yes").unwrap();
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    for name in [
        "repo/.git",
        "repo/target",
        "linked",
        "temp",
        "path-bin",
        "autostart",
    ] {
        std::fs::create_dir_all(root.join(name)).unwrap();
    }
    let mut protected: Vec<_> = [
        "repo/.git/config",
        "linked/.git",
        "operator.conf",
        "included.conf",
        "startup.rc",
    ]
    .map(|name| {
        let path = root.join(name);
        std::fs::write(&path, "original").unwrap();
        path
    })
    .into();
    protected.push(root.join("autostart"));
    // Root grants model an ordinary command's writable filesystem. Ancestors
    // of the sealed files must not acquire no-create restrictions.
    let sandbox = match LandlockSandbox::build_sealed(&protected) {
        Ok(sandbox) => sandbox,
        Err(error) => {
            assert!(
                std::env::var_os("CI").is_none(),
                "CI requires mount seals: {error}"
            );
            eprintln!("kernel test unavailable: {error}");
            return;
        }
    };
    let missing = root.join("missing-config");
    assert_eq!(
        LandlockSandbox::build_sealed(std::slice::from_ref(&missing)).unwrap_err(),
        format!(
            "Linux cannot write-seal {} inside a writable folder: the protected path does not exist",
            missing.display()
        )
    );
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command.args(["--exact", "write_boundary::landlock::mounts::tests::private_mounts_seal_files_and_leave_ancestor_and_temp_creation_working", "--nocapture"])
        .env(CHILD, &root).current_dir(root.join("repo"));
    sandbox.install_std(&mut command);
    let output = command.output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(root.join("child-ran").exists());
    for path in protected.into_iter().filter(|path| path.is_file()) {
        assert_eq!(std::fs::read(path).unwrap(), b"original");
    }
    // Namespace changes must never leak back into the host.
    std::fs::write(root.join("repo/.git/config"), "host still writable").unwrap();
}

#[test]
fn symbolic_link_seals_refuse_before_a_replaceable_entry_can_run() {
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("target");
    let link = temp.path().join("link");
    std::fs::write(&target, "original").unwrap();
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let error = MountSeals::build(&[link], &[temp.path().to_path_buf()]).unwrap_err();
    assert!(
        error.contains("directory entry could be replaced independently of its target"),
        "{error}"
    );
}

#[test]
fn additional_layers_seal_file_roots_with_complement_rules() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("protected");
    std::fs::write(&file, "original").unwrap();
    if crate::write_boundary::landlock::probe().is_err() {
        assert!(std::env::var_os("CI").is_none(), "CI requires Landlock");
        return;
    }
    let boundary =
        LandlockSandbox::build_layered(&[file.clone()], &[temp.path().to_path_buf()], &[]).unwrap();
    let mut command = std::process::Command::new("/bin/sh");
    boundary.install_std(&mut command);
    let output = command
        .args(["-c", "printf forged > protected"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(std::fs::read_to_string(file).unwrap(), "original");
}

#[test]
fn mount_seals_do_not_require_a_ruleset() {
    let boundary = LandlockSandbox::build_sealed(&[]).unwrap();
    assert!(boundary.ruleset.is_none());
    assert_eq!(boundary.grants(), 0);
}

#[test]
fn nonreplaceable_system_link_is_not_classified_as_replaceable() {
    if unsafe { libc::geteuid() } == 0 {
        return; // The privileged identity can replace root-owned directory entries.
    }
    assert!(std::path::Path::new("/proc/self").is_symlink());
    assert!(!replaceable_link(std::path::Path::new("/proc/self")));
}

#[test]
fn ancestor_failure_labels_the_protected_target_as_a_rename_guard() {
    let mounts = MountSeals {
        seals: vec![Seal {
            path: CString::new("/ancestor").unwrap(),
            display: "/ancestor/protected".into(),
            readonly: false,
            directory: true,
            submounts: false,
        }],
        uid: Vec::new(),
        gid: Vec::new(),
    };
    let failure = Failure {
        step: "bind",
        errno: libc::EINVAL,
        seal: 0,
    };
    assert_eq!(
        failure.message(&mounts),
        "private seal operation bind failed (errno 22); rename guard for /ancestor/protected"
    );
}

#[test]
fn unavailable_recursive_primitive_requires_no_submounts() {
    for submounts in [false, true] {
        let mounts = MountSeals {
            seals: vec![Seal {
                path: c"/protected".into(),
                display: "/protected".into(),
                readonly: true,
                directory: true,
                submounts,
            }],
            uid: Vec::new(),
            gid: Vec::new(),
        };
        let result = mounts.apply_with(false, |step, _| {
            if step == "recursive-readonly" {
                unsafe {
                    *libc::__errno_location() = libc::ENOSYS;
                }
                -1
            } else {
                0
            }
        });
        if submounts {
            assert_eq!(
                result.unwrap_err().message(&mounts),
                "Linux <5.12 lacks mount_setattr and /protected contains mounts that cannot be made read-only"
            );
        } else {
            assert!(result.is_ok());
        }
    }
}

#[test]
fn preflight_skips_working_directory_checks() {
    let mounts = MountSeals {
        seals: vec![Seal {
            path: c"/protected".into(),
            display: "/protected".into(),
            readonly: true,
            directory: true,
            submounts: false,
        }],
        uid: Vec::new(),
        gid: Vec::new(),
    };
    mounts
        .apply_with(false, |step, _| {
            assert!(!step.contains("working directory"));
            0
        })
        .unwrap();
}

#[test]
fn cwd_reentry_refuses_a_different_directory_identity() {
    use std::os::fd::AsRawFd;
    let directory = tempfile::tempdir().unwrap();
    let file = std::fs::File::open(directory.path()).unwrap();
    let mut before: libc::stat = unsafe { std::mem::zeroed() };
    assert_eq!(unsafe { libc::fstat(file.as_raw_fd(), &mut before) }, 0);
    assert_eq!(unsafe { topology::reenter(&before) }, -1);
    assert_eq!(
        io::Error::last_os_error().raw_os_error(),
        Some(libc::ESTALE)
    );
    let mounts = MountSeals {
        seals: Vec::new(),
        uid: Vec::new(),
        gid: Vec::new(),
    };
    assert_eq!(
        Failure {
            step: "re-enter working directory",
            errno: libc::ESTALE,
            seal: 0
        }
        .message(&mounts),
        "re-enter working directory: directory changed"
    );
}

#[test]
fn observed_outcome_lists_completed_and_partial_seals() {
    let mounts = MountSeals {
        seals: ["/first", "/second"]
            .into_iter()
            .map(|path| Seal {
                path: CString::new(path).unwrap(),
                display: path.into(),
                readonly: true,
                directory: true,
                submounts: path == "/second",
            })
            .collect(),
        uid: Vec::new(),
        gid: Vec::new(),
    };
    let report = mounts.outcome_record(StatusRecord(
        [step_index("bind"), libc::EACCES, 1, 1],
        [2, 0, 0, 0],
    ));
    assert!(
        report.contains("applied seals: [read-only /first]"),
        "{report}"
    );
    assert!(report.contains("failed step bind (errno 13)"), "{report}");
    assert!(!report.contains("read-only /second"), "{report}");
    let report = mounts.outcome_record(StatusRecord(
        [step_index("recursive-readonly"), libc::ENOSYS, 1, 1],
        [2, 0, 0, 0],
    ));
    assert!(
        report.contains("read-only /first; read-only /second; submounts writable"),
        "{report}"
    );
    assert!(
        report.contains("failed step recursive-readonly (errno 38)"),
        "{report}"
    );
    let report = mounts.outcome_record(StatusRecord(
        [step_index("clear process capabilities"), libc::EPERM, 0, 1],
        [0; 4],
    ));
    assert!(
        report.contains("applied seals: [read-only /first; read-only /second]"),
        "{report}"
    );
    assert!(
        mounts
            .outcome_record(StatusRecord([0, 0, 0, 1], [0; 4]))
            .contains("all seal steps completed")
    );
    assert!(
        mounts
            .outcome_record(StatusRecord([i32::MAX, 0, 0, 0], [0; 4]))
            .contains("unknown")
    );
    assert!(
        mounts
            .outcome_record(StatusRecord([step_index("bind"), 1, 2, 0], [0; 4]))
            .contains("unknown")
    );
}

#[test]
fn child_continues_without_recursive_support_only_without_submounts() {
    for submounts in [false, true] {
        let mounts = MountSeals {
            seals: ["/protected", "/later"]
                .map(|name| Seal {
                    path: CString::new(name).unwrap(),
                    display: name.into(),
                    readonly: true,
                    directory: true,
                    submounts,
                })
                .into(),
            uid: Vec::new(),
            gid: Vec::new(),
        };
        let mut visited = Vec::new();
        let mut cwd_reentered = false;
        let result = mounts.apply_with_status(true, &mut cwd_reentered, |step, _| {
            visited.push(step);
            if step == "recursive-readonly" {
                unsafe {
                    *libc::__errno_location() = libc::ENOSYS;
                }
                -1
            } else {
                0
            }
        });
        assert_eq!(visited.last(), Some(&"re-enter working directory"));
        assert!(cwd_reentered);
        assert_eq!(visited.iter().filter(|step| **step == "bind").count(), 2);
        if submounts {
            assert_eq!(result.unwrap_err().errno, libc::ENOSYS);
        } else {
            assert!(result.is_ok());
        }
    }
}
