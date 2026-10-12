#![cfg(test)]

use super::*;

mod successor_reuse_tests {

    use super::*;
    use archon_workflow::{
        PublishedArtifactReceipt, WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2Status,
    };

    fn record(status: WorkflowV2Status, invalidated: bool) -> WorkflowV2CallRecord {
        let call = WorkflowV2HostCall {
            id: "call".into(),
            method: WorkflowV2HostMethod::HostCommand,
            write_mode: None,
            options: Default::default(),
        };
        let mut result = archon_workflow::WorkflowV2Result::default();
        result.status = status;
        let mut record = WorkflowV2CallRecord::new("run", call, 1, "input".into(), result, vec![]);
        record.status = status;
        record.invalidated_by = invalidated.then(|| "later-call".into());
        record
    }

    fn entry(prior: &str, digest: &str, len: u64, destination: &str) -> PublishedArtifactReceipt {
        PublishedArtifactReceipt {
            relative_path: "artifact".into(),
            destination_path: destination.into(),
            byte_len: len,
            blake3: digest.into(),
            prior_blake3: Some(prior.into()),
        }
    }

    #[test]
    fn needs_review_successors_are_only_live_when_findings_are_allowed() {
        let record = record(WorkflowV2Status::NeedsReview, false);
        assert!(!record_status_allows_reuse(&record, false));
        assert!(record_status_allows_reuse(&record, true));
    }

    #[test]
    fn invalidated_successors_are_refused() {
        let record = record(WorkflowV2Status::Accepted, true);
        assert!(!record_status_allows_reuse(&record, false));
        assert!(!record_status_allows_reuse(&record, true));
    }

    #[test]
    fn receipt_chain_accepts_two_ordered_hops() {
        let mut times = std::collections::BTreeMap::from([("first".into(), "01".into())]);
        let middle = entry("first", "middle", 3, "/artifact");
        assert!(receipt_entry_extends(
            &middle,
            "/artifact",
            &times,
            "02",
            "live",
            4
        ));
        times.insert("middle".into(), "02".into());
        let last = entry("middle", "live", 4, "/artifact");
        assert!(receipt_entry_extends(
            &last,
            "/artifact",
            &times,
            "03",
            "live",
            4
        ));
    }

    #[test]
    fn receipt_chain_rejects_live_byte_length_mismatch() {
        let times = std::collections::BTreeMap::from([("first".into(), "01".into())]);
        let candidate = entry("first", "live", 5, "/artifact");
        assert!(!receipt_entry_extends(
            &candidate,
            "/artifact",
            &times,
            "02",
            "live",
            4
        ));
    }

    #[test]
    fn receipt_chain_rejects_non_declared_destination() {
        let times = std::collections::BTreeMap::from([("first".into(), "01".into())]);
        let candidate = entry("first", "next", 5, "/other");
        assert!(!receipt_entry_extends(
            &candidate,
            "/artifact",
            &times,
            "02",
            "live",
            4
        ));
    }

    #[test]
    fn receipt_chain_rejects_stale_timestamp() {
        let times = std::collections::BTreeMap::from([("first".into(), "02".into())]);
        let candidate = entry("first", "next", 5, "/artifact");
        assert!(!receipt_entry_extends(
            &candidate,
            "/artifact",
            &times,
            "01",
            "live",
            4
        ));
    }

    #[test]
    fn successor_with_stale_logic_is_not_reusable() {
        let root = tempfile::tempdir().unwrap();
        let project_root = root.path().join("project");
        let task_root = project_root.join("tasks");
        let prd_path = project_root.join("prd.md");
        std::fs::create_dir_all(&task_root).unwrap();
        std::fs::write(&prd_path, "# PRD\n").unwrap();
        let context = crate::command::workflow_host_command_catalog::HostCommandResolutionContext {
            program: std::path::PathBuf::from("/trusted/archon"),
            project_root,
            prd_digest: archon_workflow::task_set_contract::content_digest(
                &std::fs::read(&prd_path).unwrap(),
            ),
            prd_path,
            task_root,
            run_staging_root: root.path().join("staging"),
            frozen_task_id: None,
            frozen_task_file: None,
            freeze_provider_environment: Default::default(),
            acceptance_environment_allowlist: Vec::new(),
            gate_mode: archon_core::config::GateMode::Observe,
        };
        let executor = FixedHostCommandExecutor::new(
            crate::command::workflow_host_command_catalog::fixed_decomposition_catalog("rev-1")
                .unwrap(),
            context,
            root.path().join("run"),
        );
        let request =
            archon_workflow::HostCommandRequest::new("freeze-skeleton", Some("candidate".into()))
                .unwrap();
        let call = WorkflowV2HostCall {
            id: "successor".into(),
            method: WorkflowV2HostMethod::HostCommand,
            write_mode: None,
            options: archon_workflow::WorkflowV2HostOptions {
                host_command: Some(request),
                ..Default::default()
            },
        };
        let mut result = archon_workflow::WorkflowV2Result::accepted("landed");
        result.data = serde_json::json!({"logicVersion": 0});
        let record = WorkflowV2CallRecord::new("run", call, 1, "input".into(), result, vec![]);
        assert!(!executor.outcome_logic_holds_for(&record).unwrap());
    }

