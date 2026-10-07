//! The host's staged in-round authoring (`v2/acceptance-authoring.json`):
//! every accepted entry the moment it is accepted, and each owed id's last
//! finding; split from `acceptance_author` for size.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::command::workflow_task_set::passability::{
    SCHEMA,
    evidence::{BEGIN, END},
};
use archon_workflow::WorkflowLlmClient;
use archon_workflow::task_set_contract::{AcceptanceCriterion, JudgeDecision, content_digest};
use serde::{Deserialize, Serialize};

/// A receipt for the evidence verdict, tied to this entry and its judge input.
#[derive(Debug, Serialize, Deserialize)]
struct PassabilityStamp {
    schema: u32,
    evidence_verdict: JudgeDecision,
    input_digest: String,
}

/// A staged author feedback is kept to this many characters.
const FEEDBACK_CHARS: usize = 4000;

/// Accepted entries authored so far, and each owed id's latest finding.
#[derive(Debug, Default, Serialize, Deserialize)]
pub(in super::super) struct Staged {
    #[serde(default)]
    prd_digest: String,
    #[serde(default)]
    pub(in super::super) entries: BTreeMap<String, AcceptanceCriterion>,
    #[serde(default)]
    passability: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub(in super::super) feedback: BTreeMap<String, String>,
}

pub(in super::super) fn staging_path(run_dir: &Path) -> PathBuf {
    run_dir.join("v2").join("acceptance-authoring.json")
}

impl Staged {
    /// The run's staged authoring for this PRD; empty when the PRD moved
    /// (or when there is none). A staging file that exists but cannot be
    /// read is an error: what it held is never silently dropped.
    pub(in super::super) fn load(run_dir: &Path, prd_digest: &str) -> Result<Self, String> {
        let path = staging_path(run_dir);
        let staged: Self = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| {
                format!(
                    "the staged authoring {} is unreadable: {error}",
                    path.display()
                )
            })?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(error) => {
                return Err(format!(
                    "the staged authoring {} is unreadable: {error}",
                    path.display()
                ));
            }
        };
        Ok(if staged.prd_digest == prd_digest {
            staged
        } else {
            Self {
                prd_digest: prd_digest.to_string(),
                ..Self::default()
            }
        })
    }

    /// Persist what is staged (write, then rename).
    pub(in super::super) fn save(&self, run_dir: &Path) -> Result<(), String> {
        archon_workflow::stage_write::with_write(|| {
            archon_workflow::WorkflowResult::Ok(self.save_owned(run_dir))
        })
        .map_err(|error| error.to_string())?
    }

    fn save_owned(&self, run_dir: &Path) -> Result<(), String> {
        let path = staging_path(run_dir);
        let fail = |error: std::io::Error| {
            format!(
                "the staged authoring {} cannot be written: {error}",
                path.display()
            )
        };
        std::fs::create_dir_all(path.parent().expect("staging has a parent")).map_err(fail)?;
        let temporary = path.with_extension("json.tmp");
        let bytes = serde_json::to_vec_pretty(self).map_err(|error| error.to_string())?;
        std::fs::write(&temporary, bytes).map_err(fail)?;
        std::fs::rename(&temporary, &path).map_err(fail)
    }

    fn input_digest(
        &self,
        entry: &AcceptanceCriterion,
        baseline: &str,
        client: &dyn WorkflowLlmClient,
        model: &str,
    ) -> String {
        content_digest(
            serde_json::json!([
                SCHEMA,
                self.prd_digest,
                entry,
                baseline,
                client.resolve_model_alias(model),
                client.provider_id()
            ])
            .to_string()
            .as_bytes(),
        )
    }

    /// Only current accepted evidence for exactly this input can be reused.
    pub(in super::super) fn proven(
        &self,
        id: &str,
        baseline: &str,
        client: &dyn WorkflowLlmClient,
        model: &str,
    ) -> bool {
        self.entries
            .get(id)
            .zip(self.passability.get(id))
            .is_some_and(|(entry, saved)| {
                let Ok(stamp) = serde_json::from_value::<PassabilityStamp>(saved.clone()) else {
                    return false;
                };
                entry.judgment.verdict == JudgeDecision::Accepted
                    && stamp.schema == SCHEMA
                    && stamp.evidence_verdict == JudgeDecision::Accepted
                    && stamp.input_digest == self.input_digest(entry, baseline, client, model)
            })
    }

    /// Stage a check only after its executability and evidence gates cleared.
    pub(in super::super) fn accept(
        &mut self,
        entry: AcceptanceCriterion,
        baseline: &str,
        client: &dyn WorkflowLlmClient,
        model: &str,
    ) {
        let stamp = PassabilityStamp {
            schema: SCHEMA,
            evidence_verdict: JudgeDecision::Accepted,
            input_digest: self.input_digest(&entry, baseline, client, model),
        };
        self.passability.insert(
            entry.id.clone(),
            serde_json::to_value(stamp).expect("a passability stamp is serializable"),
        );
        self.feedback.remove(&entry.id);
        self.entries.insert(entry.id.clone(), entry);
    }

    /// Record that `id` is not publishable, and why: authored again later.
    pub(in super::super) fn reject(&mut self, id: &str, why: &str) {
        self.entries.remove(id);
        self.passability.remove(id);
        self.feedback.insert(id.to_string(), truncate(why));
    }

    /// Clear everything once it is published; why it could not be, if so.
    pub(in super::super) fn clear(run_dir: &Path) -> Option<String> {
        archon_workflow::stage_write::with_write(|| {
            archon_workflow::WorkflowResult::Ok(Self::clear_owned(run_dir))
        })
        .unwrap_or_else(|error| Some(error.to_string()))
    }

    fn clear_owned(run_dir: &Path) -> Option<String> {
        match std::fs::remove_file(staging_path(run_dir)) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Some(format!(
                "the published checks' staging {} could not be cleared: {error}",
                staging_path(run_dir).display()
            )),
            _ => None,
        }
    }
}

fn truncate(text: &str) -> String {
    if text.chars().count() <= FEEDBACK_CHARS {
        return text.to_string();
    }
    // Reserve the closing marker before cutting: an author never sees an
    // open untrusted-output fence, even when several findings were assembled.
    let suffix = " [truncated]";
    let budget = FEEDBACK_CHARS - suffix.chars().count() - END.chars().count() - 1;
    let mut out: String = text.chars().take(budget).collect();
    if let Some(begin) = out.rfind(BEGIN)
        && out.rfind(END).is_none_or(|end| end < begin)
    {
        out.push('\n');
        out.push_str(END);
    }
    out.push_str(suffix);
    out
}
