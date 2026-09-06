#[path = "support/native_fixture.rs"]
mod support;
use archon_workflow::acceptance_scratch::observe_commands;
use archon_workflow::task_set_contract::{AcceptanceCheck, content_digest};
use support::fixture;

fn second(
    c: &mut archon_workflow::task_set_contract::AcceptanceContract,
    refs: &mut Vec<archon_workflow::acceptance_world::FrozenCommandRef>,
    cmd: &str,
) {
    let mut entry = c.acceptance[0].clone();
    entry.id = "AC-X-002".into();
    entry.check = AcceptanceCheck::Command {
        command: cmd.into(),
        cwd: archon_workflow::task_set_contract::TrustedCwd::ProjectRoot,
    };
    c.acceptance.push(entry);
    let mut reference = refs[0].clone();
    reference.acceptance_id = "AC-X-002".into();
    reference.command_digest = content_digest(cmd.as_bytes());
    refs.push(reference);
}

#[tokio::test]
async fn audit_ignores_untracked_build_and_concurrent_workflow_output() {
    let (t, p, commit, mut c, mut refs) = fixture("test -f input");
    std::fs::create_dir_all(p.repository.join("target/large")).unwrap();
    let file = std::fs::File::create(p.repository.join("target/large/blob")).unwrap();
    file.set_len(144 * 1024 * 1024 * 1024).unwrap(); // sparse: scope, not disk throughput
    std::os::unix::net::UnixListener::bind(p.repository.join("target/socket")).unwrap();
    std::fs::create_dir_all(p.project.join(".archon/workflows")).unwrap();
    let cmd = format!(
        "test -f data/value && printf progress > '{}'",
        p.project.join(".archon/workflows/progress").display()
    );
    if let AcceptanceCheck::Command { command, .. } = &mut c.acceptance[0].check {
        *command = cmd.clone();
    }
    refs[0].command_digest = content_digest(cmd.as_bytes());
    let out = observe_commands(&p, &commit, &c, "chain", &refs, &t.path().join("evidence"))
        .await
        .unwrap();
    assert!(
        out.passed(),
        "unrelated outputs invalidated input audit: {:?}",
        out.operational_errors
    );
    assert!(
        !out.before[&p.repository.display().to_string()]
            .keys()
            .any(|k| k.starts_with("target") || k.starts_with(".git"))
    );
}

#[tokio::test]
async fn later_check_cannot_use_earlier_input_mutations() {
    let (t, p, commit, mut c, mut refs) =
        fixture("test -f data/value && printf changed > data/value && touch data/new");
    second(
        &mut c,
        &mut refs,
        "grep -q changed data/value && test -f data/new",
    );
    let out = observe_commands(&p, &commit, &c, "chain", &refs, &t.path().join("evidence"))
        .await
        .unwrap();
    assert_eq!(out.checks[0].exit_code, Some(0));
    assert_ne!(
        out.checks[1].exit_code,
        Some(0),
        "second check relied on the first check's data"
    );
    assert!(out.check_evidence.iter().all(|e| e.input_reset));
}

#[tokio::test]
async fn quota_walks_are_coarse_while_cancellation_stays_responsive() {
    let (t, mut p, commit, c, refs) = fixture("sleep 3; test -f data/value");
    p.timeout_secs = 10;
    let out = observe_commands(&p, &commit, &c, "chain", &refs, &t.path().join("evidence"))
        .await
        .unwrap();
    assert!(out.passed());
    let value = serde_json::to_value(&out.checks[0]).unwrap();
    let count = value["quota_walk_count"]
        .as_u64()
        .expect("quota walk evidence absent");
    assert!(
        (1..=2).contains(&count),
        "{count} whole-tree walks in three seconds"
    );
}

#[tokio::test]
async fn nested_data_configuration_is_not_a_host_secret() {
    let (t, p, commit, c, refs) = fixture("grep -q dataset data/config.json");
    std::fs::write(p.project.join("data/config.json"), "{\"dataset\":true}").unwrap();
    let out = observe_commands(&p, &commit, &c, "chain", &refs, &t.path().join("evidence"))
        .await
        .unwrap();
    assert!(out.passed(), "{:?}", out.operational_errors);
}

#[tokio::test]
async fn ordinary_timeout_does_not_skip_independent_later_check() {
    let (t, mut p, commit, mut c, mut refs) = fixture("sleep 30; test -f input");
    p.timeout_secs = 1;
    second(&mut c, &mut refs, "grep -q before data/value");
    let out = observe_commands(&p, &commit, &c, "chain", &refs, &t.path().join("evidence"))
        .await
        .unwrap();
    assert!(out.checks[0].operational_error.is_some());
    assert_eq!(
        out.checks[1].exit_code,
        Some(0),
        "timeout skipped an independent check"
    );
}

#[tokio::test]
async fn operator_exclusions_apply_to_copy_and_audit() {
    let (t, mut p, commit, c, refs) = fixture("test -f data/value && test ! -e data/private");
    p.project_input_excludes = vec!["data/private".into()];
    std::fs::write(p.project.join("data/private"), "operator-excluded").unwrap();
    let out = observe_commands(&p, &commit, &c, "chain", &refs, &t.path().join("evidence"))
        .await
        .unwrap();
    assert!(out.passed());
    assert!(!out.before[&p.project.display().to_string()].contains_key("data/private"));
}
