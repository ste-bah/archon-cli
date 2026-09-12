use archon_workflow::{WorkflowStore, WorkflowError, WorkflowResult, WorkflowEventKind, WorkflowEventLog};

pub(super) fn record(store: &WorkflowStore, run_id: &str, attempt: usize, error: &str, script: Option<&str>) -> WorkflowResult<()> {
    store.with_run_lock(run_id, |store| {
        let relative = format!("rejected-scripts/attempt-{attempt}.js");
        if let Some(script) = script {
            let path = store.run_dir(run_id).join(&relative);
            std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| WorkflowError::io(&path, e))?;
            std::fs::write(&path, script).map_err(|e| WorkflowError::io(&path, e))?;
        }
        let detail = serde_json::json!({"attempt":attempt,"error":error,"script_path":script.map(|_| relative)});
        store.write_run_json(run_id, &format!("rejected-scripts/attempt-{attempt}.json"), &detail)?;
        let seq = store.next_event_seq(run_id)?;
        WorkflowEventLog::new(store.clone()).emit(run_id, seq, WorkflowEventKind::ScriptPreflightRejected, detail)?;
        eprintln!("script preflight rejected attempt {attempt}: {error}");
        Ok(())
    })
}
