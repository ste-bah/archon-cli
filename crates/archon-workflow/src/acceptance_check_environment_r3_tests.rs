use super::*;

macro_rules! ordinary {
    ($id:ident, $text:expr) => {
        #[test]
        fn $id() {
            let withheld = BTreeSet::from(["FIXTURE_API_KEY".into()]);
            assert!(
                withheld_note(&[$text.as_bytes()], &withheld).is_some(),
                "{}",
                $text
            );
        }
    };
}
ordinary!(r3_setup_success, "setup: set FIXTURE_API_KEY successfully");
ordinary!(
    r3_escaped_expectation,
    r#"expected "error \"FIXTURE_API_KEY environment variable is not set\"" in stderr"#
);
ordinary!(
    r3_escaped_backslash_expectation,
    r#"expected "error \\"FIXTURE_API_KEY environment variable is not set\\"" in stderr"#
);
ordinary!(
    r3_export_success,
    "setup: export FIXTURE_API_KEY successfully"
);
ordinary!(
    r3_provide_success,
    "setup: provide FIXTURE_API_KEY successfully"
);

macro_rules! diagnostic {
    ($id:ident, $text:expr) => {
        #[test]
        fn $id() {
            let withheld = BTreeSet::from(["FIXTURE_API_KEY".into()]);
            assert!(
                withheld_note(&[$text.as_bytes()], &withheld).is_some(),
                "{}",
                $text
            );
            let after_expectation = format!("expected a different error\n{}", $text);
            assert!(withheld_note(&[after_expectation.as_bytes()], &withheld).is_some());
            assert_eq!(withheld_note(&[$text.as_bytes()], &BTreeSet::new()), None);
        }
    };
}
diagnostic!(
    r3_rust_err_string,
    r#"called Result::unwrap() on an Err value: "FIXTURE_API_KEY environment variable is not set""#
);
diagnostic!(
    r3_json_error,
    r#"{"error":"FIXTURE_API_KEY environment variable is not set"}"#
);
diagnostic!(
    r3_json_message,
    r#"{"message":"missing environment variable FIXTURE_API_KEY"}"#
);

#[cfg(unix)]
fn configured_host(locator: &str) -> (BTreeMap<String, String>, CheckPolicy) {
    let host = host(&[
        ("PATH", "/bin"),
        ("HOME", "/cold"),
        (locator, "/leased/toolchain"),
        ("FIXTURE_API_KEY", "fixture"),
    ]);
    let policy = CheckPolicy {
        toolchain_path: Some("/usr/bin:/bin".into()),
        forwarded: vec!["FIXTURE_API_KEY".into()],
        ..Default::default()
    };
    (host, policy)
}

#[cfg(unix)]
#[test]
fn r3_configured_host_cargo_home() {
    let (host, policy) = configured_host("CARGO_HOME");
    let environment = CommandEnvironment::from_host(&host, Some(&policy)).unwrap();
    assert_eq!(
        environment.variables.get("CARGO_HOME"),
        host.get("CARGO_HOME")
    );
    assert_eq!(
        environment.variables.get("FIXTURE_API_KEY"),
        host.get("FIXTURE_API_KEY")
    );
}

#[cfg(unix)]
#[test]
fn r3_configured_host_rustup_home() {
    let (host, policy) = configured_host("RUSTUP_HOME");
    let environment = CommandEnvironment::from_host(&host, Some(&policy)).unwrap();
    assert_eq!(
        environment.variables.get("RUSTUP_HOME"),
        host.get("RUSTUP_HOME")
    );
}

#[cfg(unix)]
#[test]
fn r3_configured_host_cache_and_dispatch_priority() {
    let (host, mut policy) = configured_host("GOMODCACHE");
    policy.bind_dispatch(&[("CARGO_HOME".into(), "/dispatch/cargo".into())]);
    let environment = CommandEnvironment::from_host(&host, Some(&policy)).unwrap();
    assert_eq!(
        environment.variables.get("GOMODCACHE"),
        host.get("GOMODCACHE")
    );
    assert_eq!(
        environment.variables.get("CARGO_HOME").unwrap(),
        "/dispatch/cargo"
    );
}

