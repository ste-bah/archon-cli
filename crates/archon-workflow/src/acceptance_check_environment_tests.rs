//! The builder alone (Issue 345). The real spawn path is exercised by
//! `acceptance_scratch_direct_env_tests` and the bin's probe tests.

use super::*;
use std::collections::BTreeSet;
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
        ("NODE_OPTIONS", "--require=x"),
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
    for name in ["NODE_OPTIONS", "SSH_AUTH_SOCK"]
        .into_iter()
        .chain(SECRETS.iter().map(|(name, _)| *name))
    {
        assert!(!environment.contains_key(name), "{name} leaked");
    }
    let withheld = withheld(&host, &environment);
    assert!(withheld.contains("MY_PAT") && withheld.contains("NODE_OPTIONS"));
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

/// The non-secret locators reach a check at a site with no policy; a proxy
/// address carrying credentials does not, and neither does any
/// credential-shaped name.
#[test]
fn locators_and_credential_free_proxies_are_bound_and_nothing_credential_shaped() {
    let host = host(&[
        ("PATH", "/bin"),
        ("VIRTUAL_ENV", "/venv"),
        ("PYTHONPATH", "/py"),
        ("NVM_DIR", "/nvm"),
        ("GOPATH", "/go"),
        ("JAVA_HOME", "/jdk"),
        ("SSL_CERT_FILE", "/certs.pem"),
        ("NODE_EXTRA_CA_CERTS", "/extra.pem"),
        ("HTTPS_PROXY", "http://proxy.internal:3128"),
        ("http_proxy", "http://alice:s3cret@proxy.internal:3128"),
        ("NO_PROXY", "localhost,127.0.0.1"),
        ("RUST_BACKTRACE", "1"),
    ]);
    let bound = CheckPolicy::default_for(&host).bound;
    for name in [
        "VIRTUAL_ENV",
        "PYTHONPATH",
        "NVM_DIR",
        "GOPATH",
        "JAVA_HOME",
        "SSL_CERT_FILE",
        "NODE_EXTRA_CA_CERTS",
        "HTTPS_PROXY",
        "NO_PROXY",
    ] {
        assert_eq!(bound.get(name), host.get(name), "{name}");
    }
    assert!(
        !bound.contains_key("http_proxy"),
        "a proxy with credentials"
    );
    assert!(!bound.contains_key("RUST_BACKTRACE"));
    for name in DEFAULT_BOUND.iter().chain(PROXY_VARIABLES) {
        assert!(!credential_shaped(name), "{name} is credential-shaped");
    }
    for name in [
        "GITHUB_TOKEN",
        "SERVICE_API_KEY",
        "DB_PASSWORD",
        "MY_PAT",
        "AWS_SECRET",
    ] {
        assert!(credential_shaped(name), "{name}");
    }
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

/// A proxy value is bound only as a bare address: no credentials, query or
/// fragment, and no path but `/`; a NO_PROXY list keeps its CIDR blocks.
#[test]
fn only_a_bare_proxy_address_is_bound() {
    let bound = |name: &str, value: &str| {
        let host = host(&[("PATH", "/bin"), (name, value)]);
        CheckPolicy::default_for(&host).bound.contains_key(name)
    };
    assert!(bound("HTTPS_PROXY", "http://proxy.internal:3128"));
    assert!(bound("HTTPS_PROXY", "http://proxy.internal:3128/"));
    assert!(bound("NO_PROXY", "localhost,10.0.0.0/8"));
    for value in [
        "http://alice:s3cret@proxy.internal:3128",
        "http://proxy.internal:3128/?access_token=tok",
        "http://proxy.internal:3128?token=tok",
        "http://proxy.internal:3128/#tok",
        "http://proxy.internal:3128/token/tok",
    ] {
        assert!(!bound("HTTPS_PROXY", value), "{value}");
    }
    assert!(!bound("no_proxy", "alice@host"));
}

/// Windows: a site with its own home points the profile variables at it.
#[cfg(windows)]
#[test]
fn windows_profile_variables_point_at_the_site_home() {
    let home = Path::new(r"D:\scratch\home");
    let bindings: BTreeMap<&str, PathBuf> = profile_bindings(home).into_iter().collect();
    for name in ["USERPROFILE", "APPDATA", "LOCALAPPDATA"] {
        assert_eq!(bindings[name], home, "{name}");
    }
    assert_eq!(bindings["HOMEDRIVE"], PathBuf::from("D:"));
    assert_eq!(bindings["HOMEPATH"], PathBuf::from(r"\scratch\home"));
}

#[path = "acceptance_check_environment_r3_tests.rs"]
mod round_three;
#[path = "acceptance_check_environment_shell_names_tests.rs"]
mod shell_names;

/// Round 5 rule: a withheld name is noted only where it appears as a whole
/// identifier (case-sensitive, no prose parsing); values are never shown.
#[test]
fn r5_notes_are_case_sensitive_identifier_matches_without_parsing() {
    let env = CommandEnvironment::from_host(
        &host(&[
            ("PATH", "/bin"),
            ("FIXTURE_API_KEY", "hidden-data"),
            ("BASH_ENV", "hidden-script"),
        ]),
        None,
    )
    .unwrap();
    for text in [
        "E AssertionError: expected FIXTURE_API_KEY environment variable is not set in stderr",
        "FIXTURE_API_KEY environment variable is not set",
        r#"{"failures":["FIXTURE_API_KEY"]}"#,
    ] {
        let note = env.note(&[text.as_bytes()]).expect(text);
        assert!(note.contains("FIXTURE_API_KEY"), "{note}");
        assert!(
            !note.contains("hidden-data") && !note.contains("BASH_ENV"),
            "{note}"
        );
    }
    let note = env
        .note(&[b"prefixFIXTURE_API_KEYsuffix BASH_ENV"])
        .expect("whole identifier BASH_ENV is noted");
    assert!(
        note.contains("BASH_ENV") && !note.contains("hidden-script"),
        "{note}"
    );
    assert!(
        !note.contains("FIXTURE_API_KEY"),
        "embedded name noted: {note}"
    );
    for text in [
        "prefixFIXTURE_API_KEYsuffix",
        "MY_FIXTURE_API_KEY_2",
        "fixture_api_key is missing",
        "Fixture_Api_Key is missing",
    ] {
        assert!(env.note(&[text.as_bytes()]).is_none(), "{text}");
    }
}

#[cfg(unix)]
fn non_unicode_note_case(case: &str, text: &str) {
    use std::os::unix::ffi::OsStringExt;
    if std::env::var("ISSUE_349_CASE").as_deref() != Ok(case) {
        let home = tempfile::tempdir().unwrap();
        let output = archon_shell::spawn::command(std::env::current_exe().unwrap())
            .args([case, "--nocapture"])
            .env_clear()
            .env("ISSUE_349_CASE", case)
            .env("HOME", home.path())
            .env("PATH", std::env::var_os("PATH").unwrap())
            .env("CARGO_BUILD_JOBS", "2")
            .env("RUST_TEST_THREADS", "4")
            .env(
                "FIXTURE_API_KEY",
                std::ffi::OsString::from_vec(vec![0xff, 0xfe]),
            )
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let environment = CommandEnvironment::capture(None).unwrap();
    let output = environment
        .command("/bin/sh")
        .args([
            "-c",
            "test -z \"${FIXTURE_API_KEY-}\" || exit 4; printf '%s' \"$1\"; exit 3",
            "fixture",
            text,
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(3));
    let note = environment
        .note(&[&output.stdout, &output.stderr])
        .expect("withheld name survives non-Unicode value");
    assert!(note.contains("FIXTURE_API_KEY") && !note.contains('\u{fffd}'));
    assert!(environment.note(&[b"fixture_api_key"]).is_none());
    assert!(
        environment
            .note(&[b"prefixFIXTURE_API_KEYsuffix"])
            .is_none()
    );
}

#[cfg(unix)]
#[test]
fn r4_non_unicode_pytest_note() {
    non_unicode_note_case(
        "r4_non_unicode_pytest_note",
        "E AssertionError: expected FIXTURE_API_KEY environment variable is not set in stderr",
    );
}
#[cfg(unix)]
#[test]
fn r4_non_unicode_json_note() {
    non_unicode_note_case(
        "r4_non_unicode_json_note",
        r#"{"status":"failed","failures":["FIXTURE_API_KEY environment variable is not set"]}"#,
    );
}
#[cfg(unix)]
#[test]
fn r5_non_unicode_punctuated_identifier_note() {
    non_unicode_note_case(
        "r5_non_unicode_punctuated_identifier_note",
        "prefix-FIXTURE_API_KEY-suffix",
    );
}
