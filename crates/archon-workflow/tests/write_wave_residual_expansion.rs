//! Issue-117 end to end: a residual gap an ACCEPTED verifier recorded is
//! accounted before acceptance, through the real prelude
//! (`resolveResiduals`), the production write wave (Git, forbidden paths,
//! task floors), the host's dispatch check and the final gate: a gap on a
//! file no task declares gets ONE bounded round granted exactly that file; a
//! gap on a declared file goes to its owner; a refused round stands and
//! blocks; a forged widening dispatches nothing; and a resume from the
//! deployed prelude replays every existing call, so only the new round runs.
#[path = "support/escalation_harness.rs"]
mod harness;
#[path = "support/write_wave_fixture.rs"]
mod support;
#[path = "support/residual_world.rs"]
mod world;

use archon_workflow::*;
use harness::{Answer, NEW_PRELUDE, Verdict, at_head, run};
use serde_json::{Value, json};
use world::*;

/// The prelude the live binary runs (4d3852d8c): it has no residual slot.
const DEPLOYED: &str = include_str!("fixtures/v3_primitives_4d3852d8c.js");

#[tokio::test]
async fn a_residual_on_an_unowned_file_is_expanded_fixed_and_resolved() {
    let host = host();
    host.verdicts(CROSS, vec![Verdict::AcceptWith(vec![HIGH_GAP])]);
    let result = run(&script(), NEW_PRELUDE, host.clone()).await;
    let calls = residual_calls(&host);
    assert_eq!(
        calls.len(),
        2,
        "one fix and one verifier: {:#?}",
        answers(&host)
    );
    assert!(ran(&host).iter().all(|id| !id.contains("-esc-")));
    assert_eq!(
        at_head(&host.f.repo, STORE),
        "// store: canonical instrument"
    );
    let fix = host.store.load_call_record(&calls[0]).unwrap().unwrap();
    assert_eq!(fix.dispatched_items[0].canonical_task_ids, ["TASK-A"]);
    let contract = &fix.call.options.extra["remediationContract"];
    assert_eq!(contract["residual"]["files"], json!([STORE]));
    assert_eq!(
        (contract["round"].clone(), contract["maxRounds"].clone()),
        (json!(1), json!(1))
    );
    let prompts = host.prompts.borrow();
    let (_, prompt) = prompts.iter().find(|(id, _)| *id == calls[0]).unwrap();
    assert!(
        prompt.contains(&format!("may ALSO write {STORE}")),
        "{prompt}"
    );
    assert!(
        prompt.contains("split('-').nth(2)"),
        "the gap, verbatim: {prompt}"
    );
    let verifier = host.store.load_call_record(&calls[1]).unwrap().unwrap();
    assert_eq!(verifier.dispatched_items[0].canonical_task_ids, ["TASK-A"]);
    assert!(
        answers(&host)
            .iter()
            .all(|(_, answer)| !matches!(answer, Answer::Refused(_))),
        "the host's own plan is never refused"
    );
    assert_eq!(result["residuals"][0]["files"], json!([STORE]), "{result}");
    let (status, why) = terminal(&host, &result);
    assert_eq!(status, WorkflowV2Status::Accepted, "{why}");
    assert!(why.contains("round") && why.contains("resolved"), "{why}");
}

