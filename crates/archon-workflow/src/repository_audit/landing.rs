//! Per-attempt host-owned records. Model input never selects storage or snapshot authority.
use super::{AuditContract, AuditRecord, AuditReport};
use crate::{WorkflowError, WorkflowResult};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};

pub struct AuditLanding {
    root: PathBuf,
    source: PathBuf,
    contract: AuditContract,
    /// Issue-51: declared paths whose prior record this attempt reuses; named
    /// in `hint()` so the assessor does not gather them.
    carried: usize,
    lock: Mutex<()>,
}
tokio::task_local! { static LANDING: Arc<AuditLanding>; }
pub async fn scope<T>(landing: Arc<AuditLanding>, work: impl std::future::Future<Output = T>) -> T {
    LANDING.scope(landing, work).await
}
pub fn current() -> Option<Arc<AuditLanding>> {
    LANDING.try_with(Clone::clone).ok()
}
fn invalid(message: impl std::fmt::Display) -> WorkflowError {
    WorkflowError::ArtifactInvalid(message.to_string())
}
impl AuditLanding {
    pub fn open(root: PathBuf, source: PathBuf, contract: AuditContract) -> WorkflowResult<Self> {
        for path in &contract.declared_paths {
            super::contract::validate_path(path).map_err(invalid)?;
        }
        std::fs::create_dir_all(&root).map_err(|e| WorkflowError::io(&root, e))?;
        let path = root.join("contract.json");
        let identity = json!({"contract":contract,"source":source});
        if path.exists() {
            let saved: Value = serde_json::from_slice(&std::fs::read(&path).map_err(invalid)?)?;
            if saved != identity {
                return Err(invalid("audit landing snapshot/contract identity mismatch"));
            }
        } else {
            atomic(&path, &serde_json::to_vec(&identity)?)?;
        }
        Ok(Self {
            root,
            source,
            contract,
            carried: 0,
            lock: Mutex::new(()),
        })
    }
    /// Record how many declared paths this attempt carries forward (Issue-51).
    pub fn carrying(mut self, carried: usize) -> Self {
        self.carried = carried;
        self
    }
    pub fn land(&self, record: AuditRecord) -> WorkflowResult<()> {
        let _guard = self.lock.lock().map_err(invalid)?;
        if !self.contract.declared_paths.contains(&record.declared_path) {
            return Err(invalid("unexpected audit declared_path"));
        }
        self.validate(&record)?;
        let path = self.record_path(&record.declared_path);
        atomic(&path, &serde_json::to_vec(&record)?)
    }
    fn record_path(&self, path: &str) -> PathBuf {
        use sha2::{Digest, Sha256};
        self.root
            .join(format!("{:x}.json", Sha256::digest(path.as_bytes())))
    }
    fn validate(&self, record: &AuditRecord) -> WorkflowResult<()> {
        let contract = AuditContract {
            declared_paths: vec![record.declared_path.clone()],
            ..self.contract.clone()
        };
        let report = AuditReport {
            schema_version: 1,
            snapshot: self.contract.snapshot.clone(),
            records: vec![record.clone()],
        };
        contract.validate_report(&report).map_err(invalid)?;
        super::contract::validate_files(&self.source, &report).map_err(invalid)
    }
    fn records(&self) -> WorkflowResult<BTreeMap<String, AuditRecord>> {
        let mut records = BTreeMap::new();
        for path in &self.contract.declared_paths {
            let file = self.record_path(path);
            match std::fs::read(&file) {
                Ok(bytes) => {
                    let record: AuditRecord = serde_json::from_slice(&bytes)?;
                    if record.declared_path != *path {
                        return Err(invalid("audit record storage identity mismatch"));
                    }
                    self.validate(&record)?;
                    records.insert(path.clone(), record);
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(WorkflowError::io(&file, e)),
            }
        }
        Ok(records)
    }
    pub fn remaining(&self) -> WorkflowResult<Vec<String>> {
        let _guard = self.lock.lock().map_err(invalid)?;
        let records = self.records()?;
        Ok(self
            .contract
            .declared_paths
            .iter()
            .filter(|p| !records.contains_key(*p))
            .cloned()
            .collect())
    }
    pub fn report(&self) -> WorkflowResult<AuditReport> {
        let _guard = self.lock.lock().map_err(invalid)?;
        let report = AuditReport {
            schema_version: 1,
            snapshot: self.contract.snapshot.clone(),
            records: self.records()?.into_values().collect(),
        };
        self.contract.validate_report(&report).map_err(invalid)?;
        Ok(report)
    }
    /// The exact compact completion the host accepts once every path is landed (fixed key order).
    pub fn accepted_completion(&self) -> String {
        format!(
            "{{\"schema_version\":1,\"snapshot\":{},\"records_landed\":{}}}",
            json!(self.contract.snapshot),
            self.contract.declared_paths.len()
        )
    }
    pub fn hint(&self) -> WorkflowResult<String> {
        let remaining = self.remaining()?;
        let carried = if self.carried == 0 {
            String::new()
        } else {
            format!(
                " {} other declared path(s) keep their prior verdict from the host ledger and are not part of this attempt; do not gather or land them.",
                self.carried
            )
        };
        Ok(format!(
            "Host-retained audit records: {} of {} landed for snapshot {}. Remaining paths: {}.{carried} Do not re-gather landed paths. Establish and call land-audit-record for each remaining path. Finish with exactly this data.repository_audit object (not a prose summary): {}",
            self.contract.declared_paths.len() - remaining.len(),
            self.contract.declared_paths.len(),
            self.contract.snapshot,
            serde_json::to_string(&remaining)?,
            self.accepted_completion()
        ))
    }
    /// Issue-49: only the three required fields carry authority; extra keys are ignored because
    /// the report is assembled from the host ledger. Every refusal names the field(s) at fault
    /// and shows the exact accepted object.
    pub fn complete(&self, value: &Value) -> WorkflowResult<AuditReport> {
        let accepted = self.accepted_completion();
        let refuse = |fault: String| {
            invalid(format!(
                "{fault}; accepted completion is exactly {accepted}"
            ))
        };
        if value.as_object().is_none() {
            return Err(refuse(
                "compact audit completion must be a JSON object".into(),
            ));
        }
        if value.get("schema_version") != Some(&json!(1)) {
            return Err(refuse(format!(
                "schema_version={} but host requires 1",
                value
                    .get("schema_version")
                    .map_or("missing".to_owned(), Value::to_string)
            )));
        }
        if value.get("snapshot") != Some(&json!(self.contract.snapshot)) {
            return Err(refuse(format!(
                "snapshot={} but host snapshot is {:?}",
                value
                    .get("snapshot")
                    .map_or("missing".to_owned(), Value::to_string),
                self.contract.snapshot
            )));
        }
        let report = self.report()?;
        // Issue-39: name both counts so the agent can see which side is wrong.
        let given = value.get("records_landed").and_then(Value::as_u64);
        if given != Some(report.records.len() as u64) {
            return Err(refuse(format!(
                "records_landed={} but host retained {} record(s)",
                given.map_or("missing".to_owned(), |n| n.to_string()),
                report.records.len()
            )));
        }
        Ok(report)
    }
}
fn atomic(path: &std::path::Path, bytes: &[u8]) -> WorkflowResult<()> {
    let tmp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    std::fs::write(&tmp, bytes).map_err(|e| WorkflowError::io(&tmp, e))?;
    std::fs::rename(&tmp, path).map_err(|e| WorkflowError::io(path, e))
}

/// Complete only the host-authorized compact shape, before generic evidence validation.
pub(crate) fn expand_result(
    request: &crate::WorkflowV2AgentRequest,
    result: &mut crate::WorkflowV2Result,
) -> Result<(), crate::WorkflowV2AgentError> {
    if !request
        .call
        .options
        .extra
        .contains_key("repository_audit_contract")
        || result.status != crate::WorkflowV2Status::Accepted
    {
        return Ok(());
    }
    let Some(raw) = result
        .data
        .get("repository_audit")
        .filter(|v| v.get("records_landed").is_some())
    else {
        return Ok(());
    };
    let fail = |e: String| crate::WorkflowV2AgentError::InvalidResult(e);
    let landing = current().ok_or_else(|| fail("no host audit landing context".into()))?;
    let report = landing.complete(raw).map_err(|e| fail(e.to_string()))?;
    result.data["repository_audit"] =
        serde_json::to_value(&report).map_err(|e| fail(e.to_string()))?;
    if result.summary.trim().is_empty() {
        result.summary = format!(
            "Host assembled {} landed audit records",
            report.records.len()
        );
    }
    result.evidence.push(crate::WorkflowV2Evidence::new(
        crate::WorkflowV2EvidenceKind::Inspection,
        format!(
            "Host validated snapshot {} and retained {} records in {}",
            report.snapshot,
            report.records.len(),
            landing.root.display()
        ),
    ));
    Ok(())
}

pub(crate) fn record_rejection(output: &str, error: &crate::WorkflowV2AgentError) {
    let Some(landing) = current() else {
        return;
    };
    let path = landing
        .root
        .join(format!("rejected-{}.json", uuid::Uuid::new_v4()));
    let result = serde_json::to_vec(&json!({"error":error.to_string(),"output":output}))
        .map_err(invalid)
        .and_then(|bytes| atomic(&path, &bytes));
    if let Err(e) = result {
        eprintln!("audit rejection evidence could not be persisted: {e}");
    }
    eprintln!("repository audit response rejected: {error}");
}
