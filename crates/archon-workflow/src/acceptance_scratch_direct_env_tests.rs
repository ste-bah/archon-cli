//! Issue 345 on the real spawn path: a direct (and a probe-copy shaped) site
//! runs `env` and the operator's other variables are not in its output. The
//! host environment is an explicit map, never this process's own.

use super::tests::{contract, criterion, reference, site};
use super::*;
use crate::task_set_contract::TrustedCwd;

const SECRETS: &[(&str, &str)] = &[
    ("FAKE_SECRET_TOKEN", "fake-secret-value-345"),
    ("AWS_SECRET_ACCESS_KEY", "aws-secret-value-345"),
    ("MY_PAT", "pat-secret-value-345"),
    ("ANTHROPIC_API_KEY", "engine-secret-value-345"),
];

fn host_with_secrets(home: &Path) -> BTreeMap<String, String> {
    let mut host: BTreeMap<String, String> = SECRETS
        .iter()
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect();
    for (name, value) in [
        ("PATH", "/usr/bin:/bin"),
        ("LANG", "en_GB.UTF-8"),
        ("TZ", "UTC"),
        ("CARGO_TARGET_DIR", "/host/target-345"),
        ("HOME", &home.to_string_lossy()),
    ] {
        host.insert(name.into(), value.into());
    }
    host
}

/// `command` at `site`, its id REQ-1.
async fn run(site: &DirectSite, command: &str) -> WorkflowResult<CheckResult> {
    let (contract, digest) = contract(vec![criterion("REQ-1", command, TrustedCwd::RepoRoot)]);
    run_check_direct(
        site,
        &contract,
        &digest,
        &reference(&contract, &digest, "REQ-1"),
        Arc::new(AtomicBool::new(false)),
    )
    .await
}

fn printed(result: &CheckResult) -> BTreeMap<String, String> {
    assert_eq!(result.exit_code, Some(0), "{result:?}");
    String::from_utf8_lossy(&result.stdout)
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect()
}

fn assert_no_secret(result: &CheckResult) {
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    for (name, value) in SECRETS {
        assert!(
            !text.contains(name) && !text.contains(value),
            "{name} leaked: {text}"
        );
    }
}

/// A direct site with no policy: secrets absent; PATH, the locale, the
/// host's build directory and the toolchain home under the host's home
/// present; HOME a fresh directory, never the host's.
#[tokio::test]
async fn a_direct_check_gets_the_allowlist_and_no_operator_secret() {
    let repo = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(home.path().join(".cargo")).unwrap();
    let mut site = site(repo.path(), repo.path());
    site.host = host_with_secrets(home.path());
    let result = run(&site, "env").await.unwrap();
    assert_no_secret(&result);
    let env = printed(&result);
    assert_eq!(env["PATH"], "/usr/bin:/bin");
    assert_eq!(env["LANG"], "en_GB.UTF-8");
    assert_eq!(env["TZ"], "UTC");
    assert_eq!(env["CARGO_TARGET_DIR"], "/host/target-345");
    assert_eq!(
        PathBuf::from(&env["CARGO_HOME"]),
        home.path().join(".cargo")
    );
    assert_ne!(PathBuf::from(&env["HOME"]), home.path());
    assert!(env["HOME"].contains("archon-check-home-"), "{env:?}");
    assert!(
        !Path::new(&env["HOME"]).exists(),
        "the fresh home is removed after the check"
    );
}

/// A probe-copy shaped site (its own build directory): the same rule, and
/// its build directory displaces the host's.
#[tokio::test]
async fn a_hermetic_shaped_check_gets_its_own_build_directory_and_no_secret() {
    let repo = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let mut site = site(repo.path(), repo.path());
    site.host = host_with_secrets(home.path());
    site.target = Some(repo.path().join("warm-target"));
    let result = run(&site, "env").await.unwrap();
    assert_no_secret(&result);
    let env = printed(&result);
    assert_eq!(
        PathBuf::from(&env["CARGO_TARGET_DIR"]),
        repo.path().join("warm-target")
    );
    assert_eq!(env["PATH"], "/usr/bin:/bin");
}

/// A configured policy's forwarded variable reaches the check; one the host
/// lacks stops it with an error naming it.
#[tokio::test]
async fn a_forwarded_variable_is_present_and_a_missing_one_is_named() {
    let repo = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let mut site = site(repo.path(), repo.path());
    site.host = host_with_secrets(home.path());
    site.host
        .insert("POLYGON_API_KEY".into(), "forwarded-345".into());
    site.policy = Some(CheckPolicy {
        toolchain_path: Some("/usr/bin:/bin".into()),
        bound: BTreeMap::new(),
        forwarded: vec!["POLYGON_API_KEY".into()],
    });
    let result = run(&site, "env").await.unwrap();
    assert_no_secret(&result);
    let env = printed(&result);
    assert_eq!(env["POLYGON_API_KEY"], "forwarded-345");
    assert!(!env.contains_key("LANG"), "a policy binds its own locale");
    site.host.remove("POLYGON_API_KEY");
    let error = run(&site, "env").await.unwrap_err().to_string();
    assert!(error.contains("'POLYGON_API_KEY' is absent"), "{error}");
}

/// A check that fails reading a variable the site withheld gives no
/// verdict but the operational error naming it; a failure naming none stays
/// the check's verdict.
#[tokio::test]
async fn a_failure_reading_a_withheld_variable_is_an_operational_error() {
    let repo = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let mut site = site(repo.path(), repo.path());
    site.host = host_with_secrets(home.path());
    let result = run(&site, "test -n \"$MY_PAT\"").await.unwrap();
    let error = result.operational_error.as_deref().expect("no verdict");
    assert!(error.contains("MY_PAT"), "{error}");
    assert!(!error.contains("pat-secret-value-345"), "{error}");
    let result = run(&site, "test -f absent").await.unwrap();
    assert_eq!(result.exit_code, Some(1), "{result:?}");
    assert!(result.operational_error.is_none(), "{result:?}");
}
