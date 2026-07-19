#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowV2TaskCompletionEvidenceKind {
    ImplementationCandidate,
    FocusedVerification,
    VerifiedNoop,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowV2TaskCompletionEvidence {
    pub task_id: String,
    pub evidence_kind: WorkflowV2TaskCompletionEvidenceKind,
    pub call_id: String,
    pub item_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_item_id: Option<String>,
    pub status: WorkflowV2Status,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence_refs: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifact_paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub command_refs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_input_hash: Option<String>,
}

impl WorkflowV2TaskCompletionEvidence {
    pub fn new(
        task_id: impl Into<String>,
        evidence_kind: WorkflowV2TaskCompletionEvidenceKind,
        call_id: impl Into<String>,
        item_id: impl Into<String>,
        status: WorkflowV2Status,
    ) -> Self {
        Self {
            task_id: task_id.into(),
            evidence_kind,
            call_id: call_id.into(),
            item_id: item_id.into(),
            source_call_id: None,
            source_item_id: None,
            status,
            evidence_refs: Vec::new(),
            artifact_paths: Vec::new(),
            command_refs: Vec::new(),
            source_fingerprint: None,
            item_input_hash: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowV2CallRecord {
    #[serde(default)]
    pub run_id: String,
    pub call: WorkflowV2HostCall,
    pub attempt: u32,
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    #[serde(default)]
    pub started_at: String,
    #[serde(default)]
    pub finished_at: String,
    pub input_hash: String,
    #[serde(default)]
    pub output_hash: String,
    pub status: WorkflowV2Status,
    pub result: WorkflowV2Result,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invalidated_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_task_graph: Option<WorkflowV2SourceTaskGraph>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub completed_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scaffold_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub completion_evidence: Vec<WorkflowV2TaskCompletionEvidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_snapshot_hash: Option<String>,
}

impl WorkflowV2CallRecord {
    pub fn new(
        run_id: impl Into<String>,
        call: WorkflowV2HostCall,
        attempt: u32,
        input_hash: String,
        result: WorkflowV2Result,
        depends_on: Vec<String>,
    ) -> Self {
        let now = chrono::Utc::now().to_rfc3339();
        Self {
            run_id: run_id.into(),
            call,
            attempt,
            schema_version: RESULT_SCHEMA_VERSION.to_string(),
            started_at: now.clone(),
            finished_at: now,
            input_hash,
            output_hash: stable_result_hash(&result),
            status: result.status,
            result,
            depends_on,
            invalidated_by: None,
            agent_session_id: None,
            source_fingerprint: None,
            source_task_graph: None,
            completed_ids: Vec::new(),
            scaffold_hash: None,
            completion_evidence: Vec::new(),
            evidence_snapshot_hash: None,
        }
    }

    pub fn with_source_metadata(
        mut self,
        source_fingerprint: Option<String>,
        source_task_graph: Option<WorkflowV2SourceTaskGraph>,
    ) -> Self {
        self.completed_ids = source_task_graph
            .as_ref()
            .map(|graph| graph.completed_ids.clone())
            .unwrap_or_default();
        self.source_fingerprint = source_fingerprint;
        self.source_task_graph = source_task_graph;
        self
    }

    pub fn with_scaffold_hash(mut self, scaffold_hash: Option<String>) -> Self {
        self.scaffold_hash = scaffold_hash;
        self
    }

    pub fn with_completion_evidence(
        mut self,
        completion_evidence: Vec<WorkflowV2TaskCompletionEvidence>,
    ) -> Self {
        let mut completed = self.completed_ids.iter().cloned().collect::<BTreeSet<_>>();
        for evidence in &completion_evidence {
            if matches!(
                evidence.status,
                WorkflowV2Status::Accepted | WorkflowV2Status::Noop
            ) && !evidence.task_id.trim().is_empty()
            {
                completed.insert(evidence.task_id.clone());
            }
        }
        self.completed_ids = completed.into_iter().collect();
        self.completion_evidence = completion_evidence;
        self
    }

    pub fn with_evidence_snapshot_hash(mut self, evidence_snapshot_hash: Option<String>) -> Self {
        self.evidence_snapshot_hash = evidence_snapshot_hash;
        self
    }

    pub fn is_reusable_for(&self, input_hash: &str) -> bool {
        self.input_hash == input_hash
            && self.invalidated_by.is_none()
            && matches!(
                self.status,
                WorkflowV2Status::Accepted | WorkflowV2Status::Noop
            )
            && self.result.validate().is_ok()
    }

    pub fn is_reusable_for_source(
        &self,
        input_hash: &str,
        source_fingerprint: Option<&str>,
    ) -> bool {
        self.is_reusable_for_source_and_scaffold(input_hash, source_fingerprint, None)
    }

    pub fn is_reusable_for_source_and_scaffold(
        &self,
        input_hash: &str,
        source_fingerprint: Option<&str>,
        scaffold_hash: Option<&str>,
    ) -> bool {
        self.is_reusable_for(input_hash)
            && match (&self.source_fingerprint, source_fingerprint) {
                (Some(recorded), Some(current)) => recorded == current,
                (None, None) => true,
                _ => false,
            }
            && match (&self.scaffold_hash, scaffold_hash) {
                (Some(recorded), Some(current)) => recorded == current,
                (None, None) => true,
                _ => false,
            }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowV2SourceTaskGraph {
    pub schema_version: String,
    #[serde(default)]
    pub canonical_task_universe: Vec<String>,
    #[serde(default)]
    pub items: Vec<WorkflowV2SourceTaskItem>,
    #[serde(default)]
    pub completed_ids: Vec<String>,
}

impl WorkflowV2SourceTaskGraph {
    pub fn new(
        canonical_task_universe: Vec<String>,
        items: Vec<WorkflowV2SourceTaskItem>,
        completed_ids: Vec<String>,
    ) -> Self {
        Self {
            schema_version: "workflow-v2-source-task-graph-v1".to_string(),
            canonical_task_universe,
            items,
            completed_ids,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowV2SourceTaskItem {
    pub item_id: String,
    #[serde(default)]
    pub canonical_task_ids: Vec<String>,
    #[serde(default)]
    pub dependency_ids: Vec<String>,
    #[serde(default)]
    pub target_files: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub declared_target_files: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub target_file_expansions: Vec<WorkflowV2SourceTargetExpansion>,
    #[serde(default)]
    pub acceptance_criteria: Vec<String>,
    #[serde(default)]
    pub focused_verification: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub expected_evidence: Vec<String>,
    #[serde(default)]
    pub artifact_requirements: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowV2SourceTargetExpansion {
    pub source: String,
    #[serde(default)]
    pub expanded: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

fn default_schema_version() -> String {
    RESULT_SCHEMA_VERSION.to_string()
}

fn stable_result_hash(result: &WorkflowV2Result) -> String {
    let bytes = serde_json::to_vec(result).unwrap_or_default();
    blake3::hash(&bytes).to_hex().to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct WorkflowV2Checkpoint {
    #[serde(default)]
    pub completed_call_ids: Vec<String>,
}

impl WorkflowV2Checkpoint {
    pub fn mark_completed(&mut self, call_id: &str) {
        if !self.completed_call_ids.iter().any(|id| id == call_id) {
            self.completed_call_ids.push(call_id.to_string());
        }
    }

    pub fn remove_completed_call(&mut self, call_id: &str) {
        self.completed_call_ids.retain(|id| id != call_id);
    }

    pub fn remove_completed(&mut self, call_ids: &BTreeSet<String>) {
        self.completed_call_ids
            .retain(|call_id| !call_ids.contains(call_id));
    }
}