#[cfg(unix)]
#[test]
fn r3_configured_host_offline_cargo_dependency() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let cargo_home = root.join("leased-cargo");
    let vendor = cargo_home.join("vendor/offline-fixture-1.0.0");
    std::fs::create_dir_all(&cargo_home).unwrap();
    std::fs::create_dir_all(vendor.join("src")).unwrap();
    std::fs::create_dir_all(root.join("project/src")).unwrap();
    std::fs::write(
        vendor.join("Cargo.toml"),
        "[package]\nname='offline-fixture'\nversion='1.0.0'\nedition='2021'\n",
    )
    .unwrap();
    std::fs::write(vendor.join("src/lib.rs"), "pub fn fixture() {}\n").unwrap();
    std::fs::write(
        vendor.join(".cargo-checksum.json"),
        r#"{"files":{},"package":null}"#,
    )
    .unwrap();
    std::fs::write(
        cargo_home.join("config.toml"),
        format!(
            "[source.crates-io]\nreplace-with='leased'\n[source.leased]\ndirectory={:?}\n",
            cargo_home.join("vendor").display().to_string()
        ),
    )
    .unwrap();
    std::fs::write(root.join("project/Cargo.toml"), "[package]\nname='offline-consumer'\nversion='0.1.0'\nedition='2021'\n[dependencies]\noffline-fixture='1.0.0'\n").unwrap();
    std::fs::write(
        root.join("project/src/lib.rs"),
        "pub fn consume() { offline_fixture::fixture(); }\n",
    )
    .unwrap();
    let mut host = host_environment();
    let rustup_home = std::env::var("RUSTUP_HOME")
        .unwrap_or_else(|_| format!("{}/.rustup", std::env::var("HOME").unwrap()));
    host.insert("RUSTUP_HOME".into(), rustup_home);
    host.insert("HOME".into(), root.join("cold-home").display().to_string());
    host.insert("CARGO_HOME".into(), cargo_home.display().to_string());
    host.insert("FIXTURE_API_KEY".into(), "fixture".into());
    let policy = CheckPolicy {
        toolchain_path: host.get("PATH").cloned(),
        bound: BTreeMap::from([("RUSTUP_HOME".into(), host["RUSTUP_HOME"].clone())]),
        forwarded: vec!["FIXTURE_API_KEY".into()],
    };
    let environment = CommandEnvironment::from_host(&host, Some(&policy)).unwrap();
    let cargo = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap()
        .join(".cargo/bin/cargo");
    let output = environment
        .command(cargo)
        .args([
            "metadata",
            "--offline",
            "--format-version",
            "1",
            "--manifest-path",
        ])
        .arg(root.join("project/Cargo.toml"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("offline-fixture"));
}

#[test]
fn r3_shell_maintained_names_are_not_candidates() {
    let host = BTreeMap::from([
        ("_".into(), "shell-command".into()),
        ("PWD".into(), "/tmp".into()),
        ("OLDPWD".into(), "/".into()),
        ("SHLVL".into(), "2".into()),
        ("FIXTURE_API_KEY".into(), "secret".into()),
    ]);
    let note = withheld_note(
        &[b"ordinary_snake_case failed"],
        &withheld(&host, &BTreeMap::new()),
    );
    assert_eq!(note, None);
}

#[test]
fn r3_variable_names_require_identifier_boundaries() {
    let withheld = BTreeSet::from(["FIXTURE_API_KEY".into()]);
    assert!(withheld_note(&[b"FIXTURE_API_KEY"], &withheld).is_some());
    assert!(withheld_note(&[b"(FIXTURE_API_KEY)"], &withheld).is_some());
    assert_eq!(
        withheld_note(&[b"prefix_FIXTURE_API_KEY_suffix"], &withheld),
        None
    );
}
