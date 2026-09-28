//! The write-wave harness's scripted repository audit. Split from
//! `write_wave_fixture.rs` for the 500-line ceiling.
use archon_workflow::repository_audit::AuditContract;
use archon_workflow::*;
use serde_json::json;
use std::path::Path;

impl super::Scripted {
    pub(super) fn audit_records(&self, root: &Path, contract: &AuditContract) -> WorkflowV2Result {
        let (_, script) = self.audit.as_ref().unwrap();
        let records = contract
            .declared_paths
            .iter()
            .map(|path| {
                let exists = root.join(path).exists();
                if let Some((_, equivalents)) = script.flagged.iter().find(|(p, _)| p == path)
                    && !exists
                {
                    return json!({"declared_path": path, "verdict": "exists_elsewhere",
                        "equivalents": equivalents, "required_action": "wire_or_migrate",
                        "reason": "scripted: exists at another path"});
                }
                json!({"declared_path": path, "verdict": if exists {"exists_as_declared"} else {"absent"},
                    "equivalents": [], "required_action": if exists {"none"} else {"deliver"},
                    "reason": "scripted: inspected sealed source"})
            })
            .collect::<Vec<_>>();
        let mut result = WorkflowV2Result::accepted("assessed sealed source");
        result.data = json!({"repository_audit": {"schema_version": 1, "snapshot": contract.snapshot, "records": records}});
        result
    }

    pub(super) fn dispositions_for(&self, branch_id: &str) -> serde_json::Value {
        let Some((runtime, script)) = self.audit.as_ref() else {
            return json!([]);
        };
        let snapshot = runtime.state().unwrap().snapshot.unwrap().identity;
        let entries = script
            .dispositions
            .get(branch_id)
            .cloned()
            .unwrap_or_default();
        json!(entries.iter().map(|(declared, evidence)| json!({
            "declared_path": declared, "snapshot": snapshot,
            "explanation": "created the declared file beside the existing one and left the equivalent untouched",
            "evidence_paths": evidence,
        })).collect::<Vec<_>>())
    }
}