#[tokio::test]
async fn a_refused_expansion_is_reported_blocks_and_is_never_asked_again() {
    let first = host();
    first.verdicts(CROSS, vec![Verdict::AcceptWith(vec![HIGH_GAP])]);
    // Refused over B's file: a regular round would buy the cross-owner
    // escalation; a residual round buys nothing.
    first.verdicts("TASK-A", vec![Verdict::Refuse(vec![B])]);
    let result = run(&script(), NEW_PRELUDE, first.clone()).await;
    assert_eq!(residual_calls(&first).len(), 2, "{:#?}", answers(&first));
    let (status, why) = terminal(&first, &result);
    assert_eq!(status, WorkflowV2Status::NeedsReview, "{why}");
    assert!(
        why.contains("gap-store-canonical-instrument") && why.contains("did not resolve it"),
        "{why}"
    );
    // A resume: the round is attempted, so nothing is dispatched again and
    // the gate still reads the refusal from the store.
    let second = next(first);
    let again = run(&script(), NEW_PRELUDE, second.clone()).await;
    assert!(ran(&second).is_empty(), "{:#?}", answers(&second));
    assert!(
        residual_calls(&second).is_empty(),
        "{:#?}",
        answers(&second)
    );
    let (status, why) = terminal(&second, &again);
    assert_eq!(status, WorkflowV2Status::NeedsReview, "{why}");
    assert!(why.contains("gap-store-canonical-instrument"), "{why}");
}

#[tokio::test]
async fn a_residual_on_an_owned_file_is_routed_to_its_owner() {
    let host = host();
    host.verdicts(
        CROSS,
        vec![Verdict::AcceptWith(vec![(
            "gap-b-stale",
            "medium",
            "crates/b/src/lib.rs:1 still carries the old provenance",
        )])],
    );
    let result = run(&script(), NEW_PRELUDE, host.clone()).await;
    let calls = residual_calls(&host);
    assert_eq!(calls.len(), 2, "{:#?}", answers(&host));
    let fix = host.store.load_call_record(&calls[0]).unwrap().unwrap();
    assert_eq!(fix.dispatched_items[0].canonical_task_ids, ["TASK-B"]);
    assert_eq!(
        fix.call.options.extra["remediationContract"]["residual"]["files"],
        json!([]),
        "an owned route opens nothing"
    );
    assert_eq!(at_head(&host.f.repo, B), "// b: refreshed");
    assert_eq!(at_head(&host.f.repo, STORE), "// store");
    let (status, why) = terminal(&host, &result);
    assert_eq!(status, WorkflowV2Status::Accepted, "{why}");
}

#[tokio::test]
async fn a_forged_widening_is_refused_and_dispatches_nothing() {
    // The script reads the host's plan itself, then asks for the planned
    // round with one more file, and for a round the host never planned.
    let forged = r#"
const view = await w.checkpoint('residual-gaps-probe', { residualGaps: true })
const entry = (view.residual_plan || view.data.residual_plan)[0]
await remediateFindings([{ id: 'wide', canonical_task_ids: entry.task_ids, severity: 'high', claim: 'x' }],
  { ...opts, maxRounds: 1, contestKey: entry.key, residual: { key: entry.key, files: [...entry.expansion_files, 'crates/b/src/lib.rs'] } })
await remediateFindings([{ id: 'own', canonical_task_ids: ['TASK-A'], severity: 'high', claim: 'x' }],
  { ...opts, maxRounds: 1, contestKey: 'residual-forged', residual: { key: 'residual-forged', files: ['crates/shared/src/store.rs'] } })
"#;
    let host = host();
    host.verdicts(CROSS, vec![Verdict::AcceptWith(vec![HIGH_GAP])]);
    let result = run(&script_forging(forged), NEW_PRELUDE, host.clone()).await;
    let refused: Vec<String> = answers(&host)
        .into_iter()
        .filter_map(|(id, answer)| match answer {
            Answer::Refused(why) => Some(format!("{id}: {why}")),
            _ => None,
        })
        .collect();
    assert!(
        refused
            .iter()
            .any(|why| why.contains("files are not exactly the plan's")),
        "{refused:#?}"
    );
    assert!(
        refused
            .iter()
            .any(|why| why.contains("no round of the host's plan is `residual-forged`")),
        "{refused:#?}"
    );
    assert!(
        ran(&host)
            .iter()
            .filter(|id| id.contains("residual-"))
            .count()
            == 2,
        "only the real round ran: {:#?}",
        answers(&host)
    );
    assert_eq!(at_head(&host.f.repo, B), "// b");
    let (status, why) = terminal(&host, &result);
    assert_eq!(status, WorkflowV2Status::Accepted, "{why}");
}

