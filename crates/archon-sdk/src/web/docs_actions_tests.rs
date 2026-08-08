use super::*;

fn permissive() -> EffectivePolicySummary {
    let mut policy = EffectivePolicySummary::default_safe();
    policy.web.allow_mutating_actions = true;
    policy.web.allow_document_deletion = true;
    policy
}

#[test]
fn deletion_is_denied_by_default() {
    let (allowed, reason) = deletion_allowed(&EffectivePolicySummary::default_safe());
    assert!(!allowed);
    assert!(reason.contains("allow_mutating_actions"), "{reason}");
}

#[test]
fn uploads_alone_do_not_permit_deletion() {
    // The whole reason this flag exists: an operator who enabled drag-and-drop
    // has not asked for a delete button on every row.
    let mut policy = EffectivePolicySummary::default_safe();
    policy.web.allow_mutating_actions = true;
    policy.web.allow_file_uploads = true;
    policy.subsystem.allow_file_uploads = true;

    let (allowed, reason) = deletion_allowed(&policy);
    assert!(!allowed);
    assert!(reason.contains("allow_document_deletion"), "{reason}");
}

#[test]
fn deletion_needs_both_the_global_gate_and_its_own_flag() {
    // Own flag without the global gate.
    let mut only_flag = EffectivePolicySummary::default_safe();
    only_flag.web.allow_document_deletion = true;
    assert!(!deletion_allowed(&only_flag).0);

    // Both.
    assert!(deletion_allowed(&permissive()).0);
}

#[test]
fn unknown_index_actions_are_rejected_by_name() {
    let paths = crate::web::WebRuntimePaths::default();
    let request = WebIndexControlRequest {
        action: "delete-everything".to_string(),
        job_id: Some("job-1".to_string()),
        limit: None,
    };
    let error = apply_index_control(&paths, &request).expect_err("should reject");
    assert!(
        error
            .to_string()
            .contains("unsupported index control action"),
        "{error}"
    );
}

#[test]
fn job_scoped_actions_require_a_job_id() {
    let paths = crate::web::WebRuntimePaths::default();
    for action in ["pause", "resume", "cancel"] {
        for job_id in [None, Some(String::new()), Some("   ".to_string())] {
            let request = WebIndexControlRequest {
                action: action.to_string(),
                job_id: job_id.clone(),
                limit: None,
            };
            let error = apply_index_control(&paths, &request)
                .expect_err("{action} without a job id should fail");
            assert!(
                error.to_string().contains("job id is required"),
                "action={action} error={error}"
            );
        }
    }
}
