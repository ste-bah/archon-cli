//! Issue 375: a host command a pause stopped mid-flight answered nothing. Two
//! sibling candidates' attempts that both report no subject key alike, so the
//! earlier looks superseded by the later; a resume must still run each
//! interrupted call again, never replay its stub.

use super::*;

const TWO_LANDINGS_SCRIPT: &str = r#"
async function workflow(w) {
  const first = await w.hostCommand("land-candidate", { stdin: "candidate one" });
  const second = await w.hostCommand("land-candidate", { stdin: "candidate two" });
  return { first: first.interrupted === true, second: second.interrupted === true };
}
"#;

/// Refuses every candidate: no subject, no publication receipt.
struct RefusingHost {
    runs: AtomicUsize,
}

#[async_trait::async_trait]
impl crate::command::workflow_host_command_exec::WorkflowHostCommandExecutor for RefusingHost {
    fn call_identity(
        &self,
        request: &archon_workflow::HostCommandRequest,
    ) -> archon_workflow::WorkflowResult<String> {
        Ok(format!(
            "refuse-{}-{}",
            request.command_id,
            archon_workflow::task_set_contract::content_digest(
                request.stdin.as_deref().unwrap_or_default().as_bytes()
            )
        ))
    }

    fn record_is_reusable(
        &self,
        _record: &archon_workflow::WorkflowV2CallRecord,
    ) -> archon_workflow::WorkflowResult<bool> {
        Ok(true)
    }

    async fn execute(
        &self,
        _request: archon_workflow::HostCommandRequest,
        _expected_generation: Option<u64>,
    ) -> archon_workflow::WorkflowResult<archon_workflow::HostCommandResult> {
        self.runs.fetch_add(1, Ordering::SeqCst);
        Ok(archon_workflow::HostCommandResult {
            exit_code: Some(1),
            stdout: String::new(),
            stderr: "refused".into(),
            stdout_bytes: 0,
            stderr_bytes: 7,
            timed_out: false,
            interrupted: false,
            stdout_truncated: false,
            stderr_truncated: false,
            gate_envelope: None,
            publication_receipt: None,
            subjects: Vec::new(),
            postcondition: None,
        })
    }
}

#[tokio::test]
async fn a_resume_runs_each_interrupted_sibling_landing_again() {
    let (_temp, store, run_id) = new_run();
    let host = Arc::new(RefusingHost {
        runs: AtomicUsize::new(0),
    });
    let (first_session, _rx) = runner(
        &store,
        &run_id,
        Arc::new(PanicLlm),
        Some(host.clone()),
        None,
    );
    first_session
        .run(TWO_LANDINGS_SCRIPT)
        .await
        .expect("the first session ends");
    assert_eq!(host.runs.load(Ordering::SeqCst), 2);

    // The disk as the issue found it: both landings stopped by a pause, each
    // a stub with no subject and no receipt.
    let v2 = WorkflowV2ResultStore::new(store.run_dir(&run_id).join("v2"));
    let mut slots: Vec<_> = v2
        .load_call_records()
        .unwrap()
        .into_iter()
        .filter(|record| record.call.method == WorkflowV2HostMethod::HostCommand)
        .collect();
    assert_eq!(slots.len(), 2, "{slots:?}");
    slots.sort_by(|a, b| a.started_at.cmp(&b.started_at));
    for record in &mut slots {
        record.status = WorkflowV2Status::NeedsReview;
        record.result.status = WorkflowV2Status::NeedsReview;
        record.result.data["interrupted"] = serde_json::json!("paused");
        let raw = serde_json::to_vec_pretty(&*record).unwrap();
        std::fs::write(v2.result_path(&record.call.id), raw).unwrap();
    }
    assert!(slots[0].started_at < slots[1].started_at);
    set_status(&store, &run_id, archon_workflow::RunStatus::Paused);
    resume(&store, &run_id);

    let (resumed, _rx) = runner(
        &store,
        &run_id,
        Arc::new(PanicLlm),
        Some(host.clone()),
        None,
    );
    resumed
        .run(TWO_LANDINGS_SCRIPT)
        .await
        .expect("the resume ends");
    assert_eq!(
        host.runs.load(Ordering::SeqCst),
        4,
        "each interrupted landing runs again once on resume"
    );
    for record in &slots {
        let now = v2.load_call_record(&record.call.id).unwrap().unwrap();
        assert!(
            now.result.data["interrupted"] != "paused",
            "the stub is replaced by a real answer: {:?}",
            now.result.data
        );
    }
}
