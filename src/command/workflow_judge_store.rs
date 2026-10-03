//! A staged freeze's judge verdicts, saved for its retry (Issue 255).
//!
//! The judge batch is the freeze's longest provider call (it has taken
//! close to half an hour on a large task set). A freeze stopped later, by
//! its budget or by the host, used to ask it again from nothing. The
//! verdicts are saved under the digest of the judge's EXACT input -- the
//! batch prompt (which holds every judged check verbatim), the ids it must
//! answer, the resolved model and the provider -- so a retry with an
//! identical batch reuses them and any other batch is judged afresh. Only a
//! reply that passed validation is saved; an unreadable or mismatched file
//! is ignored.

use super::judge::{PartialReply, batched_judge_prompt, judge_contract_resumable};
use super::*;
use crate::command::workflow_freeze_budget::{FREEZE_CACHE_DIR, FreezeProgress};

const SCHEMA: u32 = 1;

#[derive(serde::Serialize, serde::Deserialize)]
struct Saved {
    schema: u32,
    key: String,
    judged: AcceptanceContract,
}

pub(super) struct JudgeStore {
    dir: PathBuf,
    progress: Arc<FreezeProgress>,
}

impl JudgeStore {
    /// The project's freeze cache (`FREEZE_CACHE_DIR`), outside every
    /// probed input and the call's staging directory, counted in `progress`.
    pub(super) fn for_project(project_root: &Path, progress: Arc<FreezeProgress>) -> Self {
        let dir = project_root.join(FREEZE_CACHE_DIR).join("judge");
        Self { dir, progress }
    }

    #[cfg(test)]
    pub(super) fn at_with_progress(dir: PathBuf, progress: Arc<FreezeProgress>) -> Self {
        Self { dir, progress }
    }

    #[cfg(test)]
    pub(super) fn at(dir: PathBuf) -> Self {
        Self {
            dir,
            progress: Arc::default(),
        }
    }

    /// The digest of the judge's exact input.
    fn key(
        client: &dyn WorkflowLlmClient,
        subset: &AcceptanceContract,
        expected: &BTreeSet<String>,
    ) -> Result<String> {
        let input = serde_json::json!([
            "acceptance-judge-batch-v1",
            batched_judge_prompt(subset)?,
            expected,
            client.resolve_model_alias("sonnet"),
            client.provider_id(),
        ]);
        Ok(content_digest(input.to_string().as_bytes()))
    }

    fn load(&self, key: &str, subset: &AcceptanceContract) -> Option<AcceptanceContract> {
        let path = self.dir.join(format!("{key}.json"));
        let saved: Saved = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
        let ids = |contract: &AcceptanceContract| {
            (contract.acceptance.iter())
                .chain(&contract.supplementary)
                .map(|entry| entry.id.clone())
                .collect::<Vec<_>>()
        };
        (saved.schema == SCHEMA && saved.key == key && ids(&saved.judged) == ids(subset))
            .then_some(saved.judged)
    }

    fn save(&self, key: &str, judged: &AcceptanceContract) -> bool {
        let saved = Saved {
            schema: SCHEMA,
            key: key.to_string(),
            judged: judged.clone(),
        };
        let path = self.dir.join(format!("{key}.json"));
        let staging = self.dir.join(format!(".{key}.{}.tmp", std::process::id()));
        let written = std::fs::create_dir_all(&self.dir).is_ok()
            && serde_json::to_vec(&saved)
                .is_ok_and(|bytes| std::fs::write(&staging, bytes).is_ok())
            && std::fs::rename(&staging, &path).is_ok();
        if !written {
            let _ = std::fs::remove_file(&staging);
            eprintln!(
                "the judge's verdicts could not be saved under {}; a retry judges again",
                self.dir.display()
            );
        }
        written
    }

    /// The judge, answered from a saved identical batch when one
    /// exists, and saved otherwise.
    pub(super) async fn judge(
        &self,
        client: &dyn WorkflowLlmClient,
        subset: AcceptanceContract,
        expected: &BTreeSet<String>,
    ) -> Result<AcceptanceContract> {
        let key = Self::key(client, &subset, expected)?;
        // Issue 260: a partial reply an earlier attempt saved is continued,
        // and the chunks it holds count as progress again, so the count the
        // executor reads never goes back.
        let partial = PartialReply::new(&self.dir, &key, &self.progress);
        partial.count_saved();
        if let Some(judged) = self.load(&key, &subset) {
            eprintln!(
                "acceptance judge: reused the saved verdicts of an identical batch ({key}); no provider call made"
            );
            self.progress.reused(true);
            return Ok(judged);
        }
        let judged = judge_contract_resumable(client, subset, expected, Some(&partial)).await?;
        if self.save(&key, &judged) {
            self.progress.saved(true);
        }
        Ok(judged)
    }
}

#[cfg(test)]
#[path = "workflow_judge_store_tests.rs"]
mod tests;
