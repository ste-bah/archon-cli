//! Exact staged-byte views of authoritative prepared freeze transactions.

use super::*;

impl PreparedAcceptanceFreeze {
    pub(crate) fn into_staged_parts(
        self,
    ) -> (
        crate::command::workflow_gate::GateEvaluation,
        Vec<(String, Vec<u8>)>,
    ) {
        let publication_identity = self.publication_identity();
        let evaluation = crate::command::workflow_gate::GateEvaluation::new("", self.findings)
            .with_publication_identity(publication_identity);
        let outputs = vec![
            (ACCEPTANCE_CONTRACT_FILE.to_string(), self.contract_bytes),
            (
                archon_workflow::task_set_contract::ACCEPTANCE_LOCK_FILE.to_string(),
                serde_json::to_vec_pretty(&self.lock).expect("acceptance lock serializes"),
            ),
            (
                "acceptance-pin.json".to_string(),
                serde_json::to_vec_pretty(&self.pin).expect("acceptance pin serializes"),
            ),
        ];
        (evaluation, outputs)
    }
}

impl PreparedSkeletonFreeze {
    pub(crate) fn into_staged_parts(
        self,
    ) -> (
        crate::command::workflow_gate::GateEvaluation,
        Vec<(String, Vec<u8>)>,
    ) {
        let publication_identity = self.publication_identity();
        let evaluation = crate::command::workflow_gate::GateEvaluation::new("", self.findings)
            .with_publication_identity(publication_identity);
        let outputs = vec![
            (TASK_SKELETON_FILE.to_string(), self.skeleton_bytes),
            (
                archon_workflow::task_set_contract::TASK_SKELETON_LOCK_FILE.to_string(),
                serde_json::to_vec_pretty(&self.lock).expect("skeleton lock serializes"),
            ),
            (
                "acceptance-pin.json".to_string(),
                serde_json::to_vec_pretty(&self.pin).expect("acceptance pin serializes"),
            ),
        ];
        (evaluation, outputs)
    }
}
