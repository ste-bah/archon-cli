use super::*;

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
fn credential_and_profile_paths_are_denied_by_absolute_path() {
    let home = Path::new("/synthetic-operator");
    let paths = denied_paths(home);
    for name in [
        ".archon/.credentials.json",
        ".aws",
        ".ssh",
        ".profile",
        ".bashrc",
        ".zshenv",
        ".cargo/credentials.toml",
        ".codex/auth.json",
    ] {
        assert!(paths.contains(&home.join(name)));
    }
    let rules = macos_rules(&paths);
    assert!(rules.contains("(deny file-read* (subpath \"/synthetic-operator/.aws\"))"));
    assert!(rules.contains("(deny file-write* (subpath \"/synthetic-operator/.aws\"))"));
    assert!(!rules.contains("(deny file-write* (literal "));
    for ancestor in ["/synthetic-operator", "/"] {
        let rule = format!(
            "(deny file-write-unlink file-write-mode file-write-owner file-write-flags file-write-setugid file-write-xattr (literal \"{ancestor}\"))"
        );
        assert_eq!(rules.matches(&rule).count(), 1, "{rule}");
        let rule = format!(
            "(deny file-write-create (require-all (literal \"{ancestor}\") (vnode-type SYMLINK)))"
        );
        assert_eq!(rules.matches(&rule).count(), 1, "{rule}");
    }
}

#[test]
#[cfg(target_os = "macos")]
fn credential_paths_cannot_be_moved_out_of_the_boundary() {
    if crate::write_boundary::mechanism().is_err() {
        eprintln!("kernel boundary unavailable; profile rules are tested separately");
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let parent = temp.path().join("parent");
    let protected = parent.join("protected");
    std::fs::create_dir_all(&protected).unwrap();
    std::fs::write(protected.join("key"), "synthetic-secret").unwrap();
    let missing = temp.path().join("missing");
    let future_protected = missing.join("nested/protected");
    let mut rules = macos_rules(&[protected.clone(), future_protected]);
    let grant_root = temp.path().join(".archon/operator-env-grants");
    deny_macos_writes(
        &mut rules,
        &grant_root,
        &mut std::collections::HashSet::new(),
    );
    std::fs::create_dir(temp.path().join("redirect")).unwrap();
    let output = crate::spawn::command(crate::write_boundary::SANDBOX_EXEC)
        .args([
            "-p",
            &format!("(version 1)(allow default){rules}"),
            "/bin/sh",
            "-c",
            r#"set -eu
if mv "$1/protected" "$2/moved-protected"; then exit 11; fi
if mv "$1" "$2/moved-parent"; then exit 12; fi
if cat "$1/protected/key"; then exit 13; fi
if cp "$1/protected/key" "$2/copied-key"; then exit 14; fi
printf ordinary > "$1/new-file"
mkdir -p "$2/missing/nested"
if mv "$2/missing/nested" "$2/moved-nested"; then exit 15; fi
if mv "$2/missing" "$2/moved-missing"; then exit 16; fi
if rmdir "$2/missing/nested"; then exit 17; fi
printf ordinary > "$2/missing/nested/new-file"
if chmod 700 "$1"; then exit 18; fi
if ln -s "$2/redirect" "$2/.archon"; then exit 19; fi
if mkdir -p "$2/.archon/operator-env-grants" && printf forged > "$2/.archon/operator-env-grants/grant"; then exit 20; fi
"#,
            "probe",
        ])
        .arg(&parent)
        .arg(temp.path())
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty(), "secret must not be printed");
    assert!(!temp.path().join("redirect/operator-env-grants").exists());
    assert!(!grant_root.join("grant").exists());
    assert!(!temp.path().join(".archon").is_symlink());
    assert_eq!(
        std::fs::read_to_string(protected.join("key")).unwrap(),
        "synthetic-secret"
    );
    for name in [
        "moved-protected",
        "moved-parent",
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
    std::fs::create_dir(home.path().join(".aws")).unwrap();
    let secret = home.path().join(".aws/credentials");
    std::fs::write(&secret, "sk-test-file-355").unwrap();
    #[cfg(target_os = "linux")]
    let mut command = crate::spawn::command("/bin/sh");
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = crate::spawn::command(crate::write_boundary::SANDBOX_EXEC);
        command.args([
            "-p",
            &format!(
                "(version 1)(allow default){}",
                macos_rules(&denied_paths(home.path()))
            ),
            "/bin/sh",
        ]);
        command
    };
    #[cfg(target_os = "linux")]
    crate::write_boundary::landlock::LandlockSandbox::build_read(&denied_paths(home.path()))
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