    fn successor_fixture(
        original_command: &str,
        successor_command: &str,
        declares_path: bool,
        stale_logic: bool,
    ) -> (
        tempfile::TempDir,
        FixedHostCommandExecutor,
        WorkflowV2CallRecord,
        archon_workflow::PublicationReceiptV1,
        String,
    ) {
        let root = tempfile::tempdir().unwrap();
        let project_root = root.path().join("project");
        let task_root = project_root.join("tasks");
        let prd_path = project_root.join("prd.md");
        std::fs::create_dir_all(&task_root).unwrap();
        std::fs::write(
            &prd_path,
            "# Requirements\n\n## Requirements\n- REQ-X-001: retain a published artifact.\n\n## Acceptance Criteria\n- AC-X-001: the artifact is retained.\n",
        )
        .unwrap();

        let mut catalog =
            crate::command::workflow_host_command_catalog::fixed_decomposition_catalog("rev-1")
                .unwrap();
        if !declares_path {
            catalog
                .capabilities
                .get_mut(successor_command)
                .unwrap()
                .declared_write_set
                .retain(|path| !path.ends_with("acceptance-pin.json"));
            catalog.recompute_digest().unwrap();
        }
        let context = crate::command::workflow_host_command_catalog::HostCommandResolutionContext {
            program: std::path::PathBuf::from("/trusted/archon"),
            project_root: project_root.clone(),
            prd_digest: archon_workflow::task_set_contract::content_digest(
                &std::fs::read(&prd_path).unwrap(),
            ),
            prd_path,
            task_root: task_root.clone(),
            run_staging_root: root.path().join("staging"),
            frozen_task_id: None,
            frozen_task_file: None,
            freeze_provider_environment: Default::default(),
            acceptance_environment_allowlist: Vec::new(),
            gate_mode: archon_core::config::GateMode::Observe,
        };
        crate::command::workflow_host_command_exec_tests::seed_frozen_chain(
            &context,
            &task_root.join("TASK-X-010.md"),
        );
        let pin_path =
            crate::command::workflow_task_set::acceptance_pin_path(&project_root, &task_root);
        let original_bytes = std::fs::read(&pin_path).unwrap();
        let mut updated_pin: archon_workflow::task_set_contract::AcceptancePin =
            serde_json::from_slice(&original_bytes).unwrap();
        updated_pin.freeze_event_id = "later-freeze".into();
        let live_bytes = serde_json::to_vec_pretty(&updated_pin).unwrap();
        std::fs::write(&pin_path, &live_bytes).unwrap();
        let run_root = root.path().join("run");
        let executor = FixedHostCommandExecutor::new(catalog, context, run_root.clone());
        let original_request =
            archon_workflow::HostCommandRequest::new(original_command, Some("original".into()))
                .unwrap();
        let original_call = WorkflowV2HostCall {
            id: "original-call".into(),
            method: WorkflowV2HostMethod::HostCommand,
            write_mode: None,
            options: archon_workflow::WorkflowV2HostOptions {
                host_command: Some(original_request),
                ..Default::default()
            },
        };
        let original = WorkflowV2CallRecord::new(
            "run",
            original_call,
            1,
            "original-input".into(),
            Default::default(),
            vec![],
        );
        let original_digest = archon_workflow::task_set_contract::content_digest(&original_bytes);
        let receipt = archon_workflow::PublicationReceiptV1 {
            schema_version: archon_workflow::PUBLICATION_RECEIPT_SCHEMA_VERSION,
            call_id: "original-call".into(),
            command_id: original_command.into(),
            entries: vec![entry(
                "prior",
                &original_digest,
                original_bytes.len() as u64,
                &pin_path.to_string_lossy(),
            )],
            committed_at: "2026-01-01T00:00:00Z".into(),
        };

        let successor_request =
            archon_workflow::HostCommandRequest::new(successor_command, Some("successor".into()))
                .unwrap();
        let identity = if pipeline_order(&executor.catalog, successor_command)
            > pipeline_order(&executor.catalog, original_command)
        {
            executor.call_identity(&successor_request).unwrap()
        } else {
            "earlier-successor".into()
        };
        let mut outcome = archon_workflow::HostCommandResult {
            exit_code: Some(0),
            stdout: String::new(),
            stderr: String::new(),
            stdout_bytes: 0,
            stderr_bytes: 0,
            timed_out: false,
            interrupted: false,
            stdout_truncated: false,
            stderr_truncated: false,
            gate_envelope: Some(archon_workflow::GateEnvelopeV1 {
                schema_version: archon_workflow::GATE_ENVELOPE_SCHEMA_VERSION,
                report: serde_json::json!({}),
                policy_findings: vec![],
                operational_error: None,
            }),
            publication_receipt: Some(archon_workflow::PublicationReceiptV1 {
                schema_version: archon_workflow::PUBLICATION_RECEIPT_SCHEMA_VERSION,
                call_id: identity.clone(),
                command_id: successor_command.into(),
                entries: vec![entry(
                    &original_digest,
                    &archon_workflow::task_set_contract::content_digest(&live_bytes),
                    live_bytes.len() as u64,
                    &pin_path.to_string_lossy(),
                )],
                committed_at: "2026-01-02T00:00:00Z".into(),
            }),
            subjects: vec![],
            postcondition: Some(archon_workflow::CommandPostconditionEvaluation {
                satisfied: true,
                summary: "published".into(),
            }),
        };
        let mut data = serde_json::to_value(&outcome).unwrap();
        let logic_version = executor.logic_version_for(successor_command).unwrap();
        data[crate::command::workflow_host_command_logic::LOGIC_VERSION_STAMP] =
            serde_json::json!(if stale_logic {
                logic_version.saturating_sub(1)
            } else {
                logic_version
            });
        if successor_command != "freeze-acceptance" {
            data[crate::command::workflow_host_command_logic::LOGIC_DIGEST_STAMP] =
                serde_json::json!(executor.logic_digest(&successor_request).unwrap());
            data[crate::command::workflow_host_command_logic::LOGIC_BUILD_STAMP] =
                serde_json::json!(executor.logic_build(&successor_request).unwrap());
        }
        outcome.publication_receipt.as_mut().unwrap().call_id = identity.clone();
        data["publicationReceipt"] =
            serde_json::to_value(outcome.publication_receipt.unwrap()).unwrap();
        let mut result = archon_workflow::WorkflowV2Result::accepted("published");
        result.data = data;
        let successor = WorkflowV2CallRecord::new(
            "run",
            WorkflowV2HostCall {
                id: identity,
                method: WorkflowV2HostMethod::HostCommand,
                write_mode: None,
                options: archon_workflow::WorkflowV2HostOptions {
                    host_command: Some(successor_request),
                    ..Default::default()
                },
            },
            1,
            "successor-input".into(),
            result,
            vec![],
        );
        archon_workflow::WorkflowV2ResultStore::new(run_root.join("v2"))
            .save_call_record(&successor)
            .unwrap();
        (
            root,
            executor,
            original,
            receipt,
            pin_path.to_string_lossy().into_owned(),
        )
    }

