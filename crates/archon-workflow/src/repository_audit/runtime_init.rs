//! Executor-bound audit initialization under the run-control lock.
use super::*;
impl AuditRuntime {
    pub fn initialize(
        store: WorkflowStore,
        run_id: String,
        policy: AuditPolicy,
    ) -> WorkflowResult<Self> {
        Self::initialize_for_executor(store, run_id, policy, None)
    }
    /// Initialize only for the executor captured by the caller, under the write lock.
    pub fn initialize_for_executor(
        store: WorkflowStore,
        run_id: String,
        policy: AuditPolicy,
        executor: Option<u64>,
    ) -> WorkflowResult<Self> {
        let generation = store.with_run_lock(&run_id, |locked| {
            let run = locked.load_state(&run_id)?;
            let generation = executor.unwrap_or(run.generation);
            crate::control_pause::require_executor(&run, generation)?;
            let path = locked.run_dir(&run_id).join(STATE_PATH);
            let required = locked
                .run_dir(&run_id)
                .join("v2/repository-audit/required.json");
            if required.exists() && !path.exists() {
                return Err(WorkflowError::StateCorrupt(
                    "mandatory repository audit state is missing".into(),
                ));
            }
            if path.exists() {
                let mut state: AuditState = serde_json::from_slice(
                    &std::fs::read(&path).map_err(|e| WorkflowError::io(&path, e))?,
                )?;
                if state.schema_version != 1 {
                    return Err(WorkflowError::StateCorrupt(
                        "unsupported audit state schema".into(),
                    ));
                }
                if state.generation != generation {
                    state
                        .budget
                        .recover_interrupted(chrono::Utc::now().timestamp_millis())?;
                    state.generation = generation;
                    state.final_receipt = None;
                }
                locked.write_run_json(&run_id, STATE_PATH, &state)?;
            } else {
                locked.write_run_json(
                    &run_id,
                    STATE_PATH,
                    &AuditState {
                        schema_version: 1,
                        generation,
                        budget: AuditBudget::new(policy),
                        ledger: AuditLedger::default(),
                        declared_paths: BTreeSet::new(),
                        snapshot: None,
                        attempts: 0,
                        last_error: None,
                        final_receipt: None,
                        operator_controls: vec![],
                        policy_provenance: None,
                    },
                )?;
            }
            locked.write_run_json(
                &run_id,
                "v2/repository-audit/required.json",
                &json!({"schema_version":1}),
            )?;
            Ok(generation)
        })?;
        Ok(Self {
            store,
            run_id,
            generation,
            assessment_lock: Arc::new(tokio::sync::Mutex::new(())),
            write_boundary_lock: Arc::new(tokio::sync::Mutex::new(())),
        })
    }
}
