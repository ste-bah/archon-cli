use archon_workflow::requirement_state::{RequirementDecision, RequirementState, task_status};
use archon_workflow::task_skeleton::TaskSkeleton;
use archon_workflow::v2::requirement_classes::RequirementClassification;
use std::collections::BTreeSet;
pub(super) fn extend(
    skeleton: &mut TaskSkeleton,
    classes: &RequirementClassification,
    added: &BTreeSet<String>,
) {
    if added.is_empty() {
        return;
    }
    for task in &mut skeleton.tasks {
        if !task.implements.iter().any(|id| added.contains(id)) {
            continue;
        }
        let legacy = task.requirement_states.is_empty();
        for id in &task.implements {
            if legacy || added.contains(id) {
                let reason = classes
                    .entries
                    .iter()
                    .find(|e| e.task_id == task.task_id && &e.id == id)
                    .and_then(|e| match e.class {
                        archon_workflow::v2::requirement_classes::RequirementClass::Missing => {
                            e.reason.as_deref()
                        }
                        archon_workflow::v2::requirement_classes::RequirementClass::NoCheck => {
                            Some("no_check")
                        }
                        _ => None,
                    })
                    .unwrap_or("proof_pending");
                let state = if !added.contains(id) && reason == "proof_pending" {
                    RequirementState::Undecided
                } else {
                    RequirementState::Missing
                };
                task.requirement_states
                    .insert(id.clone(), RequirementDecision::new(state, reason));
            }
        }
        task.status = task_status(&task.requirement_states);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn skeleton(legacy: bool) -> TaskSkeleton {
        let mut skeleton: TaskSkeleton = serde_json::from_value(serde_json::json!({"schema_version":1,"acceptance_digest":"anchor","tasks":[{"task_id":"TASK-X-001","file_name":"TASK-X-001.md","implements":["old","new"],"status":"done","requirement_states":{"old":{"state":"done","reason":"breakage_kill"}},"proven_done":[{"id":"old","kind":"breakage","check_id":"check","check_digest":"digest","base_commit":"base","mutant_id":"m","body_gut_mutant_id":"gut","patch_digest":"patch","patch":"evidence"}]}]})).unwrap();
        if legacy {
            skeleton.tasks[0].status = None;
            skeleton.tasks[0].requirement_states.clear();
            skeleton.tasks[0].proven_done.clear();
        }
        skeleton
    }
    fn classes(new_class: &str, reason: Option<&str>) -> RequirementClassification {
        let entry = |id, class, reason: Option<&str>| serde_json::json!({"task_id":"TASK-X-001","id":id,"class":class,"reason":reason,"fallback":null,"checks":{},"deliverables":[],"text_item":false,"passing_check_without_deliverable":false});
        serde_json::from_value(serde_json::json!({"schema_version":2,"baseline_commit":"base","acceptance_digest":"anchor","baseline_results_digest":"results","error":null,"entries":[entry("old","missing",Some("all_checks_fail_on_base")),entry("new",new_class,reason)]})).unwrap()
    }
    #[test]
    fn republish_states_keep_existing_proofs_and_clear_done_for_new_missing_work() {
        let mut skeleton = skeleton(false);
        let before = skeleton.tasks[0].proven_done.clone();
        let old = skeleton.tasks[0].requirement_states["old"].clone();
        extend(
            &mut skeleton,
            &classes("no_check", None),
            &["new".into()].into(),
        );
        assert_eq!(skeleton.tasks[0].proven_done, before);
        assert_eq!(skeleton.tasks[0].requirement_states["old"], old);
        assert_eq!(
            skeleton.tasks[0].requirement_states["new"].reason,
            "no_check"
        );
        assert!(skeleton.tasks[0].status.is_none());
        archon_workflow::task_skeleton::validate_skeleton(&skeleton, "anchor").unwrap();
    }
    #[test]
    fn republish_states_use_observed_failure_reason_for_new_id() {
        let mut skeleton = skeleton(false);
        extend(
            &mut skeleton,
            &classes("missing", Some("all_checks_fail_on_base")),
            &["new".into()].into(),
        );
        assert_eq!(
            skeleton.tasks[0].requirement_states["new"].reason,
            "all_checks_fail_on_base"
        );
        assert!(skeleton.tasks[0].status.is_none());
    }
    #[test]
    fn republish_states_populate_legacy_keys_and_preserve_unextended_records() {
        let mut skeleton = skeleton(true);
        let before = skeleton.clone();
        extend(&mut skeleton, &classes("no_check", None), &BTreeSet::new());
        assert_eq!(skeleton, before);
        extend(
            &mut skeleton,
            &classes("no_check", None),
            &["new".into()].into(),
        );
        assert_eq!(skeleton.tasks[0].requirement_states.len(), 2);
        assert_eq!(
            skeleton.tasks[0].requirement_states["old"].reason,
            "all_checks_fail_on_base"
        );
        assert_eq!(
            skeleton.tasks[0].requirement_states["new"].state,
            RequirementState::Missing
        );
        archon_workflow::task_skeleton::validate_skeleton(&skeleton, "anchor").unwrap();
    }
    #[test]
    fn republish_candidate_done_new_id_changes_only_its_owner() {
        for legacy in [false, true] {
            let mut s = skeleton(legacy);
            let mut other = s.tasks[0].clone();
            other.task_id = "TASK-X-002".into();
            other.file_name = "TASK-X-002.md".into();
            other.implements.retain(|id| id != "new");
            let before = other.clone();
            s.tasks.push(other);
            extend(
                &mut s,
                &classes("candidate_done", None),
                &["new".into()].into(),
            );
            assert_eq!(s.tasks[1], before);
            assert_eq!(
                s.tasks[0].requirement_states["new"].state,
                RequirementState::Missing
            );
            assert_eq!(s.tasks[0].requirement_states["new"].reason, "proof_pending");
            assert_eq!(s.tasks[0].status, None);
            if !legacy {
                assert_eq!(s.tasks[1].status.as_deref(), Some("done"));
            }
        }
    }
}
