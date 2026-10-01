//! Batch O2 (REM-17): a residual claim carries every text whole, and a round
//! an earlier binary dispatched under the cut text keeps that text.

use serde_json::json;

use super::super::tests::*;
use super::super::*;
use super::Wording;
use crate::v2::WorkflowV2Status;

/// A world whose one accepted verifier recorded a gap and a summary far
/// longer than any earlier cut.
fn long_world() -> (World, String, String) {
    let w = world();
    let description = format!(
        "crates/b/src/lib.rs::ingest is wrong: {} END-OF-DESCRIPTION",
        "the lane reads the wrong segment; ".repeat(60)
    );
    let summary = format!(
        "{} END-OF-SUMMARY",
        "every declared test passes and the gap stands; ".repeat(40)
    );
    let mut verdict = verdict(
        "verification-wave-review-verify-task-a-1-2",
        &["TASK-A"],
        &[("gap-b", "medium", &description)],
    );
    verdict.result.summary = summary.clone();
    w.save(&verdict);
    (w, description, summary)
}

fn fix_call(round: &PlannedRound, claim: &str) -> WorkflowV2CallRecord {
    let tasks: Vec<&str> = round.tasks.iter().map(String::as_str).collect();
    let files: Vec<&str> = round.files.iter().map(String::as_str).collect();
    let contract = contract(
        "remediate",
        &tasks,
        json!({"residual": {"key": round.key, "files": files}, "contest": round.key}),
    );
    let mut call = call("review-remediate-residual-9", contract, true);
    // The prelude quotes the claim inside a finding, as it files a round.
    let finding = json!([{"id": round.key, "claim": claim}]);
    call.options.task = Some(format!("Findings (verbatim):\n{finding}"));
    record(call, WorkflowV2Status::Accepted, &tasks, &[])
}

#[test]
fn a_round_claim_carries_every_gap_and_summary_whole() {
    let (w, description, summary) = long_world();
    let plan = w.plan();
    assert_eq!(plan.rounds.len(), 1, "{:?}", plan.reported);
    let round = &plan.rounds[0];
    let view = round_view(round, &w.store);
    let claim = view["claim"].as_str().unwrap();
    assert!(description.chars().count() > 800 && summary.chars().count() > 600);
    // The claim quotes the gaps and summaries as JSON: both whole.
    assert!(
        claim.contains(&json!(description).to_string()[1..]),
        "{claim}"
    );
    assert!(claim.contains("END-OF-SUMMARY"), "{claim}");
    assert_eq!(view["findings"][0]["description"], json!(description));
    assert_eq!(claim, round_claim(round));
}

#[test]
fn a_round_dispatched_under_the_cut_text_keeps_it_and_any_other_gets_the_whole_text() {
    let (w, description, _) = long_world();
    let round = w.plan().rounds[0].clone();
    let legacy = super::super::view::round_claim_worded(&round, true, Wording::Legacy);
    let whole = round_claim(&round);
    let cut: String = description.chars().take(800).collect();
    assert!(legacy.contains(&format!("{cut}...")) && !legacy.contains("END-OF-DESCRIPTION"));
    assert_ne!(legacy, whole);
    // A call recorded under the whole text: the whole text stands.
    w.store.save_call_record(&fix_call(&round, &whole)).unwrap();
    assert_eq!(round_view(&round, &w.store)["claim"], json!(whole));
    // A call an earlier binary recorded under the cut text: the resumed
    // view rebuilds exactly that input, so its answer replays.
    w.store
        .save_call_record(&fix_call(&round, &legacy))
        .unwrap();
    assert_eq!(round_view(&round, &w.store)["claim"], json!(legacy));
    // The plan is unchanged by the round's own calls, and its key with it.
    assert_eq!(w.plan().rounds[0].key, round.key);
}

#[test]
fn a_refused_review_verdict_is_carried_whole_and_cut_only_in_its_legacy_form() {
    let long = "x".repeat(5_000);
    let evidence: Vec<_> = (0..8)
        .map(|i| json!({"summary": format!("{i}{}", "e".repeat(700)), "source": null}))
        .collect();
    let refusal = json!({"call_id": "v", "summary": long, "blocker_evidence": evidence,
        "review_prompt": long});
    assert_eq!(Wording::Whole.refusal(&refusal), refusal);
    let cut = Wording::Legacy.refusal(&refusal);
    assert_eq!(cut["summary"].as_str().unwrap().chars().count(), 4_003);
    assert_eq!(
        cut["review_prompt"].as_str().unwrap().chars().count(),
        4_003
    );
    let excerpts = cut["blocker_evidence"].as_array().unwrap();
    assert_eq!(excerpts.len(), 6);
    assert!(
        excerpts
            .iter()
            .all(|e| e["summary"].as_str().unwrap().len() == 603)
    );
    // A retried round's judgment was never cut, in any form.
    let judgment = json!({"round": "r", "summary": long});
    assert_eq!(Wording::Legacy.refusal(&judgment), judgment);
}

/// m5: the host's dispatch check holds a round's prompt to every gap's
/// WHOLE description -- not its opening words -- unless the round was
/// dispatched under the cut text, which then stands for it.
#[test]
fn the_dispatch_check_requires_each_gaps_whole_description() {
    let (w, description, _) = long_world();
    let round = w.plan().rounds[0].clone();
    let prompt = |text: &str| {
        let finding = json!([{"id": round.key, "claim": json!([{
            "id": round.residuals[0].id, "description": text}]).to_string()}]);
        format!("Findings (verbatim):\n{finding}")
    };
    let check = |text: &str, cut_ok: bool| {
        super::super::dispatch::unquoted_in(&prompt(text), &round, |_| cut_ok)
    };
    assert_eq!(check(&description, false), None);
    let opening: String = description.chars().take(300).collect();
    assert!(check(&opening, false).is_some_and(|missing| missing.starts_with("text of gap")));
    let cut: String = description.chars().take(800).collect();
    assert!(
        check(&cut, false).is_some(),
        "the cut text alone is not the gap"
    );
    assert_eq!(
        check(&cut, true),
        None,
        "a round dispatched under it keeps it"
    );
}
