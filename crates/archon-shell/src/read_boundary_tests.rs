use super::*;

#[test]
#[cfg(target_os = "linux")]
fn failed_rule_boundary_refuses_launch_only_when_landlock_is_active() {
    for (active, contained) in [
        (true, "named-paths-denied"),
        (false, "named-paths-denied"),
        (true, "denied"),
        (false, "denied"),
    ] {
        let mut command = crate::spawn::command("/bin/true");
        let reads = install_linux_boundary(
            &mut command,
            Err("injected read ruleset construction failure".into()),
            active,
            contained,
        );
        if active {
            assert!(reads.starts_with("launch-refused"));
            assert!(reads.contains("injected read ruleset construction failure"));
            let error = command
                .status()
                .expect_err("read failure must prevent exec");
            assert_eq!(error.raw_os_error(), Some(libc::EPERM));
        } else {
            assert_eq!(reads, "not-contained");
            assert!(command.status().unwrap().success());
        }
    }
}

#[test]
fn unavailable_boundary_does_not_claim_enforcement_for_a_program_name() {
    if crate::write_boundary::mechanism().is_ok() {
        return;
    }
    #[derive(Clone)]
    struct Capture(std::sync::Arc<std::sync::Mutex<bool>>);
    impl tracing::Subscriber for Capture {
        fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
            true
        }
        fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
            tracing::span::Id::from_u64(1)
        }
        fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
        fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
        fn event(&self, event: &tracing::Event<'_>) {
            struct Fields<'a>(&'a std::sync::Mutex<bool>);
            impl tracing::field::Visit for Fields<'_> {
                fn record_debug(
                    &mut self,
                    field: &tracing::field::Field,
                    value: &dyn std::fmt::Debug,
                ) {
                    if field.name() == "credential_reads"
                        && format!("{value:?}").contains("named-paths-denied")
                    {
                        *self.0.lock().unwrap() = true;
                    }
                }
            }
            event.record(&mut Fields(&self.0));
        }
        fn enter(&self, _: &tracing::span::Id) {}
        fn exit(&self, _: &tracing::span::Id) {}
    }
    let claimed = std::sync::Arc::new(std::sync::Mutex::new(false));
    tracing::subscriber::with_default(Capture(claimed.clone()), || {
        let _ = command(OsStr::new(crate::write_boundary::SANDBOX_EXEC));
    });
    assert!(
        !*claimed.lock().unwrap(),
        "host did not enforce any read policy"
    );
}

#[test]
fn relocated_host_credentials_and_atomic_siblings_are_protected() {
    const OVERRIDE: &str = "ARCHON_CREDENTIALS_FILE";
    if std::env::var_os("ARCHON_R2_PATH_PROBE").is_none() {
        let temp = tempfile::tempdir().unwrap();
        let status = crate::spawn::command(std::env::current_exe().unwrap())
            .args(["--exact", "read_boundary::tests::relocated_host_credentials_and_atomic_siblings_are_protected"])
            .env("ARCHON_R2_PATH_PROBE", "1")
            .env(OVERRIDE, temp.path().join("relocated.json"))
            .status().unwrap();
        assert!(
            status.success(),
            "relocated credential store must be denied"
        );
        return;
    }
    let relocated = PathBuf::from(std::env::var_os(OVERRIDE).unwrap());
    let paths = operator_paths();
    assert!(
        paths.contains(&relocated),
        "host credential override must be protected"
    );
    assert!(
        paths.contains(&relocated.with_extension("json.tmp")),
        "atomic sibling must be protected"
    );
}
#[test]
// Rule text uses `/` spellings; Windows joins with `\` and never builds these rules.
#[cfg(unix)]
fn credential_and_profile_paths_are_denied_by_absolute_path() {
    let home = Path::new("/synthetic-operator");
    let paths = denied_paths(home);
    for name in [
        ".archon/.credentials.json",
        ".aws",
        ".ssh",
        ".cargo/credentials.toml",
        ".codex/auth.json",
    ] {
        assert!(paths.contains(&home.join(name)));
    }
    let rules = macos_rules_for(&paths, &home.join(".archon/operator-env-grants"));
    assert!(rules.contains("(deny file-read* (subpath \"/synthetic-operator/.aws\"))"));
    assert!(rules.contains("(deny file-write* (subpath \"/synthetic-operator/.aws\"))"));
    for ancestor in ["/synthetic-operator/.config", "/synthetic-operator", "/"] {
        let rule = format!("(deny file-write* (literal \"{ancestor}\"))");
        assert_eq!(rules.matches(&rule).count(), 1, "{rule}");
    }
    assert!(!rules.contains("vnode-type"));
}

