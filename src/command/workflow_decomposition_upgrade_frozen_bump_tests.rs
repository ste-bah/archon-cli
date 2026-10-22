//! A freeze-acceptance logic bump never re-freezes a contract that is
//! already frozen and accepted: a resume verifies it (verify-frozen-acceptance)
//! and never asks freeze-acceptance again. The run here has the shape of a
//! long-lived observe-mode run: an accepted acceptance freeze judged by the
//! previous logic version, an accepted skeleton freeze whose version did not
//! move (it rewrote the shared acceptance pin after the acceptance freeze),
//! and a later acceptance attempt that was interrupted before any verdict.

use super::*;

fn capabilities(output: &str) -> Vec<&str> {
    output
        .lines()
        .filter_map(|line| line.split(" capability=").nth(1))
        .filter_map(|tail| tail.split(' ').next())
        .collect()
}

#[tokio::test]
async fn a_freeze_acceptance_bump_resumes_a_frozen_accepted_contract_by_verify_only() {
    let (project, store, run_id, _) = chained_run_behind(1, 0, None).await;
    let state: FixedDecompositionStateV1 =
        read_json(&store.run_dir(&run_id).join(FIXED_DECOMPOSITION_STATE_PATH));
    let task_root = Path::new(&state.identity.task_root_identity);
    use archon_workflow::task_set_contract::{
        ACCEPTANCE_CONTRACT_FILE as CONTRACT, ACCEPTANCE_LOCK_FILE as LOCK,
    };
    let frozen = [CONTRACT, LOCK].map(|name| std::fs::read(task_root.join(name)).unwrap());
    let output = dry_run(&project, &run_id).await;
    let current = crate::command::workflow_host_command_logic::versions()["freeze-acceptance"];
    assert!(
        output.contains(&format!(
            "frozen_chain_override acceptance=verify reason=judged attempt acceptance-judged gate logic changed; re-run the phase under current logic (logicVersion recorded={} current={current}",
            current - 1
        )),
        "{output}"
    );
    let called = capabilities(&output);
    assert_eq!(
        called.first().copied(),
        Some("verify-frozen-acceptance"),
        "{output}"
    );
    assert!(
        !called.contains(&"freeze-acceptance"),
        "an accepted frozen contract is never frozen again: {output}"
    );
    assert_eq!(
        frozen,
        [CONTRACT, LOCK].map(|name| std::fs::read(task_root.join(name)).unwrap()),
        "the dry run changed the frozen contract"
    );
}

#[tokio::test]
async fn the_same_logic_replays_the_recorded_freeze_and_never_re_executes_it() {
    // No bump: the recorded accepted freeze answers again (reuse), so the
    // resume does not re-run it either.
    let (project, _store, run_id, _) = chained_run_behind(0, 0, None).await;
    let output = dry_run(&project, &run_id).await;
    assert!(
        output.contains("frozen_chain_override acceptance=keep reason=judged attempt acceptance-judged replays under this build's logic"),
        "{output}"
    );
    assert!(
        !capabilities(&output).contains(&"verify-frozen-acceptance"),
        "{output}"
    );
}
