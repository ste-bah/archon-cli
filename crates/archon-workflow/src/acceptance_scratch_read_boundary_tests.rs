//! Exercise the shared direct/scratch launch, rather than a second sandbox.
use super::*;
use crate::acceptance_world::AuthorizedCommand;
use crate::task_set_contract::TrustedCwd;

fn site(root: &Path, scratch: bool) -> CommandSite<'_> {
    let mut environment: BTreeMap<_, _> = archon_shell::execution_environment::environment()
        .into_iter()
        .filter_map(|(key, value)| Some((key.into_string().ok()?, value.into_string().ok()?)))
        .collect();
    environment.insert("HOME".into(), root.display().to_string());
    CommandSite {
        project: root,
        repository: root,
        environment,
        resource_limits: Default::default(),
        audit_root: scratch.then_some(root),
        audit_target: None,
        scratch_bytes: u64::MAX,
        output_bytes: 16384,
        timeout_secs: 60,
        mutation_plan: None,
        mutation_root: None,
        redactor: None,
    }
}

async fn run(site: &CommandSite<'_>, text: &str) -> CheckResult {
    let command = AuthorizedCommand::for_test(text, TrustedCwd::ProjectRoot);
    run_at(
        site,
        "AC-BOUNDARY-001",
        &command,
        Arc::new(AtomicBool::new(false)),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn read_boundary_is_recorded_without_changing_a_passing_verdict() {
    let root = tempfile::tempdir().unwrap();
    for scratch in [false, true] {
        let result = run(
            &site(root.path(), scratch),
            "printf 'one check completed\\n'",
        )
        .await;
        assert!(result.passed(), "{result:?}");
        if archon_shell::read_boundary::operator_grants_supported() {
            assert!(result.environment_note.is_none(), "{result:?}");
        } else {
            let note = result
                .environment_note
                .as_deref()
                .expect("persisted incomplete boundary");
            assert!(note.contains("Filesystem read boundary:"), "{note}");
            assert!(note.contains("not-contained"), "{note}");
            #[cfg(windows)]
            assert!(note.contains("Startup folders and registry Run keys are not contained"));
        }
        let saved = serde_json::to_string(&result).unwrap();
        let restored: CheckResult = serde_json::from_str(&saved).unwrap();
        assert_eq!(restored.environment_note, result.environment_note);
        assert!(restored.passed(), "{restored:?}");
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[tokio::test]
async fn direct_and_scratch_checks_deny_credentials_and_config_writes() {
    const PROBE: &str = "ARCHON_CHECK_BOUNDARY_HOME";
    if std::env::var_os(PROBE).is_none() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path().canonicalize().unwrap();
        std::fs::create_dir(home.join(".aws")).unwrap();
        std::fs::create_dir_all(home.join(".config/git")).unwrap();
        std::fs::write(home.join(".config/git/config"), "[user]\nname = Xdg Test\n").unwrap();
        std::fs::write(home.join(".aws/credentials"), "synthetic-secret").unwrap();
        std::fs::write(home.join(".gitconfig"), "[user]\nname = Boundary Test\n").unwrap();
        std::fs::write(home.join(".bashrc"), "# operator startup\n").unwrap();
        #[cfg(target_os = "macos")]
        let autostart = "Library/LaunchAgents/probe.plist";
        #[cfg(target_os = "linux")]
        let autostart = ".config/systemd/user/probe.service";
        let path = home.join(autostart);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "original-startup").unwrap();
        let output = archon_shell::spawn::command(std::env::current_exe().unwrap())
            .args(["--exact", "acceptance_scratch::process::read_boundary_tests::direct_and_scratch_checks_deny_credentials_and_config_writes", "--nocapture"])
            .env(PROBE, &home)
            .env("HOME", &home)
            .output().unwrap();
        eprint!("{}", String::from_utf8_lossy(&output.stderr));
        assert!(output.status.success(), "{output:?}");
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("1 passed"),
            "{output:?}"
        );
        assert_eq!(
            std::fs::read_to_string(home.join(".gitconfig")).unwrap(),
            "[user]\nname = Boundary Test\n"
        );
        assert_eq!(std::fs::read_to_string(path).unwrap(), "original-startup");
        return;
    }
    if !archon_shell::read_boundary::operator_grants_supported() {
        eprintln!(
            "SKIPPED bounded direct/scratch credential and config-write probes: kernel sandbox unavailable; the launch-report test exercises the unbounded record"
        );
        return;
    }
    let _host_run = archon_shell::spawn::host_run();
    let home = PathBuf::from(std::env::var_os(PROBE).unwrap());
    let root = tempfile::tempdir().unwrap();
    let quote = |p: &Path| format!("'{}'", p.display().to_string().replace('\'', "'\\''"));
    let mut paths = vec![
        home.join(".aws/credentials"),
        home.join(".gitconfig"),
        home.join(".bashrc"),
        home.join(".config/git/config"),
    ];
    #[cfg(target_os = "macos")]
    paths.push(home.join("Library/LaunchAgents/probe.plist"));
    #[cfg(target_os = "linux")]
    paths.push(home.join(".config/systemd/user/probe.service"));
    let mut text = "set -eu\n".to_string();
    text.push_str(&format!(
        "if cat {} >/dev/null 2>&1; then exit 21; fi\n",
        quote(&home.join(".aws/credentials"))
    ));
    text.push_str(&format!(
        "cat {} >/dev/null\ncat {} >/dev/null\ngit init -q .\ngit config local.probe writable\ngit status --porcelain\n",
        quote(&home.join(".gitconfig")),
        quote(&home.join(".bashrc"))
    ));
    for path in paths {
        text.push_str(&format!(
            "if printf planted > {} 2>/dev/null; then exit 22; fi\n",
            quote(&path)
        ));
    }
    text.push_str("printf ordinary > ordinary\ncat ordinary\n");
    for scratch in [false, true] {
        let result = run(&site(root.path(), scratch), &text).await;
        assert!(result.passed(), "{result:?}");
        assert!(result.environment_note.is_none(), "{result:?}");
        assert_eq!(
            std::fs::read_to_string(root.path().join("ordinary")).unwrap(),
            "ordinary"
        );
    }
    // Register an operator repository with a pointer file, then exercise the
    // actual kernel rules while retaining writable siblings and nested scratch.
    std::fs::rename(root.path().join(".git"), root.path().join("metadata")).unwrap();
    let pointer = "gitdir: metadata\n";
    std::fs::write(root.path().join(".git"), pointer).unwrap();
    let config = std::fs::read(root.path().join("metadata/config")).unwrap();
    archon_shell::spawn::register_repository(root.path()).unwrap();
    let result = run(&site(root.path(), false),
        "set -eu\ncat .git metadata/config >/dev/null\nif printf planted > metadata/config 2>/dev/null; then exit 31; fi\nif printf planted > .git 2>/dev/null; then exit 32; fi\nmkdir target metadata/new-directory nested-scratch\nprintf writable > new-file\nprintf lock > metadata/index.lock\ncd nested-scratch\ngit init -q .\ngit config scratch.probe writable\n").await;
    assert!(result.passed(), "{result:?}");
    assert_eq!(
        std::fs::read_to_string(root.path().join(".git")).unwrap(),
        pointer
    );
    assert_eq!(
        std::fs::read(root.path().join("metadata/config")).unwrap(),
        config
    );
    assert_eq!(
        std::fs::read_to_string(root.path().join("new-file")).unwrap(),
        "writable"
    );
    assert!(root.path().join("target").is_dir());
    assert!(root.path().join("metadata/new-directory").is_dir());
    assert_eq!(
        std::fs::read_to_string(root.path().join("metadata/index.lock")).unwrap(),
        "lock"
    );
}

/// Opt in with an operator-owned build-check command. This lets each host use
/// its required resource/slot wrapper while still exercising the actual runner.
#[tokio::test]
#[ignore = "requires an operator-owned build-check command and resource wrapper"]
async fn r18_build_check_keeps_its_verdict_under_the_read_boundary() {
    let command = std::env::var("ARCHON_BOUNDARY_BUILD_CHECK")
        .expect("set ARCHON_BOUNDARY_BUILD_CHECK to the operator-owned build-check command");
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("src")).unwrap();
    std::fs::write(root.path().join("Cargo.toml"), "[package]\nname = \"boundary-build-probe\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[workspace]\n").unwrap();
    std::fs::write(
        root.path().join("src/lib.rs"),
        "#[test] fn completed() { assert_eq!(2 + 2, 4); }\n",
    )
    .unwrap();
    let mut at = site(root.path(), false);
    at.timeout_secs = 600;
    let target = root.path().join("target");
    at.environment
        .insert("CARGO_TARGET_DIR".into(), target.display().to_string());
    // A direct site's normal shared caches stay accessible; credential files
    // under those homes remain protected by the host's read boundary.
    for name in ["CARGO_HOME", "RUSTUP_HOME"] {
        if let Some(value) = std::env::var_os(name) {
            at.environment
                .insert(name.into(), value.to_string_lossy().into_owned());
        }
    }
    at.environment
        .insert("HOME".into(), std::env::var("HOME").unwrap());
    let result = run(&at, &command).await;
    assert!(result.passed(), "{result:?}");
    assert_eq!(result.exit_code, Some(0));
    assert!(
        std::fs::read_dir(target.join("debug/deps"))
            .unwrap()
            .any(|entry| {
                let path = entry.unwrap().path();
                let name = path.file_name().unwrap().to_string_lossy();
                (name.starts_with("boundary_build_probe-")
                    || name.starts_with("libboundary_build_probe-"))
                    && path.extension().is_none_or(|extension| extension != "d")
            }),
        "the check must actually compile the fixture"
    );
    if archon_shell::read_boundary::operator_grants_supported() {
        assert!(result.environment_note.is_none(), "{result:?}");
    } else {
        let note = result
            .environment_note
            .as_deref()
            .expect("incomplete boundary");
        assert!(note.contains("unbounded"), "{note}");
        assert!(note.contains("Filesystem read boundary:"), "{note}");
    }
}
