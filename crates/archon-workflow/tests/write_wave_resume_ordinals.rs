//! Issue-122b end to end, through the real prelude, the production write
//! wave and the host's views:
//!
//! A session that finished one residual round and was stopped after the
//! next round's fix landed but before its verifier resumes with the finished
//! round SKIPPED -- and the cut round's fix still replays under the id it
//! was filed under, so only its verifier runs. Under the prelude at
//! 3a9cd5aec the skipped round advanced no ordinal, so the cut round's calls
//! were asked under ids nothing recorded (live, a fix whose landing could
//! not be replayed by its branch was then written again). A session recorded by the 3a9cd5aec or
//! the 7c9f11a6c prelude replays every call it recorded under the new one.
//!
//! Issue-122a (a stopped contest remediation) is proven in
//! `write_wave_contest_resolution`.
#[path = "support/escalation_harness.rs"]
mod harness;
#[path = "support/write_wave_fixture.rs"]
mod support;
#[path = "support/residual_world.rs"]
mod world;

use std::path::Path;
use std::rc::Rc;

use archon_workflow::*;
use harness::{Answer, Host, NEW_PRELUDE, Verdict, run};
use world::*;

const AT_3A9CD5AEC: &str = include_str!("fixtures/v3_primitives_3a9cd5aec.js");
const AT_7C9F11A6C: &str = include_str!("fixtures/v3_primitives_7c9f11a6c.js");

const B_GAP: (&str, &str, &str) = (
    "gap-b-lane",
    "high",
    "crates/b/src/lib.rs:1 drops the lane's version",
);

fn verdicts(host: &Host) {
    host.verdicts(CROSS, vec![Verdict::AcceptWith(vec![HIGH_GAP, B_GAP])]);
}

fn contract_key(record: &WorkflowV2CallRecord) -> Option<String> {
    record.call.options.extra.get("remediationContract")?["residual"]["key"]
        .as_str()
        .map(str::to_string)
}

/// Remove every record (and branch/stage state) whose id is in `ids`: the
/// session stopped before them.
fn forget(root: &Path, ids: &[String]) {
    for id in ids {
        for entry in std::fs::read_dir(root.join("results")).unwrap().flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with(&format!("{id}-")) && name.ends_with(".json") {
                std::fs::remove_file(entry.path()).unwrap();
            }
        }
        let _ = std::fs::remove_dir_all(root.join("branches").join(id));
        let _ = std::fs::remove_dir_all(
            root.parent()
                .unwrap()
                .join("write-coordination/stages")
                .join(id),
        );
    }
}

/// A first session under `prelude` that planned two residual rounds, stopped
/// after the second round's fix landed: its verifier and done checkpoint
/// (and anything after) never happened. Returns the host, the second
/// round's fix id and what the session recorded.
async fn stopped_after_second_fix(prelude: &str) -> (Rc<Host>, String, Vec<String>) {
    let first = host();
    verdicts(&first);
    run(&script(), prelude, first.clone()).await;
    let records = first.store.load_call_records().unwrap();
    let mut rounds: Vec<(String, WorkflowV2CallRecord)> = records
        .iter()
        .filter(|r| r.call.write_mode.is_some())
        .filter_map(|r| contract_key(r).map(|key| (key, r.clone())))
        .collect();
    rounds.sort_by(|a, b| a.1.started_at.cmp(&b.1.started_at));
    assert_eq!(rounds.len(), 2, "two residual rounds: {rounds:#?}");
    let (key, fix) = rounds[1].clone();
    let cut: Vec<String> = records
        .iter()
        .filter(|r| r.started_at > fix.started_at || contract_key(r).as_deref() == Some(&key))
        .filter(|r| r.call.id != fix.call.id)
        .map(|r| r.call.id.clone())
        .chain([format!("{key}-done")])
        .collect();
    assert!(
        cut.iter().any(|id| id.starts_with("verification-wave-")),
        "{cut:?}"
    );
    forget(first.store.root(), &cut);
    let recorded: Vec<String> = first
        .store
        .load_call_records()
        .unwrap()
        .into_iter()
        .map(|r| r.call.id)
        .collect();
    (next(first), fix.call.id, recorded)
}

#[tokio::test]
async fn a_skipped_round_keeps_the_cut_rounds_fix_under_its_id() {
    let (second, fix, recorded) = stopped_after_second_fix(NEW_PRELUDE).await;
    run(&script(), NEW_PRELUDE, second.clone()).await;
    let answered = answers(&second);
    let fix_answer = answered.iter().find(|(id, _)| *id == fix);
    assert_eq!(
        fix_answer.map(|(_, a)| a.clone()),
        Some(Answer::Replayed),
        "the landed fix replays: {answered:#?}"
    );
    let fresh = ran(&second);
    assert_eq!(fresh.len(), 1, "only the cut verifier runs: {answered:#?}");
    assert!(fresh[0].starts_with("verification-wave-"), "{fresh:?}");
    for (id, answer) in &answered {
        if recorded.contains(id) && id.starts_with("review-remediate-") {
            assert_eq!(*answer, Answer::Replayed, "{id}: {answered:#?}");
        }
    }
}

#[tokio::test]
async fn under_the_3a9cd5aec_prelude_the_cut_rounds_fix_is_filed_under_a_new_id() {
    let (second, fix, recorded) = stopped_after_second_fix(AT_3A9CD5AEC).await;
    run(&script(), AT_3A9CD5AEC, second.clone()).await;
    let answered = answers(&second);
    assert!(
        answered.iter().all(|(id, _)| *id != fix),
        "the recorded fix is never asked: {answered:#?}"
    );
    assert!(
        answered
            .iter()
            .any(|(id, _)| id.starts_with("review-remediate-") && !recorded.contains(id)),
        "the same round's fix is asked under an id nothing recorded: {answered:#?}"
    );
}

/// Byte identity: every call a stopped session recorded under an older
/// prelude that the new prelude asks again replays.
async fn replays_everything_recorded_by(prelude: &str) {
    let (second, fix, recorded) = stopped_after_second_fix(prelude).await;
    run(&script(), NEW_PRELUDE, second.clone()).await;
    let answered = answers(&second);
    for (id, answer) in &answered {
        if recorded.contains(id) && *answer != Answer::Checkpoint {
            assert_eq!(*answer, Answer::Replayed, "{id}: {answered:#?}");
        }
    }
    assert!(
        answered
            .iter()
            .any(|(id, a)| *id == fix && *a == Answer::Replayed),
        "{answered:#?}"
    );
}

#[tokio::test]
async fn a_session_recorded_by_the_3a9cd5aec_prelude_replays_under_the_new_one() {
    replays_everything_recorded_by(AT_3A9CD5AEC).await;
}

#[tokio::test]
async fn a_session_recorded_by_the_7c9f11a6c_prelude_replays_under_the_new_one() {
    replays_everything_recorded_by(AT_7C9F11A6C).await;
}