    #[test]
    fn later_declared_successor_keeps_original_receipt_live() {
        let (_root, executor, original, receipt, _) =
            successor_fixture("freeze-acceptance", "freeze-skeleton", true, false);
        assert!(
            executor
                .receipt_matches_live_with_successors(&original, Some(&receipt), false)
                .unwrap()
        );
    }

    #[test]
    fn successor_with_stale_logic_version_is_refused_by_receipt_chain() {
        let (_root, executor, original, receipt, _) =
            successor_fixture("freeze-acceptance", "freeze-skeleton", true, true);
        assert!(
            !executor
                .receipt_matches_live_with_successors(&original, Some(&receipt), false)
                .unwrap()
        );
    }

    #[test]
    fn successor_that_does_not_declare_the_path_is_refused() {
        let (_root, executor, original, receipt, _) =
            successor_fixture("freeze-acceptance", "freeze-skeleton", false, false);
        assert!(
            !executor
                .receipt_matches_live_with_successors(&original, Some(&receipt), false)
                .unwrap()
        );
    }

    #[test]
    fn successor_earlier_in_pipeline_is_refused() {
        let (_root, executor, original, receipt, _) =
            successor_fixture("freeze-skeleton", "freeze-acceptance", true, false);
        assert!(
            !executor
                .receipt_matches_live_with_successors(&original, Some(&receipt), false)
                .unwrap()
        );
    }

    #[test]
    fn successor_order_uses_declared_pipeline_order() {
        let catalog =
            crate::command::workflow_host_command_catalog::fixed_decomposition_catalog("rev-1")
                .unwrap();
        assert!(
            pipeline_order(&catalog, "freeze-skeleton")
                < pipeline_order(&catalog, "land-task-body")
        );
        assert!(
            pipeline_order(&catalog, "task-set-lint")
                < pipeline_order(&catalog, "requirements-trace")
        );
        assert_eq!(
            pipeline_order(&catalog, "verify-frozen-acceptance"),
            Some(0)
        );
        assert_eq!(pipeline_order(&catalog, "verify-frozen-skeleton"), Some(1));
    }
}
