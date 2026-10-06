//! Issue 345 through the probe's real sites: its hermetic copy and the
//! direct site each run a check that prints `env`, and the operator's other
//! variables are not in its output. The host environment is an explicit map
//! (`with_host_environment`), never this process's own.

use super::super::probe_tests::trees;
use super::*;
use archon_workflow::task_set_contract::TrustedCwd;

const SECRETS: &[(&str, &str)] = &[
    ("FAKE_SECRET_TOKEN", "fake-secret-value-345"),
    ("AWS_SECRET_ACCESS_KEY", "aws-secret-value-345"),
    ("MY_PAT", "pat-secret-value-345"),
    ("ANTHROPIC_API_KEY", "engine-secret-value-345"),
];

/// The host's HOME in these tests.
const HOST_HOME: &str = "/nonexistent-host-home-345";

/// This process's PATH (read, never set) with the locale, a HOME and the
/// secrets.
fn host() -> BTreeMap<String, String> {
    let mut host: BTreeMap<String, String> = (SECRETS.iter())
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect();
    host.insert("PATH".into(), std::env::var("PATH").unwrap());
    host.insert("LANG".into(), "en_GB.UTF-8".into());
    host.insert("TZ".into(), "UTC".into());
    host.insert("HOME".into(), HOST_HOME.into());
    host
}

/// The `NAME=value` lines a passing `env` check printed, after checking no
/// secret's name or value is among them.
fn printed(results: &[CheckResult]) -> BTreeMap<String, String> {
    assert_eq!(results.len(), 1, "{results:?}");
    let result = &results[0];
    assert_eq!(result.exit_code, Some(0), "{result:?}");
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
    (text.lines())
        .filter_map(|line| line.split_once('='))
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect()
}

async fn run_env(probe: &HostProbe, trees: &super::super::probe_tests::Trees) -> Vec<CheckResult> {
    let contract = trees.contract();
    let digest = super::super::contract_digest(&contract).unwrap();
    let refs = super::super::refs_for(&contract, &digest, &trees.ids());
    probe.run(&contract, &digest, &refs).await
}

#[tokio::test]
async fn a_hermetic_check_gets_the_allowlist_and_no_operator_secret() {
    let trees = trees(&[("AC-E-001", "env", TrustedCwd::RepoRoot)]);
    let copies = tempfile::tempdir().unwrap();
    let probe = HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks)
        .with_copy_parent(copies.path().to_path_buf())
        .without_process_memo()
        .with_host_environment(host());
    assert!(matches!(probe.site, Site::Hermetic));
    let env = printed(&run_env(&probe, &trees).await);
    assert_eq!(env["PATH"], host()["PATH"]);
    assert_eq!(env["LANG"], "en_GB.UTF-8");
    assert_eq!(env["TZ"], "UTC");
    assert!(
        Path::new(&env["CARGO_TARGET_DIR"]).starts_with(copies.path()),
        "the probe's own warm target: {env:?}"
    );
    assert!(env["HOME"].contains("archon-check-home-"), "{env:?}");
}

#[tokio::test]
async fn a_direct_check_gets_the_allowlist_and_no_operator_secret() {
    let trees = trees(&[("AC-E-002", "env", TrustedCwd::RepoRoot)]);
    let probe = HostProbe::at(
        trees.set.project.path().to_path_buf(),
        trees.repo.clone(),
        None,
    )
    .with_host_environment(host());
    assert!(matches!(probe.site, Site::Direct));
    let env = printed(&run_env(&probe, &trees).await);
    assert_eq!(env["PATH"], host()["PATH"]);
    assert_eq!(env["LANG"], "en_GB.UTF-8");
    assert_eq!(env["TZ"], "UTC");
    assert_eq!(
        env["HOME"], HOST_HOME,
        "the direct site keeps the host's HOME"
    );
    // What the site records of itself (listing, identity, redaction) is the
    // same allowlist.
    let listed = listing_environment(&probe);
    for (name, _) in SECRETS {
        assert!(!listed.contains_key(*name), "{name} listed");
    }
    assert_eq!(listed["PATH"], host()["PATH"]);
}
