use super::*;

struct BlockedLlm {
    started: tokio::sync::Notify,
    release: tokio::sync::Semaphore,
}

#[async_trait::async_trait]
impl WorkflowLlmClient for BlockedLlm {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        self.started.notify_one();
        let _ = self.release.acquire().await;
        let mut result = WorkflowV2Result::accepted("finished");
        result
            .evidence
            .push(archon_workflow::WorkflowV2Evidence::new(
                archon_workflow::WorkflowV2EvidenceKind::Inspection,
                "provider call finished",
            ));
        Ok(WorkflowAgentOutcome {
            content: serde_json::to_string(&result).unwrap(),
            tool_uses: Vec::new(),
            tokens_in: 1,
            tokens_out: 1,
            stop_reason: Some("end_turn".into()),
        })
    }
}

#[test]
#[ignore = "internal subprocess entry"]
fn lease_probe_entry() {
    let dir = std::env::var_os("GROUPF_LEASE_DIR").unwrap();
    let id = std::env::var("GROUPF_LEASE_ID").unwrap();
    let busy = crate::command::workflow_executor_lease::acquire(Path::new(&dir), &id).is_err();
    println!("lease_busy={busy}");
}

fn second_process_busy(store: &WorkflowStore, id: &str) -> bool {
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            &format!(
                "{}::lease_probe_entry",
                module_path!().split_once("::").unwrap().1
            ),
            "--ignored",
            "--nocapture",
        ])
        .env("GROUPF_LEASE_DIR", store.run_dir(id))
        .env("GROUPF_LEASE_ID", id)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(
        text.contains("lease_busy="),
        "subprocess probe did not run: {text}"
    );
    text.contains("lease_busy=true")
}

#[tokio::test]
async fn aborting_outer_resume_keeps_lease_until_blocking_script_stops() {
    let temp = tempfile::tempdir().unwrap();
    let (store, id) = running_run(temp.path());
    let llm = Arc::new(BlockedLlm {
        started: tokio::sync::Notify::new(),
        release: tokio::sync::Semaphore::new(0),
    });
    let outer = tokio::spawn({
        let store = store.clone();
        let id = id.clone();
        let llm = llm.clone();
        let root = temp.path().to_path_buf();
        async move { resume(&root, &store, &id, llm).await }
    });
    tokio::time::timeout(std::time::Duration::from_secs(10), llm.started.notified())
        .await
        .unwrap();
    outer.abort();
    assert!(outer.await.unwrap_err().is_cancelled());
    let held_after_abort = second_process_busy(&store, &id);
    llm.release.close();
    let result_path =
        WorkflowV2ResultStore::new(store.run_dir(&id).join("v2")).result_path("probe");
    // Wait for the actual script and its runtime to stop, rather than just
    // for the provider call to return.
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while !result_path.is_file() || second_process_busy(&store, &id) {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert!(
        held_after_abort,
        "a second process took the lease while the detached script was running"
    );
}
