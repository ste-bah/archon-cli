//! The audit's durable state, and the assessment plumbing of `AuditRuntime`,
//! split out of `runtime.rs` to hold the 500-line ceiling.
use super::*;

pub const STATE_PATH: &str = "v2/repository-audit/state.json";
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub identity: String,
    pub root: PathBuf,
    pub paths: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditState {
    pub schema_version: u32,
    pub generation: u64,
    pub budget: AuditBudget,
    pub ledger: AuditLedger,
    pub declared_paths: BTreeSet<String>,
    pub snapshot: Option<Snapshot>,
    pub attempts: u64,
    pub last_error: Option<String>,
    #[serde(default)]
    pub final_receipt: Option<FinalReceipt>,
    #[serde(default)]
    pub operator_controls: Vec<serde_json::Value>,
    #[serde(default)]
    pub policy_provenance: Option<serde_json::Value>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FinalReceipt {
    pub generation: u64,
    pub snapshot: String,
    pub declared_paths: BTreeSet<String>,
    pub assessment_count: usize,
}
impl AuditState {
    /// Record the discharges (Issue-104) and contests (Issue-112) the host's
    /// records support for the latest report, replacing its earlier ones.
    pub fn rejudge(&mut self, run_dir: &std::path::Path, repository: &std::path::Path) {
        let Some(report) = self.ledger.history.last().cloned() else {
            return;
        };
        let (discharges, contests) =
            crate::repository_audit::contest::judge(run_dir, &report, repository);
        self.ledger.record_discharges(discharges);
        self.ledger.record_contests(contests);
    }
    pub fn require_final_receipt(&self) -> WorkflowResult<()> {
        let snapshot = self.snapshot.as_ref().ok_or_else(|| {
            WorkflowError::StateCorrupt("repository audit final snapshot missing".into())
        })?;
        let expected = FinalReceipt {
            generation: self.generation,
            snapshot: snapshot.identity.clone(),
            declared_paths: self.declared_paths.clone(),
            assessment_count: self.ledger.history.len(),
        };
        if self.final_receipt.as_ref() != Some(&expected) {
            return Err(WorkflowError::StateCorrupt(
                "repository audit final assessment receipt missing or stale".into(),
            ));
        }
        Ok(())
    }
}

impl AuditRuntime {
    pub(super) async fn await_assessment<T>(
        &self,
        id: &str,
        allowance: Option<u64>,
        work: impl std::future::Future<Output = WorkflowResult<T>>,
    ) -> WorkflowResult<T> {
        let work = crate::control_race::until_run_stops_from_generation(
            &self.store,
            &self.run_id,
            id,
            Some(self.generation),
            work,
        );
        tokio::pin!(work);
        let timeout = async {
            match allowance {
                Some(ms) => tokio::time::sleep(Duration::from_millis(ms)).await,
                None => std::future::pending().await,
            }
        };
        tokio::pin!(timeout);
        let mut heartbeat = tokio::time::interval(Duration::from_secs(5));
        loop {
            tokio::select! {
                result=&mut work=>return result,
                _=&mut timeout=>return Err(WorkflowError::ControlPaused("repository audit configured execution allowance exhausted".into())),
                _=heartbeat.tick()=>self.update(|s|s.budget.heartbeat(id,chrono::Utc::now().timestamp_millis()))?,
            }
        }
    }
    pub(in crate::repository_audit) fn event(
        &self,
        kind: WorkflowEventKind,
        detail: serde_json::Value,
    ) -> WorkflowResult<()> {
        self.store.with_run_lock(&self.run_id, |store| {
            let seq = store.next_event_seq(&self.run_id)?;
            WorkflowEventLog::new(store.clone())
                .emit(&self.run_id, seq, kind, detail)
                .map(|_| ())
        })
    }
}
pub(super) fn validate_files(snapshot: &Snapshot, report: &AuditReport) -> WorkflowResult<()> {
    crate::repository_audit::contract::validate_files(&snapshot.root, report)
        .map_err(|error| WorkflowError::ArtifactInvalid(error.to_string()))
}
