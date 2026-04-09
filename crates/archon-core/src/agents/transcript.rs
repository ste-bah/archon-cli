//! Sidechain transcript recording and agent resume (AGT-024).
//!
//! Records every subagent's messages to a JSONL file for persistence and resume.
//! Path: `~/.archon/sessions/{session_id}/subagents/agent-{agent_id}.jsonl`
//! Metadata: `~/.archon/sessions/{session_id}/subagents/agent-{agent_id}.meta.json`

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// AgentMetadata — sidecar metadata for transcript files
// ---------------------------------------------------------------------------

/// Metadata stored alongside the transcript JSONL file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMetadata {
    pub agent_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
}

// ---------------------------------------------------------------------------
// AgentTranscriptStore — fire-and-forget JSONL recording
// ---------------------------------------------------------------------------

/// Manages transcript JSONL files for subagents in a session.
#[derive(Debug, Clone)]
pub struct AgentTranscriptStore {
    base_dir: PathBuf,
}

impl AgentTranscriptStore {
    /// Create a store rooted at `~/.archon/sessions/{session_id}/subagents/`.
    pub fn new(session_id: &str) -> Option<Self> {
        let home = dirs::home_dir()?;
        Some(Self {
            base_dir: home
                .join(".archon/sessions")
                .join(session_id)
                .join("subagents"),
        })
    }

    /// Create a store at an explicit base directory (for testing).
    pub fn with_base_dir(base_dir: PathBuf) -> Self {
        Self { base_dir }
    }

    /// Path to the transcript JSONL file.
    pub fn transcript_path(&self, agent_id: &str) -> PathBuf {
        self.base_dir.join(format!("agent-{agent_id}.jsonl"))
    }

    /// Path to the metadata sidecar JSON file.
    pub fn metadata_path(&self, agent_id: &str) -> PathBuf {
        self.base_dir.join(format!("agent-{agent_id}.meta.json"))
    }

    /// Fire-and-forget: append a message to the transcript JSONL.
    /// Failure is logged as warning, never blocks agent execution.
    pub fn record_message(&self, agent_id: &str, message: &serde_json::Value) {
        if let Err(e) = self.append_jsonl(agent_id, message) {
            tracing::warn!(agent_id, error = %e, "Failed to record transcript message");
        }
    }

    /// Write metadata sidecar.
    pub fn write_metadata(&self, agent_id: &str, meta: &AgentMetadata) {
        if let Err(e) = self.write_metadata_inner(agent_id, meta) {
            tracing::warn!(agent_id, error = %e, "Failed to write agent metadata");
        }
    }

    /// Read metadata sidecar (returns None if missing or malformed).
    pub fn read_metadata(&self, agent_id: &str) -> Option<AgentMetadata> {
        let path = self.metadata_path(agent_id);
        let raw = std::fs::read_to_string(&path).ok()?;
        serde_json::from_str(&raw).ok()
    }

    /// Read all messages from a transcript JSONL.
    pub fn get_transcript(&self, agent_id: &str) -> Option<Vec<serde_json::Value>> {
        let path = self.transcript_path(agent_id);
        let content = std::fs::read_to_string(&path).ok()?;
        let messages: Vec<serde_json::Value> = content
            .lines()
            .filter(|l| !l.trim().is_empty())
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect();
        if messages.is_empty() {
            None
        } else {
            Some(messages)
        }
    }

    // -- internal helpers --

