//! Old total-clock strikes are not evidence against a check's author.
use super::super::probe_tests::trees;
use super::*;
use archon_workflow::task_set_contract::TrustedCwd;

#[test]
fn issue356_legacy_total_clock_strikes_do_not_authorize_a_finding() {
    let trees = trees(&[("AC-B-001", "test -f feature.txt", TrustedCwd::RepoRoot)]);
    let probe = HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks);
    let contract = trees.contract();
    let path = strike(&probe, &trees.base, &contract, "AC-B-001");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    for bound in [1_800, 7_200, 20_000] {
        std::fs::write(
            &path,
            format!("it ran past the probe's per-check bound of {bound}s"),
        )
        .unwrap();
        let finding = strike_or_unproven(
            &probe,
            &trees.base,
            &contract,
            "AC-B-001",
            None,
            ("a tool could not run", UnprovenCause::Host),
            || ("author finding".into(), "host unproven".into()),
        );
        assert!(
            finding.is_none(),
            "the old clock alone cannot authorize an author finding"
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "a tool could not run",
            "only current non-clock evidence gets a strike"
        );
    }
}

fn set_command(contract: &mut AcceptanceContract, command: &str) {
    match &mut contract.acceptance[0].check {
        archon_workflow::task_set_contract::AcceptanceCheck::Command { command: text, .. } => {
            *text = command.to_string();
        }
        _ => panic!("command check fixture"),
    }
}

fn failed_result(id: &str, stderr: &str) -> CheckResult {
    CheckResult {
        acceptance_id: id.into(),
        exit_code: Some(101),
        quota_walk_count: 0,
        stdout: Vec::new(),
        stderr: stderr.as_bytes().to_vec(),
        environment_note: None,
        operational_error: None,
        classification: None,
    }
}

#[test]
fn issue475_same_missing_setup_path_after_reauthoring_goes_to_author() {
    let trees = trees(&[("AC-P-001", "cargo test", TrustedCwd::ProjectRoot)]);
    let probe = HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks);
    let mut contract = trees.contract();
    set_command(
        &mut contract,
        "cargo test --manifest-path ../../repo/Cargo.toml",
    );
    let first_text = "cargo test --manifest-path ../../repo/a/../b/";
    set_command(&mut contract, first_text);
    let result = failed_result("AC-P-001", "../../repo/a/../b does not exist");
    let at = Context::on_host_path(&contract).within(1, &["repo", "project"]);
    let why = super::super::verdict::no_verdict(first_text, &result, &at).expect("no verdict");
    assert!(why.contains("setup failure of its site"), "{why}");
    assert!(settle(&probe, &trees.base, &contract, &result, &why, &at,).is_none());
    set_command(&mut contract, "cargo test --manifest-path ../../repo/b");
    let text = check_text(&contract, "AC-P-001");
    let result = failed_result("AC-P-001", "../../repo/b does not exist");
    let why = super::super::verdict::no_verdict(text, &result, &at).expect("no verdict");
    let finding = settle(&probe, &trees.base, &contract, &result, &why, &at)
        .expect("same setup failure routes to the author after resume");
    assert!(finding.contains("It failed so again"), "{finding}");
}

#[test]
fn issue475_different_missing_path_and_commit_are_fresh_strikes() {
    let trees = trees(&[("AC-P-001", "cargo test", TrustedCwd::ProjectRoot)]);
    let probe = HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks);
    let mut contract = trees.contract();
    set_command(
        &mut contract,
        "cargo test --manifest-path ../../repo/Cargo.toml",
    );
    let result = failed_result("AC-P-001", "../../repo/Cargo.toml does not exist");
    let at = Context::on_host_path(&contract).within(1, &["repo", "project"]);
    let text = check_text(&contract, "AC-P-001");
    let why = super::super::verdict::no_verdict(text, &result, &at).unwrap();
    assert!(settle(&probe, &trees.base, &contract, &result, &why, &at,).is_none());
    assert!(
        settle(
            &probe,
            &trees.base,
            &contract,
            &failed_result("AC-P-001", "../../repo/other.toml does not exist"),
            &why,
            &at,
        )
        .is_none(),
        "a different missing path starts a fresh strike"
    );
    assert!(
        settle(&probe, &trees.head, &contract, &result, &why, &at,).is_none(),
        "a different commit starts a fresh strike"
    );
    let mut other_root = contract.clone();
    match &mut other_root.acceptance[0].check {
        archon_workflow::task_set_contract::AcceptanceCheck::Command { cwd, .. } => {
            *cwd = TrustedCwd::RepoRoot
        }
        _ => panic!("command check fixture"),
    }
    let setup_a = setup_path_signature(&contract, "AC-P-001", &result, &why, &at).unwrap();
    let at_other = Context::on_host_path(&other_root).within(1, &["repo", "project"]);
    let setup_b = setup_path_signature(&other_root, "AC-P-001", &result, &why, &at_other).unwrap();
    assert_ne!(
        setup_a, setup_b,
        "the selected root is part of the setup identity"
    );
}

#[test]
fn issue475_no_exit_kills_with_changed_text_stay_fresh_host_strikes() {
    let trees = trees(&[("AC-P-001", "sleep 1", TrustedCwd::ProjectRoot)]);
    let probe = HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks);
    let mut contract = trees.contract();
    let at = Context::on_host_path(&contract).within(1, &["repo", "project"]);
    for text in ["sleep 1", "sleep 2"] {
        set_command(&mut contract, text);
        let result = failed_result("AC-P-001", "");
        let mut result = result;
        result.exit_code = None;
        let why = super::super::verdict::no_verdict(text, &result, &at).unwrap();
        assert!(settle(&probe, &trees.base, &contract, &result, &why, &at).is_none());
    }
}

#[test]
fn issue475_absolute_setup_paths_are_normalized_by_the_real_classifier() {
    let trees = trees(&[("AC-P-001", "cargo test", TrustedCwd::ProjectRoot)]);
    let mut contract = trees.contract();
    let at = Context::on_host_path(&contract).within(1, &["repo", "project"]);
    let mut signatures = Vec::new();
    for (text, output) in [
        (
            "cargo test --manifest-path /tmp/a/../b/",
            "/tmp/a/../b does not exist",
        ),
        ("cargo test --manifest-path /tmp/b", "/tmp/b does not exist"),
    ] {
        set_command(&mut contract, text);
        let result = failed_result("AC-P-001", output);
        let why = super::super::verdict::no_verdict(text, &result, &at).expect("no verdict");
        assert!(why.contains("setup failure of its site"), "{why}");
        signatures.push(setup_path_signature(&contract, "AC-P-001", &result, &why, &at).unwrap());
    }
    assert_eq!(signatures[0], signatures[1]);
    assert_eq!(normalize_path("/tmp/a/../b///"), "/tmp/b");
}