#[test]
#[cfg(target_os = "macos")]
fn credential_paths_cannot_be_moved_out_of_the_boundary() {
    if crate::write_boundary::mechanism().is_err() {
        eprintln!("kernel boundary unavailable; profile rules are tested separately");
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().canonicalize().unwrap();
    let parent = home.join("grandparent/parent");
    let protected = parent.join("protected");
    std::fs::create_dir_all(&protected).unwrap();
    std::fs::write(protected.join("key"), "synthetic-secret").unwrap();
    let missing = home.join("missing");
    let future_protected = missing.join("nested/protected");
    let paths = [protected.clone(), future_protected];
    let grant_root = home.join(".archon/operator-env-grants");
    prepare_macos_ancestors(&home, &paths, &grant_root);
    let rules = macos_rules_for(&paths, &grant_root);
    std::fs::create_dir(home.join("redirect")).unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let scratch_path = scratch.path().canonicalize().unwrap();
    let output = crate::spawn::command(crate::write_boundary::SANDBOX_EXEC)
        .args([
            "-p",
            &format!("(version 1)(allow default){rules}"),
            "/bin/sh",
            "-c",
            r#"set -eu
if mv "$1/protected" "$2/moved-protected"; then exit 11; fi
if mv "$1" "$2/moved-parent"; then exit 12; fi
if mv "$2/grandparent" "$2/moved-grandparent"; then exit 21; fi
if cat "$1/protected/key"; then exit 13; fi
if cp "$1/protected/key" "$2/copied-key"; then exit 14; fi
printf ordinary > "$1/new-file"
mkdir -p "$2/missing/nested"
if mv "$2/missing/nested" "$2/moved-nested"; then exit 15; fi
if mv "$2/missing" "$2/moved-missing"; then exit 16; fi
if rmdir "$2/missing/nested"; then exit 17; fi
if chmod 700 "$1"; then exit 18; fi
if ARCHON_SYMLINK_TARGET="$2/redirect" ARCHON_SYMLINK_PATH="$2/.archon" "$4" --exact read_boundary::tests::symlink_probe; then exit 19; else test "$?" -eq 1; fi
mkdir -p "$2/.archon" "$2/missing/nested"
if printf forged > "$2/.archon/operator-env-grants/grant"; then exit 20; fi
mkdir -p "$2/x/operator-env-grants"
printf forged > "$2/x/operator-env-grants/grant"
if ARCHON_RENAME_FROM="$2/x" ARCHON_RENAME_TO="$2/.archon" "$4" --exact read_boundary::tests::rename_probe; then exit 22; else test "$?" -eq 1; fi
mkdir -p "$2/y/fish"
printf forged > "$2/y/fish/config.fish"
if ARCHON_RENAME_FROM="$2/y" ARCHON_RENAME_TO="$2/missing/nested" "$4" --exact read_boundary::tests::rename_probe; then exit 23; else test "$?" -eq 1; fi
printf ordinary > "$2/missing/nested/new-file"
for base in "$2" "$3"; do
    printf ordinary > "$base/ordinary"
    mv "$base/ordinary" "$base/renamed"
    rm "$base/renamed"
done
"#,
            "probe",
        ])
        .arg(&parent)
        .arg(&home)
        .arg(&scratch_path)
        .arg(std::env::current_exe().unwrap())
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(!String::from_utf8_lossy(&output.stdout).contains("synthetic-secret"));
    assert!(!temp.path().join("redirect/operator-env-grants").exists());
    assert!(!grant_root.join("grant").exists());
    assert!(!home.join("missing/nested/fish/config.fish").exists());
    assert!(!temp.path().join(".archon").is_symlink());
    assert_eq!(
        std::fs::read_to_string(protected.join("key")).unwrap(),
        "synthetic-secret"
    );
    for name in [
        "moved-protected",
        "moved-parent",
        "moved-grandparent",
        "copied-key",
        "moved-nested",
        "moved-missing",
    ] {
        let path = temp.path().join(name);
        assert!(
            !path.exists(),
            "unexpected copy or moved secret at {path:?}"
        );
    }
    assert_eq!(
        std::fs::read_to_string(parent.join("new-file")).unwrap(),
        "ordinary"
    );
    assert_eq!(
        std::fs::read_to_string(missing.join("nested/new-file")).unwrap(),
        "ordinary"
    );
}

#[test]
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn credential_file_is_unreadable_when_kernel_boundary_is_available() {
    if crate::write_boundary::mechanism().is_err() {
        eprintln!(
            "kernel read probe unavailable in this host sandbox; profile/path policy is tested separately"
        );
        return;
    }
    let home = tempfile::tempdir().unwrap();
    let home_path = home.path().canonicalize().unwrap();
    std::fs::create_dir(home_path.join(".aws")).unwrap();
    let secret = home_path.join(".aws/credentials");
    std::fs::write(&secret, "sk-test-file-355").unwrap();
    #[cfg(target_os = "linux")]
    let mut command = crate::spawn::command("/bin/sh");
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = crate::spawn::command(crate::write_boundary::SANDBOX_EXEC);
        command.args([
            "-p",
            &format!("(version 1)(allow default){}", {
                let paths = denied_paths(&home_path);
                let grant = home_path.join(".archon/operator-env-grants");
                prepare_macos_ancestors(&home_path, &paths, &grant);
                macos_rules_for(&paths, &grant)
            }),
            "/bin/sh",
        ]);
        command
    };
    #[cfg(target_os = "linux")]
    crate::write_boundary::landlock::LandlockSandbox::build_read(&denied_paths(&home_path))
        .unwrap()
        .install_std(&mut command);
    let status = command
        .args(["-c", "test ! -r \"$1\" || ! cat \"$1\" >/dev/null", "probe"])
        .arg(&secret)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "credential read must be denied");
}