    fn ensure_dir(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.base_dir)
    }

    fn append_jsonl(
        &self,
        agent_id: &str,
        message: &serde_json::Value,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.ensure_dir()?;
        let path = self.transcript_path(agent_id);
        let line = serde_json::to_string(message)?;
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        writeln!(file, "{line}")?;
        Ok(())
    }

    fn write_metadata_inner(
        &self,
        agent_id: &str,
        meta: &AgentMetadata,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.ensure_dir()?;
        let path = self.metadata_path(agent_id);
        let json = serde_json::to_string_pretty(meta)?;
        std::fs::write(&path, json)?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Resume support (AC-109)
// ---------------------------------------------------------------------------

/// Resume context loaded from a transcript + metadata sidecar.
#[derive(Debug)]
pub struct ResumeContext {
    /// Messages from the transcript JSONL to inject as initial history.
    pub messages: Vec<serde_json::Value>,
    /// Agent type from the metadata (used to resolve agent definition).
    pub agent_type: String,
    /// Worktree path from metadata (if it still exists on disk).
    pub worktree_path: Option<String>,
}

/// Load a resume context for a previously interrupted agent.
///
/// Returns `None` if the transcript is missing or empty.
/// Falls back to `"general-purpose"` if metadata is missing or has no agent_type.
pub fn load_resume_context(
    store: &AgentTranscriptStore,
    agent_id: &str,
) -> Option<ResumeContext> {
    let messages = store.get_transcript(agent_id)?;
    let metadata = store.read_metadata(agent_id);

    let agent_type = metadata
        .as_ref()
        .map(|m| m.agent_type.clone())
        .unwrap_or_else(|| "general-purpose".into());

    // Only report worktree if it still exists on disk
    let worktree_path = metadata
        .as_ref()
        .and_then(|m| m.worktree_path.clone())
        .filter(|p| std::path::Path::new(p).is_dir());

    Some(ResumeContext {
        messages,
        agent_type,
        worktree_path,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_store() -> (AgentTranscriptStore, TempDir) {
        let tmp = TempDir::new().unwrap();
        let store = AgentTranscriptStore::with_base_dir(tmp.path().to_path_buf());
        (store, tmp)
    }

    #[test]
    fn transcript_path_format() {
        let (store, _tmp) = test_store();
        let path = store.transcript_path("abc123");
        assert!(path.ends_with("agent-abc123.jsonl"));
    }

    #[test]
    fn metadata_path_format() {
        let (store, _tmp) = test_store();
        let path = store.metadata_path("abc123");
        assert!(path.ends_with("agent-abc123.meta.json"));
    }

    #[test]
    fn record_and_get_transcript() {
        let (store, _tmp) = test_store();
        let msg1 = serde_json::json!({"role": "user", "content": "hello"});
        let msg2 = serde_json::json!({"role": "assistant", "content": "world"});
        store.record_message("test-1", &msg1);
        store.record_message("test-1", &msg2);

        let transcript = store.get_transcript("test-1").unwrap();
        assert_eq!(transcript.len(), 2);
        assert_eq!(transcript[0]["role"], "user");
        assert_eq!(transcript[1]["role"], "assistant");
    }

    #[test]
    fn get_transcript_returns_none_for_missing() {
        let (store, _tmp) = test_store();
        assert!(store.get_transcript("nonexistent").is_none());
    }

    #[test]
    fn write_and_read_metadata() {
        let (store, _tmp) = test_store();
        let meta = AgentMetadata {
            agent_type: "explore".into(),
            worktree_path: Some("/tmp/wt".into()),
            description: Some("test agent".into()),
            filename: None,
        };
        store.write_metadata("test-2", &meta);

        let read_back = store.read_metadata("test-2").unwrap();
        assert_eq!(read_back.agent_type, "explore");
        assert_eq!(read_back.worktree_path.as_deref(), Some("/tmp/wt"));
        assert_eq!(read_back.description.as_deref(), Some("test agent"));
    }

    #[test]
    fn read_metadata_returns_none_for_missing() {
        let (store, _tmp) = test_store();
        assert!(store.read_metadata("nonexistent").is_none());
    }

    #[test]
    fn metadata_serde_roundtrip() {
        let meta = AgentMetadata {
            agent_type: "code-reviewer".into(),
            worktree_path: None,
            description: Some("reviews code".into()),
            filename: None,
        };
        let json = serde_json::to_string(&meta).unwrap();
        let restored: AgentMetadata = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.agent_type, "code-reviewer");
        assert!(restored.worktree_path.is_none());
        assert_eq!(restored.description.as_deref(), Some("reviews code"));
    }

    #[test]
    fn metadata_skips_none_fields_in_json() {
        let meta = AgentMetadata {
            agent_type: "explore".into(),
            worktree_path: None,
            description: None,
            filename: None,
        };
        let json = serde_json::to_string(&meta).unwrap();
        assert!(!json.contains("worktree_path"));
        assert!(!json.contains("description"));
        assert!(!json.contains("filename"));
    }

    #[test]
    fn record_message_failure_does_not_panic() {
        // Store with an invalid base dir that can't be created
        let store = AgentTranscriptStore::with_base_dir(PathBuf::from("/dev/null/impossible"));
        // Should not panic — fire-and-forget
        store.record_message("x", &serde_json::json!({"test": true}));
    }

    #[test]
    fn multiple_agents_separate_transcripts() {
        let (store, _tmp) = test_store();
        store.record_message("agent-a", &serde_json::json!({"role": "user", "content": "a"}));
        store.record_message("agent-b", &serde_json::json!({"role": "user", "content": "b"}));

        let ta = store.get_transcript("agent-a").unwrap();
        let tb = store.get_transcript("agent-b").unwrap();
        assert_eq!(ta.len(), 1);
        assert_eq!(tb.len(), 1);
        assert_eq!(ta[0]["content"], "a");
        assert_eq!(tb[0]["content"], "b");
    }

    #[test]
    fn new_returns_some_when_home_exists() {
        // This test depends on $HOME existing
        if dirs::home_dir().is_some() {
            let store = AgentTranscriptStore::new("test-session");
            assert!(store.is_some());
            let store = store.unwrap();
            assert!(store.base_dir.to_str().unwrap().contains("test-session"));
            assert!(store.base_dir.to_str().unwrap().contains("subagents"));
        }
    }

    // -----------------------------------------------------------------------
    // Resume context tests (AC-109)
    // -----------------------------------------------------------------------

    #[test]
    fn resume_context_loads_transcript_and_metadata() {
        let (store, _tmp) = test_store();
        let meta = AgentMetadata {
            agent_type: "explore".into(),
            worktree_path: None,
            description: Some("test".into()),
            filename: None,
        };
        store.write_metadata("resume-1", &meta);
        store.record_message("resume-1", &serde_json::json!({"role": "user", "content": "hi"}));
        store.record_message("resume-1", &serde_json::json!({"role": "assistant", "content": "hello"}));

        let ctx = load_resume_context(&store, "resume-1").unwrap();
        assert_eq!(ctx.agent_type, "explore");
        assert_eq!(ctx.messages.len(), 2);
        assert!(ctx.worktree_path.is_none());
    }

    #[test]
    fn resume_context_falls_back_to_general_purpose() {
        let (store, _tmp) = test_store();
        // No metadata written — just a transcript
        store.record_message("resume-2", &serde_json::json!({"role": "user", "content": "x"}));

        let ctx = load_resume_context(&store, "resume-2").unwrap();
        assert_eq!(ctx.agent_type, "general-purpose");
    }

    #[test]
    fn resume_context_returns_none_for_missing_transcript() {
        let (store, _tmp) = test_store();
        assert!(load_resume_context(&store, "nonexistent").is_none());
    }

    #[test]
    fn resume_context_filters_nonexistent_worktree_path() {
        let (store, _tmp) = test_store();
        let meta = AgentMetadata {
            agent_type: "plan".into(),
            worktree_path: Some("/tmp/nonexistent-worktree-abc123".into()),
            description: None,
            filename: None,
        };
        store.write_metadata("resume-3", &meta);
        store.record_message("resume-3", &serde_json::json!({"role": "user", "content": "test"}));

        let ctx = load_resume_context(&store, "resume-3").unwrap();
        assert_eq!(ctx.agent_type, "plan");
        // Worktree path filtered because directory doesn't exist
        assert!(ctx.worktree_path.is_none());
    }
}
