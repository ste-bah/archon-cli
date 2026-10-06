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

/// A failure is no verdict only when its output says a withheld variable
/// the allowlist can forward is missing; a bare mention (Rust's backtrace
/// hint), a name the allowlist refuses, or a look-alike name is a verdict.
#[test]
fn only_a_forwardable_variable_said_missing_makes_a_failure_no_verdict() {
    let withheld: BTreeSet<String> = ["MY_SERVICE_TOKEN", "RUST_BACKTRACE", "MY_PAT", "PYTHONPATH"]
        .into_iter()
        .map(String::from)
        .collect();
    for said in [
        "sh: 1: MY_SERVICE_TOKEN: parameter not set",
        "bash: line 1: MY_SERVICE_TOKEN: unbound variable",
        "Error: MY_SERVICE_TOKEN is not set",
        "KeyError: 'MY_SERVICE_TOKEN'",
        "missing environment variable MY_SERVICE_TOKEN",
        "thread 'main' panicked: MY_SERVICE_TOKEN must be set: NotPresent",
        // Library messages as they are printed (the review's table).
        "MY_SERVICE_TOKEN environment variable is not set",
        "django.core.exceptions.ImproperlyConfigured: Set the MY_SERVICE_TOKEN environment variable",
        "openai.OpenAIError: The api_key client option must be set either by passing api_key to the client or by setting the MY_SERVICE_TOKEN environment variable",
        "decouple.UndefinedValueError: MY_SERVICE_TOKEN not found. Declare it as envvar or define a default value.",
        "❌ Invalid environment variables: { MY_SERVICE_TOKEN: [ 'Required' ] }",
        "pydantic_core._pydantic_core.ValidationError: 1 validation error for Settings\nMY_SERVICE_TOKEN\n  Field required [type=missing, input_value={}, input_type=dict]",
        "Please export MY_SERVICE_TOKEN and retry",
        "MY_SERVICE_TOKEN is empty",
    ] {
        let error = withheld_error(&[said.as_bytes()], &withheld).expect(said);
        assert!(error.contains("MY_SERVICE_TOKEN") && error.contains("environment_allowlist"));
        assert!(
            !error.contains("MY_PAT") && !error.contains("PYTHONPATH"),
            "{error}"
        );
    }
    for verdict in [
        "note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace",
        "RUST_BACKTRACE is not set",
        "MY_PAT is not set",
        "PYTHONPATH is not set",
        "MY_SERVICE_TOKENS is not set",
        "uses MY_SERVICE_TOKEN; assertion failed",
        "note: MY_SERVICE_TOKEN=abc was used\nassertion failed: left == right",
        "Set the MY_PAT environment variable",
        "PYTHONPATH not found",
        // No name in the text: undetectable, so a verdict (see `says_missing`).
        "thread 'main' panicked: called `Result::unwrap()` on an `Err` value: NotPresent",
        "exit 1",
    ] {
        assert_eq!(
            withheld_error(&[verdict.as_bytes()], &withheld),
            None,
            "{verdict}"
        );
    }
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

/// Windows: a withheld name is matched ignoring case, as `withheld` and the
/// lookup ignore it there.
#[cfg(windows)]
#[test]
fn windows_withheld_names_match_ignoring_case() {
    let host = host(&[("Path", r"C:\Windows"), ("My_Service_Token", "t")]);
    let environment = check_environment(&host, &CheckPolicy::default_for(&host), &[]).unwrap();
    let withheld = withheld(&host, &environment);
    assert!(withheld.contains("My_Service_Token"));
    let error = withheld_error(&[b"MY_SERVICE_TOKEN is not set"], &withheld).expect("matched");
    assert!(error.contains("My_Service_Token"), "{error}");
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
