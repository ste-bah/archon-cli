//! Process isolation for tests that change the environment (which belongs to every thread).
const CHILD: &str = "ARCHON_ISOLATED_ENV_TEST";

/// True in the parent after running this exact test in a child; false in that child.
pub(crate) fn isolated() -> bool {
    let thread = std::thread::current();
    let name = thread.name().expect("test harness names each test thread");
    if std::env::var(CHILD).as_deref() == Ok(name) {
        std::fs::write(
            std::env::var("ARCHON_ISOLATED_ENV_MARKER").expect("child marker"),
            b"ran",
        )
        .expect("record exact test execution");
        return false;
    }
    let marker_dir = tempfile::tempdir().expect("isolated test marker directory");
    let marker = marker_dir.path().join("ran");
    let status = archon_shell::spawn::command(std::env::current_exe().unwrap())
        .args([
            "--exact",
            name,
            "--nocapture",
            "--test-threads=1",
            "--include-ignored",
        ])
        .env(CHILD, name)
        .env("ARCHON_ISOLATED_ENV_MARKER", &marker)
        .status()
        .expect("start isolated environment test");
    assert!(status.success(), "isolated test {name} failed: {status}");
    assert!(marker.is_file(), "child did not execute exact test {name}");
    true
}

/// Immutable build-time bindings for probes; never this process's mutable environment.
#[cfg(unix)]
pub(crate) fn probe_host() -> std::collections::BTreeMap<String, String> {
    let mut host = std::collections::BTreeMap::from([
        ("PATH".into(), env!("PATH").into()),
        ("LANG".into(), "C".into()),
        ("TZ".into(), "UTC".into()),
    ]);
    for (name, value) in [
        ("CARGO_HOME", option_env!("CARGO_HOME")),
        ("RUSTUP_HOME", option_env!("RUSTUP_HOME")),
        ("RUSTUP_TOOLCHAIN", option_env!("RUSTUP_TOOLCHAIN")),
        ("HOME", option_env!("HOME")),
    ] {
        if let Some(value) = value {
            host.insert(name.into(), value.into());
        }
    }
    host
}