/// Session 1 is the deployed prelude: the review round runs and its verifier
/// accepts with the gap, and nothing accounts it. Session 2 is this prelude
/// over the same run: every call session 1 made replays under its own id and
/// input identity (so its prompt is unchanged), and the only work
/// dispatched is the new round.
#[tokio::test]
async fn a_resume_from_the_deployed_prelude_replays_every_call_and_runs_only_the_new_round() {
    let first = host();
    first.verdicts(CROSS, vec![Verdict::AcceptWith(vec![HIGH_GAP])]);
    let before = run(&script(), DEPLOYED, first.clone()).await;
    assert_eq!(
        before["residuals"],
        Value::Null,
        "the deployed prelude has no slot"
    );
    let recorded: Vec<(String, Answer)> = answers(&first);
    assert_eq!(recorded.len(), 2, "{recorded:#?}");
    assert!(recorded.iter().all(|(_, answer)| *answer == Answer::Ran));
    let (status, why) = terminal(&first, &before);
    assert_eq!(
        status,
        WorkflowV2Status::NeedsReview,
        "under this host, the gap no round carried blocks: {why}"
    );
    let second = next(first);
    let after = run(&script(), NEW_PRELUDE, second.clone()).await;
    let answered = answers(&second);
    for (id, _) in &recorded {
        assert!(
            answered.contains(&(id.clone(), Answer::Replayed)),
            "{id} replays: {answered:#?}"
        );
    }
    let new: Vec<&String> = answered
        .iter()
        .filter(|(_, answer)| *answer != Answer::Replayed)
        .map(|(id, _)| id)
        .collect();
    assert_eq!(new.len(), 2, "{answered:#?}");
    assert!(
        new.iter().all(|id| id.contains("residual-")),
        "{answered:#?}"
    );
    assert_eq!(
        at_head(&second.f.repo, STORE),
        "// store: canonical instrument"
    );
    let (status, why) = terminal(&second, &after);
    assert_eq!(status, WorkflowV2Status::Accepted, "{why}");
}

#[tokio::test]
async fn a_glob_resolves_to_exact_files_and_is_expanded_like_a_named_file() {
    let host = host();
    host.verdicts(
        CROSS,
        vec![Verdict::AcceptWith(vec![(
            "gap-lanes",
            "high",
            "the crates/shared/src/*.rs lanes carry the wrong instrument",
        )])],
    );
    let result = run(&script(), NEW_PRELUDE, host.clone()).await;
    let calls = residual_calls(&host);
    assert_eq!(calls.len(), 2, "{:#?}", answers(&host));
    let fix = host.store.load_call_record(&calls[0]).unwrap().unwrap();
    assert_eq!(
        fix.call.options.extra["remediationContract"]["residual"]["files"],
        json!([STORE])
    );
    assert_eq!(fix.dispatched_items[0].canonical_task_ids, ["TASK-A"]);
    let (status, why) = terminal(&host, &result);
    assert_eq!(status, WorkflowV2Status::Accepted, "{why}");
}

#[tokio::test]
async fn a_pathless_high_gap_is_resolved_by_an_accepting_adjudication() {
    let host = host();
    host.verdicts(
        CROSS,
        vec![Verdict::AcceptWith(vec![PATHLESS]), Verdict::Accept],
    );
    let result = run(&script(), NEW_PRELUDE, host.clone()).await;
    let calls = residual_calls(&host);
    assert_eq!(
        calls.len(),
        1,
        "one read-only verifier: {:#?}",
        answers(&host)
    );
    assert!(calls[0].ends_with("-adjudicate"), "{calls:?}");
    let record = host.store.load_call_record(&calls[0]).unwrap().unwrap();
    assert!(record.call.write_mode.is_none());
    assert_eq!(
        record.dispatched_items[0].canonical_task_ids,
        ["TASK-A", "TASK-B"],
        "the recording unit's tasks"
    );
    let prompt = record.call.options.task.clone().unwrap_or_default();
    assert!(
        prompt.contains("declared by no task in this run"),
        "{prompt}"
    );
    assert!(
        prompt.contains("every finding resolved"),
        "the recording summary: {prompt}"
    );
    let (status, why) = terminal(&host, &result);
    assert_eq!(status, WorkflowV2Status::Accepted, "{why}");
    // Once per gap: a resume asks nothing again.
    let second = next(host);
    run(&script(), NEW_PRELUDE, second.clone()).await;
    assert!(ran(&second).is_empty(), "{:#?}", answers(&second));
}