#[test]
#[cfg(target_os = "macos")]
fn rename_probe() {
    if let Some(from) = std::env::var_os("ARCHON_RENAME_FROM") {
        // Exit directly: the parent checks the actual rename syscall result.
        let code = match std::fs::rename(from, std::env::var_os("ARCHON_RENAME_TO").unwrap()) {
            Ok(()) => 0,
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => 1,
            Err(error) => panic!("rename must fail at the boundary: {error}"),
        };
        std::process::exit(code);
    }
}

#[test]
#[cfg(target_os = "macos")]
fn symlink_probe() {
    if let Some(target) = std::env::var_os("ARCHON_SYMLINK_TARGET") {
        // An existing directory must be tested as the exact destination, rather
        // than ln's convention of creating a link inside a destination directory.
        let code = match std::os::unix::fs::symlink(
            target,
            std::env::var_os("ARCHON_SYMLINK_PATH").unwrap(),
        ) {
            Ok(()) => 0,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::AlreadyExists
                ) =>
            {
                1
            }
            Err(error) => panic!("unexpected ancestor symlink error: {error}"),
        };
        std::process::exit(code);
    }
}

#[test]
#[cfg(unix)]
fn host_prepares_missing_ancestors_and_private_grant_root() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().canonicalize().unwrap();
    let grant = home.join(".archon/operator-env-grants");
    let paths = denied_paths(&home);
    prepare_macos_ancestors(&home, &paths, &grant);
    for path in paths.iter().chain([&grant]) {
        for ancestor in path.ancestors().skip(1).take_while(|p| *p != home) {
            assert!(ancestor.is_dir(), "{}", ancestor.display());
        }
    }
    for path in [home.join(".archon"), grant.clone()] {
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
    assert!(!home.join(".config/fish/config.fish").exists());
    #[cfg(not(windows))]
    for path in ["AppData", "Documents"] {
        assert!(!home.join(path).exists());
    }
}

