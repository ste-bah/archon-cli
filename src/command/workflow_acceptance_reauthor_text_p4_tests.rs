#![allow(non_snake_case)]
use super::*;
use crate::command::workflow_task_set::reauthor::{AuthorScope, author};
use crate::command::workflow_task_set::republish::test_fixture::frozen_set_proven as frozen_set;

const S1: &str = "A path the check reads by its literal relative path, as an operand of a command that reads it (an existence test included), is a move-aside handle: the host may move it aside to prove the check can fail. A handle is not the deciding assertion; the test or implementation run whose status decides the check must still decide it.";

#[test]
fn P4_14_R4_reauthor_explains_handles_and_build_failure() {
    let set = frozen_set(&[("AC-X-001", "run", false)]);
    let scope =
        AuthorScope::for_task_set_without_cache(set.project.path(), &set.tasks, &set.prd).unwrap();
    let frozen = &set.contract().acceptance[0];
    for notes in [
        vec![],
        vec![format!(
            "check '{}' {}",
            frozen.id,
            crate::command::workflow_task_set::executability::CANNOT_FAIL
        )],
    ] {
        let prompt = author::author_prompt_with_prefetch(&scope, frozen, &notes, 1, &[], &[])
            .expect("read requirement texts");
        assert!(prompt.contains(S1));
        assert!(prompt.contains("a build failure at or naming a moved input also proves it, while any other build failure leaves the check unproven"));
        assert!(!prompt.contains("tree at unknown"));
        if !notes.is_empty() {
            assert!(
                prompt.starts_with("Make the check fail on the pre-implementation tree if it does not exercise what the work must add")
            );
        }
    }
}

#[test]
fn P4_20_prefetch_mines_only_check_text() {
    let set = frozen_set(&[("AC-X-001", "cat actual.txt", false)]);
    std::fs::write(set.project.path().join("actual.txt"), "real check input").unwrap();
    let mut frozen = set.contract().acceptance[0].clone();
    frozen.criterion = "A descriptive ids/versions.txt is not a file operand".into();
    let paths = author::prefetch_inputs(
        &frozen,
        &["inspect noise/from-feedback.txt".into()],
        set.project.path(),
        set.project.path(),
    );
    assert!(paths.contains(&"actual.txt".to_string()));
    assert!(paths.contains(&"noise/from-feedback.txt".to_string()));
    assert!(!paths.contains(&"ids/versions.txt".to_string()));
}

#[test]
fn named_inputs_keep_missing_nested_paths_and_exclude_missing_outside_paths() {
    let set = frozen_set(&[(
        "AC-X-001",
        "inspect noise/from-feedback.txt ../outside/missing.txt",
        false,
    )]);
    let paths = crate::command::workflow_task_set::executability::named_inputs(
        &set.contract().acceptance[0],
        set.project.path(),
        set.project.path(),
    );
    assert_eq!(paths, ["noise/from-feedback.txt"]);
}

#[test]
fn P4_22_every_applicable_reason_reaches_the_author() {
    use crate::command::workflow_gate::{GateFinding, GateId};
    let set = frozen_set(&[("AC-X-001", "run", false)]);
    let scope =
        AuthorScope::for_task_set_without_cache(set.project.path(), &set.tasks, &set.prd).unwrap();
    let finding = |id: &str, text: &str| {
        GateFinding::new(
            GateId::FreezeAcceptance,
            text,
            id,
            None,
            archon_workflow::RemediationScope::CandidateArtifact,
        )
    };
    let mut seeds = BTreeMap::from([("AC-X-001".into(), "existing crash reason".into())]);
    seed_findings(
        &mut seeds,
        &BTreeSet::from(["AC-X-001".into()]),
        &[
            finding("AC-X-001", "the caller's first gate reason"),
            finding("AC-X-001", "the caller's second gate reason"),
            finding("AC-X-002", "a sibling's reason"),
        ],
    );
    assert_eq!(
        seeds["AC-X-001"],
        "existing crash reason\nthe caller's first gate reason\nthe caller's second gate reason"
    );
    let prompt = author::author_prompt_with_prefetch(
        &scope,
        &set.contract().acceptance[0],
        &[seeds["AC-X-001"].clone()],
        1,
        &[],
        &[],
    )
    .expect("read requirement texts");
    assert!(prompt.contains("the caller's first gate reason"));
    assert!(prompt.contains("the caller's second gate reason"));
    assert!(!prompt.contains("a sibling's reason"));
    let caller = include_str!("workflow_task_set_findings.rs");
    assert!(
        caller.contains("super::reauthor::seed_findings(&mut seeds, &named, &prepared.findings)")
    );
}

#[test]
fn shared_move_aside_handle_sentence_matches_author_rule() {
    for source in [
        include_str!("workflow_acceptance_executability_prove_diagnostics.rs"),
        include_str!("workflow_acceptance_live_root.rs"),
        include_str!("workflow_decompose_v1_acceptance.js"),
    ] {
        assert!(source.contains(S1), "shared S1 is missing or differs");
    }
    assert!(include_str!("workflow_task_set_judge.rs").contains("A move-aside handle line is neither the deciding assertion nor a reason to refute; judge the deciding test."));
}

#[test]
fn p4_author_and_progress_node_suite() {
    let script =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/command/workflow_decompose_p4_test.cjs");
    let output = std::process::Command::new("node")
        .arg(script)
        .output()
        .expect("node must be available");
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
async fn explicit_id_repair_without_prior_seeds_shows_the_original_defect() {
    use crate::command::workflow_task_set::executability::ExecutabilityProbe;
    use crate::command::workflow_task_set::reauthor::{
        ReauthorGate, reauthor_partial_with_cache,
        test_client::{ScriptedAuthorJudge, command_entry},
    };
    struct Probe;
    #[async_trait::async_trait]
    impl ExecutabilityProbe for Probe {
        async fn script_defects(
            &self,
            contract: &AcceptanceContract,
            ids: &BTreeSet<String>,
        ) -> BTreeMap<String, String> {
            assert!(ids.contains("AC-X-001"));
            if serde_json::to_string(contract)
                .unwrap()
                .contains("broken-original")
            {
                BTreeMap::from([("AC-X-001".into(), "original script syntax defect".into())])
            } else {
                BTreeMap::new()
            }
        }
    }
    let set = frozen_set(&[("AC-X-001", "broken-original", true)]);
    let scope =
        AuthorScope::for_task_set_without_cache(set.project.path(), &set.tasks, &set.prd).unwrap();
    let client = ScriptedAuthorJudge::new(
        |entry, _| command_entry(entry, "test -f present"),
        |_, _| true,
    );
    let progress = reauthor_partial_with_cache(
        &client,
        &set.contract(),
        &BTreeSet::from(["AC-X-001".into()]),
        &scope,
        crate::command::workflow_task_set::judge::JUDGE_MODEL_ALIAS,
        &ReauthorGate {
            probe: &Probe,
            seeds: &BTreeMap::new(),
        },
        None,
    )
    .await
    .unwrap();
    assert!(progress.unproven.is_empty());
    let prompts = client.prompts.lock().unwrap();
    assert_eq!(prompts.len(), 1);
    assert!(prompts[0].contains("original script syntax defect"));
}
