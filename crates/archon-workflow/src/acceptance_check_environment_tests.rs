//! The builder alone (Issue 345). The real spawn path is exercised by
//! `acceptance_scratch_direct_env_tests` and the bin's probe tests.

use super::*;
use std::path::PathBuf;

fn host(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    (pairs.iter())
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect()
}

const SECRETS: &[(&str, &str)] = &[
    ("FAKE_SECRET_TOKEN", "fake-secret-345"),
    ("AWS_SECRET_ACCESS_KEY", "aws-secret-345"),
    ("MY_PAT", "pat-secret-345"),
    ("ANTHROPIC_API_KEY", "engine-secret-345"),
];

/// The default policy keeps PATH, the locale and the toolchain homes, and
/// none of the operator's other variables.
#[test]
fn the_default_policy_binds_only_the_allowlist() {
    let mut pairs = vec![
        ("PATH", "/usr/bin:/bin"),
        ("LANG", "en_GB.UTF-8"),
        ("LC_ALL", "C"),
        ("TZ", "UTC"),
        ("RUSTUP_TOOLCHAIN", "stable"),
        ("CARGO_HOME", "/cargo"),
        ("PYTHONPATH", "/py"),
        ("SSH_AUTH_SOCK", "/tmp/agent"),
    ];
    pairs.extend_from_slice(SECRETS);
    let host = host(&pairs);
    let home = Path::new("/fresh/home");
    let environment =
        check_environment(&host, &CheckPolicy::default_for(&host), &[("HOME", home)]).unwrap();
    for (name, value) in [
        ("PATH", "/usr/bin:/bin"),
        ("LANG", "en_GB.UTF-8"),
        ("LC_ALL", "C"),
        ("TZ", "UTC"),
        ("RUSTUP_TOOLCHAIN", "stable"),
        ("CARGO_HOME", "/cargo"),
        ("HOME", "/fresh/home"),
    ] {
        assert_eq!(
            environment.get(name).map(String::as_str),
            Some(value),
            "{name}"
        );
    }
    for name in ["PYTHONPATH", "SSH_AUTH_SOCK"]
        .into_iter()
        .chain(SECRETS.iter().map(|(name, _)| *name))
    {
        assert!(!environment.contains_key(name), "{name} leaked");
    }
    let withheld = withheld(&host, &environment);
    assert!(withheld.contains("MY_PAT") && withheld.contains("PYTHONPATH"));
    assert!(!withheld.contains("PATH") && !withheld.contains("LANG"));
}

/// The toolchain homes a tool would find under the host's home are named
/// outright: the check is given a fresh one.
#[test]
fn toolchain_homes_under_the_host_home_are_named_outright() {
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(home.path().join(".cargo")).unwrap();
    std::fs::create_dir_all(home.path().join(".rustup")).unwrap();
    let name = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    let host = host(&[("PATH", "/bin"), (name, &home.path().to_string_lossy())]);
    let policy = CheckPolicy::default_for(&host);
    assert_eq!(
        policy.bound.get("CARGO_HOME").map(PathBuf::from),
        Some(home.path().join(".cargo"))
    );
    assert_eq!(
        policy.bound.get("RUSTUP_HOME").map(PathBuf::from),
        Some(home.path().join(".rustup"))
    );
    assert!(
        !policy.bound.contains_key(name),
        "the host home is never bound"
    );
}

/// A configured policy forwards exactly what it names; a forwarded name the
/// host lacks, or an execution control, is an error naming it, never a check
/// run without it.
#[test]
fn a_configured_policy_forwards_only_what_it_names_and_requires_it() {
    let host = host(&[
        ("PATH", "/host/bin"),
        ("LANG", "en_GB.UTF-8"),
        ("POLYGON_API_KEY", "forwarded-345"),
        ("MY_PAT", "pat-secret-345"),
    ]);
    let policy = CheckPolicy {
        toolchain_path: Some("/toolchain/bin".into()),
        bound: BTreeMap::from([("TZ".to_string(), "UTC".to_string())]),
        forwarded: vec!["POLYGON_API_KEY".into()],
    };
    let environment = check_environment(&host, &policy, &[]).unwrap();
    assert_eq!(environment["PATH"], "/toolchain/bin");
    assert_eq!(environment["TZ"], "UTC");
    assert_eq!(environment["POLYGON_API_KEY"], "forwarded-345");
    assert!(!environment.contains_key("MY_PAT"));
    assert!(
        !environment.contains_key("LANG"),
        "a policy binds its own locale"
    );
    let missing = CheckPolicy {
        forwarded: vec!["OPENBB_API_URL".into()],
        ..policy.clone()
    };
    let error = check_environment(&host, &missing, &[]).unwrap_err();
    assert!(error.contains("'OPENBB_API_URL' is absent"), "{error}");
    let control = CheckPolicy {
        forwarded: vec!["LD_PRELOAD".into()],
        ..policy
    };
    let error = check_environment(&host, &control, &[]).unwrap_err();
    assert!(error.contains("LD_PRELOAD"), "{error}");
}

/// A host with no PATH cannot run a check; that is said, not guessed.
#[test]
fn a_host_without_path_is_an_error_naming_path() {
    let host = host(&[("LANG", "C")]);
    let error = check_environment(&host, &CheckPolicy::default_for(&host), &[]).unwrap_err();
    assert!(error.contains("no PATH"), "{error}");
}

/// Only a whole-word name the site withheld makes a failure no verdict.
#[test]
fn a_withheld_name_is_matched_as_a_whole_word() {
    let withheld = BTreeSet::from(["MY_PAT".to_string()]);
    let error = withheld_error(&[b"test -n \"$MY_PAT\""], &withheld).expect("named");
    assert!(error.contains("MY_PAT") && error.contains("environment_allowlist"));
    assert_eq!(withheld_error(&[b"echo $MY_PATH MY_PATS"], &withheld), None);
    assert_eq!(withheld_error(&[b"exit 1"], &withheld), None);
}

/// The site's own directories are never displaced by the policy.
#[test]
fn site_directories_win_over_bound_values() {
    let host = host(&[("PATH", "/bin"), ("CARGO_TARGET_DIR", "/host/target")]);
    let target = Path::new("/site/target");
    let environment = site_variables(
        &host,
        &CheckPolicy::default_for(&host),
        &[("CARGO_TARGET_DIR", target)],
    );
    assert_eq!(environment["CARGO_TARGET_DIR"], "/site/target");
}

/// Windows: the system variables a process needs are kept at every site,
/// configured or not.
#[cfg(windows)]
#[test]
fn windows_system_variables_are_kept_at_every_site() {
    let host = host(&[
        ("Path", r"C:\Windows\System32"),
        ("SystemRoot", r"C:\Windows"),
        ("PATHEXT", ".COM;.EXE"),
        ("ComSpec", r"C:\Windows\System32\cmd.exe"),
        ("TEMP", r"C:\Temp"),
        ("MY_PAT", "pat-secret-345"),
    ]);
    let configured = CheckPolicy {
        toolchain_path: Some(r"C:\Tools".into()),
        ..CheckPolicy::default()
    };
    for policy in [CheckPolicy::default_for(&host), configured] {
        let environment = check_environment(&host, &policy, &[]).unwrap();
        for name in ["SystemRoot", "PATHEXT", "ComSpec", "TEMP"] {
            assert_eq!(environment.get(name), host.get(name), "{name}");
        }
        assert!(!environment.contains_key("MY_PAT"));
    }
}
