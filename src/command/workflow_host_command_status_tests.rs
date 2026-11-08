use super::*;

#[path = "workflow_decompose_origin_stamp_tests.rs"]
mod origin_stamp;
#[test]
fn frozen_status_is_stamped_only_when_present() {
    let candidate = "```yaml\ntask_id: TASK-X-010\nstatus: ready\n```\n";
    let mut frozen = frozen_with_owned_fields();
    for status in [None, Some("done"), Some("ready")] {
        frozen.status = status.map(str::to_string);
        let (body, notes) = super::super::workflow_host_command_binding::stamp_frozen_frontmatter(
            candidate, &frozen,
        )
        .unwrap();
        match status {
            Some(status) => assert!(body.contains(&format!("status: {status}"))),
            None => assert!(body.contains("status: ready")),
        }
        assert_eq!(
            notes.iter().any(|note| note == "status"),
            status == Some("done")
        );
    }
    frozen.status = None;
    let (body, notes) = super::super::workflow_host_command_binding::stamp_frozen_frontmatter(
        "```yaml\ntask_id: TASK-X-010\n```\n",
        &frozen,
    )
    .unwrap();
    assert!(!body.contains("status:"));
    assert!(!notes.iter().any(|note| note == "status"));
    frozen.status = Some("done".into());
    let (body, notes) = super::super::workflow_host_command_binding::stamp_frozen_frontmatter(
        "```yaml\ntask_id: TASK-X-010\n```\n",
        &frozen,
    )
    .unwrap();
    assert!(body.contains("status: done"));
    assert!(notes.iter().any(|note| note == "status"));
}

#[test]
fn stamp_refuses_only_authored_completion_and_preserves_other_values() {
    use super::super::workflow_host_command_binding::stamp_frozen_frontmatter;
    for status in ["done", "complete", "completed", "verified", "  DONE "] {
        let candidate = format!("```yaml\ntask_id: TASK-X-010\nstatus: '{status}'\n```\n");
        let error = stamp_frozen_frontmatter(&candidate, &frozen_with_owned_fields()).unwrap_err();
        assert!(
            matches!(error, archon_workflow::WorkflowError::SpecInvalid(ref text)
            if text == "authored status claims completion; completion is host-owned until proven; declare status: ready")
        );
    }
    for status in [
        "ready",
        "blocked",
        "blocekd",
        "1",
        "true",
        "[done]",
        "{value: done}",
        "null",
    ] {
        let candidate = format!("```yaml\ntask_id: TASK-X-010\nstatus: {status}\n```\n");
        let (body, notes) =
            stamp_frozen_frontmatter(&candidate, &frozen_with_owned_fields()).unwrap();
        let (origin, origin_notes) =
            origin_stamp::stamp_frozen_frontmatter(&candidate, &frozen_with_owned_fields())
                .unwrap();
        assert_eq!(body.as_bytes(), origin.as_bytes());
        assert_eq!(notes, origin_notes);
        assert!(origin_stamp::HOST_STAMPED_FIELDS.contains(&"implements"));
        let metadata = |text: &str| {
            serde_yaml_ng::from_str::<serde_json::Value>(
                text.split("```yaml")
                    .nth(1)
                    .unwrap()
                    .split("```")
                    .next()
                    .unwrap(),
            )
            .unwrap()
        };
        assert_eq!(metadata(&body)["status"], metadata(&candidate)["status"]);
        assert!(!notes.iter().any(|note| note == "status"));
    }
}

#[tokio::test]
async fn authored_completion_and_missing_status_return_body_refusals() {
    for status in ["done", "verified"] {
        let candidate = format!(
            "```yaml\ntask_id: TASK-X-010\ntitle: T\ncomplexity: low\nstatus: {status}\ndepends_on: []\nblocks: []\nimplements: []\nrequired_env_keys: []\nrequired_tools: []\ndeliverable_contracts: []\n```\n"
        );
        let finding = refusal(&candidate).await;
        assert_eq!(finding.remediation_scope, RemediationScope::Body);
        assert_eq!(
            finding.text,
            "candidate TASK body for TASK-X-010 is refused: authored status claims completion; completion is host-owned until proven; declare status: ready; return the whole task file"
        );
        assert!(!finding.text.contains(&format!("status: {status}")));
    }
    let finding = refusal("```yaml\ntask_id: TASK-X-010\ntitle: T\ncomplexity: low\ndepends_on: []\nblocks: []\nimplements: []\nrequired_env_keys: []\nrequired_tools: []\ndeliverable_contracts: []\n```\n").await;
    assert_eq!(finding.remediation_scope, RemediationScope::Body);
    assert!(
        finding.text.contains("missing required key(s): status"),
        "{}",
        finding.text
    );
}
