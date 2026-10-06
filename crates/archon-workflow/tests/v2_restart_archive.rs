//! Issue-254: the per-call archive keeps every record a resume or a reuse
//! needs. After a restart-task, an unrelated call's last accepted record is
//! still answered from its archive, and a new accepted attempt of the
//! restarted call is reused -- also from the archive, once a later attempt
//! was interrupted. Every check reads the files back.

#[path = "support/restart_run.rs"]
mod restart_run;

use archon_workflow::v2::restart::restart_generated_v2_task;
use archon_workflow::{WorkflowError, WorkflowV2CallRecord, WorkflowV2ResultStore};
use restart_run::{accepted, agent_call, generated_run, interrupted, slot, v2_store};

const T3: &str = "2026-10-03T05:16:09+00:00";
const T4: &str = "2026-10-03T06:00:00+00:00";

fn at(mut record: WorkflowV2CallRecord, attempt: u32, when: &str) -> WorkflowV2CallRecord {
    record.attempt = attempt;
    record.started_at = when.to_string();
    record.finished_at = when.to_string();
    record
}

fn history_candidate(v2: &WorkflowV2ResultStore, id: &str) -> Option<WorkflowV2CallRecord> {
    v2.call_record_for_reuse(&agent_call(id), &format!("in-{id}"))
        .unwrap()
        .filter(|candidate| candidate.from_history)
        .map(|candidate| candidate.record)
}

#[test]
fn reuse_after_a_restart_still_reads_the_archive() {
    let temp = tempfile::tempdir().unwrap();
    let (store, run) = generated_run(&temp, &["author-a", "author-c"]);
    let v2 = v2_store(&store, &run);
    for (id, task) in [("author-a", "T-A"), ("author-c", "T-C")] {
        v2.save_call_record(&accepted(id, task)).unwrap();
        v2.save_call_record(&interrupted(id)).unwrap();
    }

    restart_generated_v2_task(&store, &run, "T-A")
        .unwrap()
        .unwrap();

    // The unrelated call: its accepted record is answered from its archive
    // and goes back into the slot.
    let kept = history_candidate(&v2, "author-c").expect("author-c still reusable");
    assert_eq!(kept, accepted("author-c", "T-C"));
    // The prior session may read history, but cannot restore it into a
    // successor's slot. A newly opened session keeps unrelated reuse intact.
    assert!(matches!(
        v2.restore_call_record(&kept),
        Err(WorkflowError::ControlCancelled(_))
    ));
    assert_eq!(slot(&v2, "author-c").unwrap(), interrupted("author-c"));
    let v2 = v2_store(&store, &run);
    v2.restore_call_record(&kept).unwrap();
    assert_eq!(slot(&v2, "author-c").unwrap(), kept);

    // The restarted call never answers from before the restart ...
    assert!(history_candidate(&v2, "author-a").is_none());
    assert_eq!(v2.next_attempt("author-a").unwrap(), 3);
    // ... but its new accepted attempt is reused from the slot,
    let fresh = at(accepted("author-a", "T-A"), 3, T3);
    v2.save_call_record(&fresh).unwrap();
    let candidate = v2
        .call_record_for_reuse(&agent_call("author-a"), "in-author-a")
        .unwrap()
        .unwrap();
    assert!(!candidate.from_history);
    assert_eq!(candidate.record, fresh);
    // and from the archive once a later attempt took the slot and was
    // interrupted.
    v2.save_call_record(&at(interrupted("author-a"), 4, T4))
        .unwrap();
    assert_eq!(history_candidate(&v2, "author-a"), Some(fresh));
    assert_eq!(v2.next_attempt("author-a").unwrap(), 5);
}
