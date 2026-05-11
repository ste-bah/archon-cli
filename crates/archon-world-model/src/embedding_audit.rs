//! Policy-ledger audit wrapper for external world-model embeddings.

use std::io::Write;
use std::path::PathBuf;

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::embedding::{
    EmbeddingBackendKind, EmbeddingRequest, EmbeddingVector, WorldEmbeddingAdapter,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmbeddingAuditEvent {
    pub event_id: String,
    pub provider: String,
    pub model: String,
    pub source_hash: String,
    pub redaction_policy: String,
    pub dimensions: usize,
    pub policy_reason: String,
    pub created_at: DateTime<Utc>,
}

pub struct AuditedEmbeddingAdapter {
    inner: Box<dyn WorldEmbeddingAdapter>,
    ledger_path: PathBuf,
    policy_reason: String,
}

impl AuditedEmbeddingAdapter {
    pub fn new(
        inner: Box<dyn WorldEmbeddingAdapter>,
        ledger_path: PathBuf,
        policy_reason: impl Into<String>,
    ) -> Self {
        Self {
            inner,
            ledger_path,
            policy_reason: policy_reason.into(),
        }
    }
}

impl WorldEmbeddingAdapter for AuditedEmbeddingAdapter {
    fn backend_kind(&self) -> EmbeddingBackendKind {
        self.inner.backend_kind()
    }

    fn dimensions(&self) -> usize {
        self.inner.dimensions()
    }

    fn provider_name(&self) -> &str {
        self.inner.provider_name()
    }

    fn model_name(&self) -> &str {
        self.inner.model_name()
    }

    fn embed(&self, request: &EmbeddingRequest) -> Result<EmbeddingVector> {
        let vector = self.inner.embed(request)?;
        if vector.provider != "local" && self.inner.backend_kind() == EmbeddingBackendKind::External
        {
            append_embedding_audit_event(
                &self.ledger_path,
                EmbeddingAuditEvent {
                    event_id: format!("world-embedding-audit-{}", uuid::Uuid::new_v4()),
                    provider: vector.provider.clone(),
                    model: vector.model.clone(),
                    source_hash: vector.source_hash.clone(),
                    redaction_policy: vector.redaction_policy.clone(),
                    dimensions: vector.values.len(),
                    policy_reason: self.policy_reason.clone(),
                    created_at: Utc::now(),
                },
            )?;
        }
        Ok(vector)
    }
}

pub fn append_embedding_audit_event(path: &PathBuf, event: EmbeddingAuditEvent) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut line = serde_json::to_vec(&event)?;
    line.push(b'\n');
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?
        .write_all(&line)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ExternalAdapter;

    impl WorldEmbeddingAdapter for ExternalAdapter {
        fn backend_kind(&self) -> EmbeddingBackendKind {
            EmbeddingBackendKind::External
        }

        fn dimensions(&self) -> usize {
            2
        }

        fn provider_name(&self) -> &str {
            "openai"
        }

        fn model_name(&self) -> &str {
            "text-embedding-3-small"
        }

        fn embed(&self, request: &EmbeddingRequest) -> Result<EmbeddingVector> {
            Ok(EmbeddingVector {
                values: vec![1.0, 0.0],
                provider: "openai".into(),
                model: "text-embedding-3-small".into(),
                source_hash: request.source_hash.clone(),
                redaction_policy: request.redaction_policy.clone(),
            })
        }
    }

    #[test]
    fn external_embedding_calls_write_policy_ledger() {
        let temp = tempfile::tempdir().unwrap();
        let ledger = temp.path().join("embedding-policy-events.jsonl");
        let adapter = AuditedEmbeddingAdapter::new(
            Box::new(ExternalAdapter),
            ledger.clone(),
            "policy allowed",
        );

        adapter
            .embed(&EmbeddingRequest {
                text: "hello".into(),
                source_hash: "source-1".into(),
                redaction_policy: "redacted".into(),
            })
            .unwrap();

        let content = std::fs::read_to_string(ledger).unwrap();
        assert!(content.contains("\"provider\":\"openai\""));
        assert!(content.contains("\"policy_reason\":\"policy allowed\""));
    }
}