#[tokio::test]
async fn a_pathless_high_gap_the_adjudicator_records_again_blocks_by_name() {
    let host = host();
    host.verdicts(
        CROSS,
        vec![
            Verdict::AcceptWith(vec![PATHLESS]),
            Verdict::AcceptWith(vec![(
                "gap-roster-again",
                "high",
                "the lanes are still owned by no task",
            )]),
        ],
    );
    let result = run(&script(), NEW_PRELUDE, host.clone()).await;
    // The adjudicator's own new HIGH gap names no file either: the second
    // pass could only report it, so the third adjudicates it (Batch B: no
    // HIGH gap is left with no pass to plan it).
    let calls = residual_calls(&host);
    assert_eq!(calls.len(), 2, "{:#?}", answers(&host));
    assert!(calls[1].ends_with("p3-adjudicate"), "{calls:?}");
    let (status, why) = terminal(&host, &result);
    assert_eq!(status, WorkflowV2Status::NeedsReview, "{why}");
    assert!(
        why.contains("gap-roster") && why.contains("recorded high gap(s) again"),
        "{why}"
    );
    assert!(why.contains("resolved `gap-roster-again`"), "{why}");
}

/// A new, unrelated medium on the file the round fixed.
const STORE_TEST: (&str, &str, &str) = (
    "gap-store-test",
    "medium",
    "crates/shared/src/store.rs:120 lacks a test",
);

/// The round's verifier is asked, through the real prelude, for a
/// disposition of the gap it judges; one that reports it resolved keeps a
/// new medium on the same file a warning of its own, and one that does not
/// leaves the gate as strict as before: the targeted gap reopens and blocks.
#[tokio::test]
async fn a_resolved_disposition_keeps_a_new_gap_on_the_fixed_file_from_reopening_it() {
    for disposes in [true, false] {
        let host = host();
        host.verdicts(CROSS, vec![Verdict::AcceptWith(vec![HIGH_GAP])]);
        let verdict = if disposes {
            Verdict::AcceptDisposing(vec![STORE_TEST], vec![(HIGH_GAP.0, "resolved")])
        } else {
            Verdict::AcceptWith(vec![STORE_TEST])
        };
        host.verdicts("TASK-A", vec![verdict]);
        let result = run(&script(), NEW_PRELUDE, host.clone()).await;
        let calls = residual_calls(&host);
        assert_eq!(calls.len(), 2, "{:#?}", answers(&host));
        let verifier = host.store.load_call_record(&calls[1]).unwrap().unwrap();
        let prompt = verifier.call.options.task.clone().unwrap_or_default();
        assert!(
            prompt.contains("gap_dispositions") && prompt.contains(HIGH_GAP.0),
            "the verifier is asked: {prompt}"
        );
        let fix = host.store.load_call_record(&calls[0]).unwrap().unwrap();
        let asked_fix = fix.call.options.task.clone().unwrap_or_default();
        assert!(!asked_fix.contains("gap_dispositions"), "{asked_fix}");
        let (status, why) = terminal(&host, &result);
        if disposes {
            assert_eq!(status, WorkflowV2Status::Accepted, "{why}");
            assert!(
                why.contains("gap-store-test"),
                "the new medium warns: {why}"
            );
        } else {
            assert_eq!(status, WorkflowV2Status::NeedsReview, "{why}");
            assert!(why.contains(HIGH_GAP.0) && why.contains("again"), "{why}");
        }
    }
}