#[test]
#[cfg(unix)]
fn preparation_failure_never_creates_home_or_relaxes_rules() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().canonicalize().unwrap().join("absent-home");
    let grant = home.join(".archon/operator-env-grants");
    let paths = denied_paths(&home);
    prepare_macos_ancestors(&home, &paths, &grant);
    assert!(!home.exists());
    let rules = macos_rules_for(&paths, &grant);
    assert!(rules.contains(&format!(
        "(deny file-write* (literal {}))",
        quoted(&home.join(".archon"))
    )));
    assert!(rules.contains(&format!("(deny file-write* (subpath {}))", quoted(&grant))));
}

#[test]
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn persistent_configuration_has_write_only_and_ancestor_rules() {
    let home = Path::new("/synthetic-operator");
    let mut rules = macos_rules_for(
        &denied_paths(home),
        &home.join(".archon/operator-env-grants"),
    );
    let mut ancestors = std::collections::HashSet::new();
    for path in config::user_paths(home) {
        assert!(!denied_paths(home).contains(&path));
        deny_macos_writes(&mut rules, &path, &mut ancestors);
        assert!(!rules.contains(&format!("(deny file-read* (subpath {}))", quoted(&path))));
        assert!(rules.contains(&format!("(deny file-write* (subpath {}))", quoted(&path))));
        for ancestor in path.ancestors().skip(1) {
            assert!(rules.contains(&format!(
                "(deny file-write* (literal {}))",
                quoted(ancestor)
            )));
        }
    }
    for path in crate::spawn::host_protected_paths() {
        assert!(!operator_paths().contains(&path));
        assert!(config::write_paths().contains(&path));
    }
    for name in [".profile", ".bashrc", ".zshenv"] {
        let path = home.join(name);
        assert!(!denied_paths(home).contains(&path));
        assert!(config::user_paths(home).contains(&path));
    }
}

#[test]
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn git_status_reads_real_global_config_under_boundary() {
    const PROBE: &str = "ARCHON_GIT_BOUNDARY_TEST_HOME";
    if std::env::var_os(PROBE).is_none() {
        let home = tempfile::tempdir().unwrap();
        std::fs::write(
            home.path().join(".gitconfig"),
            "[user]\nname = Boundary Test\n",
        )
        .unwrap();
        let output = crate::spawn::command(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "read_boundary::tests::git_status_reads_real_global_config_under_boundary",
                "--nocapture",
            ])
            .env("HOME", home.path())
            .env(PROBE, home.path())
            .output()
            .unwrap();
        eprint!("{}", String::from_utf8_lossy(&output.stderr));
        assert!(output.status.success(), "{output:?}");
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("1 passed"),
            "{output:?}"
        );
        return;
    }
    if !operator_grants_supported() {
        eprintln!(
            "SKIPPED real git-under-boundary test: kernel sandbox unavailable; no bounded git status was run"
        );
        return;
    }
    let root = tempfile::tempdir().unwrap();
    assert!(
        crate::spawn::command("git")
            .arg("init")
            .arg(root.path())
            .output()
            .unwrap()
            .status
            .success()
    );
    let (mut command, report) = command_with_report(OsStr::new("git"));
    // Force Git itself to read the real synthetic ~/.gitconfig, not the host snapshot.
    command.env(
        "GIT_CONFIG_GLOBAL",
        PathBuf::from(std::env::var_os(PROBE).unwrap()).join(".gitconfig"),
    );
    let output = command
        .current_dir(root.path())
        .args(["status", "--porcelain"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{report}: {output:?}");
}

#[test]
fn contained_filesystem_reports_stay_in_launch_logs() {
    let full = report("named-paths-denied", "named-paths-denied");
    assert!(full.contains("process reads: not-contained"));
    assert!(observation_note(full).is_none());
    for text in [
        report("not-contained", "named-paths-denied"),
        report("named-paths-denied", "not-contained"),
        "launch-refused: injected".into(),
    ] {
        assert_eq!(observation_note(text.clone()), Some(text));
    }
}

#[test]
#[cfg(target_os = "linux")]
fn inactive_ruleset_reports_mount_bypass_and_keeps_observation() {
    for reads in ["not-contained", "named-paths-denied"] {
        let text = report(reads, INACTIVE_MOUNT_WRITES);
        assert!(text.contains("mount-sealed; bypassable through /proc/<pid>/{root,cwd,fd} or ptrace of same-user host processes without Landlock"));
        assert!(!text.contains("write-sealed"));
        assert_eq!(observation_note(text.clone()), Some(text));
    }
}
