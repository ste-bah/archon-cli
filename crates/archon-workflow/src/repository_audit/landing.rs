//! Per-attempt host-owned records. Model input never selects storage or snapshot authority.
use super::{AuditContract, AuditRecord, AuditReport};
use crate::{WorkflowError, WorkflowResult};
use std::{path::PathBuf, sync::{Arc, Mutex}, collections::BTreeMap};
use serde_json::{Value, json};

pub struct AuditLanding {
    root: PathBuf,
    source: PathBuf,
    contract: AuditContract,
    lock: Mutex<()>,
}
tokio::task_local! { static LANDING: Arc<AuditLanding>; }
pub async fn scope<T>(landing: Arc<AuditLanding>, work: impl std::future::Future<Output=T>) -> T {
    LANDING.scope(landing, work).await
}
pub fn current() -> Option<Arc<AuditLanding>> { LANDING.try_with(Clone::clone).ok() }
fn invalid(message: impl std::fmt::Display) -> WorkflowError { WorkflowError::ArtifactInvalid(message.to_string()) }
impl AuditLanding {
    pub fn open(root: PathBuf, source: PathBuf, contract: AuditContract) -> WorkflowResult<Self> {
        for path in &contract.declared_paths { super::contract::validate_path(path).map_err(invalid)?; }
        std::fs::create_dir_all(&root).map_err(|e| WorkflowError::io(&root,e))?;
        let path = root.join("contract.json");
        let identity = json!({"contract":contract,"source":source});
        if path.exists() {
            let saved: Value = serde_json::from_slice(&std::fs::read(&path).map_err(invalid)?)?;
            if saved != identity { return Err(invalid("audit landing snapshot/contract identity mismatch")); }
        } else { atomic(&path, &serde_json::to_vec(&identity)?)?; }
        Ok(Self {root, source, contract, lock:Mutex::new(())})
    }
    pub fn land(&self, record: AuditRecord) -> WorkflowResult<()> {
        let _guard = self.lock.lock().map_err(invalid)?;
        if !self.contract.declared_paths.contains(&record.declared_path) { return Err(invalid("unexpected audit declared_path")); }
        self.validate(&record)?;
        let path = self.record_path(&record.declared_path);
        atomic(&path, &serde_json::to_vec(&record)?)
    }
    fn record_path(&self, path: &str) -> PathBuf {
        use sha2::{Digest,Sha256};
        self.root.join(format!("{:x}.json",Sha256::digest(path.as_bytes())))
    }
    fn validate(&self, record: &AuditRecord) -> WorkflowResult<()> {
        let contract = AuditContract {declared_paths:vec![record.declared_path.clone()],..self.contract.clone()};
        let report = AuditReport {schema_version:1,snapshot:self.contract.snapshot.clone(),records:vec![record.clone()]};
        contract.validate_report(&report).map_err(invalid)?;
        super::contract::validate_files(&self.source,&report).map_err(invalid)
    }
    fn records(&self) -> WorkflowResult<BTreeMap<String,AuditRecord>> {
        let mut records = BTreeMap::new();
        for path in &self.contract.declared_paths {
            let file = self.record_path(path);
            match std::fs::read(&file) {
                Ok(bytes) => {
                    let record: AuditRecord = serde_json::from_slice(&bytes)?;
                    if record.declared_path != *path { return Err(invalid("audit record storage identity mismatch")); }
                    self.validate(&record)?;
                    records.insert(path.clone(),record);
                }
                Err(e) if e.kind()==std::io::ErrorKind::NotFound => {},
                Err(e) => return Err(WorkflowError::io(&file,e)),
            }
        }
        Ok(records)
    }
    pub fn remaining(&self) -> WorkflowResult<Vec<String>> {
        let _guard = self.lock.lock().map_err(invalid)?;
        let records = self.records()?;
        Ok(self.contract.declared_paths.iter().filter(|p| !records.contains_key(*p)).cloned().collect())
    }
    pub fn report(&self) -> WorkflowResult<AuditReport> {
        let _guard = self.lock.lock().map_err(invalid)?;
        let report = AuditReport {schema_version:1,snapshot:self.contract.snapshot.clone(),records:self.records()?.into_values().collect()};
        self.contract.validate_report(&report).map_err(invalid)?;
        Ok(report)
    }
    pub fn hint(&self) -> WorkflowResult<String> {
        let remaining = self.remaining()?;
        Ok(format!("Host-retained audit records: {} of {} landed for snapshot {}. Remaining paths: {}. Do not re-gather landed paths. Establish and call land-audit-record for each remaining path. Finish with records_landed={} (not a prose summary).",
            self.contract.declared_paths.len()-remaining.len(),self.contract.declared_paths.len(), self.contract.snapshot,
            serde_json::to_string(&remaining)?,self.contract.declared_paths.len()))
    }
    pub fn complete(&self, value: &Value) -> WorkflowResult<AuditReport> {
        if value.get("snapshot") != Some(&json!(self.contract.snapshot)) || value.get("schema_version") != Some(&json!(1)) {
            return Err(invalid("audit landing completion snapshot/schema mismatch"));
        }
        let report = self.report()?;
        if value.get("records_landed").and_then(Value::as_u64) != Some(report.records.len() as u64) {
            return Err(invalid("records_landed does not match host saved records"));
        }
        Ok(report)
    }
}
fn atomic(path: &std::path::Path, bytes: &[u8]) -> WorkflowResult<()> {
    let tmp = path.with_extension(format!("{}.tmp",uuid::Uuid::new_v4()));
    std::fs::write(&tmp,bytes).map_err(|e| WorkflowError::io(&tmp,e))?;
    std::fs::rename(&tmp,path).map_err(|e| WorkflowError::io(path,e))
}
