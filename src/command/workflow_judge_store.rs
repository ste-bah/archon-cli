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

use super::judge::{PartialReply, require_judged_prose};
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
    roots: super::judge::JudgeRoots,
}

impl JudgeStore {
    /// The project's freeze cache (`FREEZE_CACHE_DIR`), outside every
    /// probed input and the call's staging directory, counted in `progress`.
    pub(super) fn for_project(
        project_root: &Path,
        progress: Arc<FreezeProgress>,
        roots: super::judge::JudgeRoots,
    ) -> Self {
        let dir = project_root.join(FREEZE_CACHE_DIR).join("judge");
        Self {
            dir,
            progress,
            roots,
        }
    }

    #[cfg(test)]
    pub(super) fn at_with_progress(dir: PathBuf, progress: Arc<FreezeProgress>) -> Self {
        Self {
            dir,
            progress,
            roots: Default::default(),
        }
    }

    #[cfg(test)]
    pub(super) fn at(dir: PathBuf) -> Self {
        Self {
            dir,
            progress: Arc::default(),
            roots: Default::default(),
        }
    }

    /// The digest of the judge's exact input.
    #[cfg(test)]
    fn key(
        client: &dyn WorkflowLlmClient,
        subset: &AcceptanceContract,
        expected: &BTreeSet<String>,
    ) -> Result<String> {
        Self::key_with_roots(client, subset, expected, &Default::default())
    }

    fn key_with_roots(
        client: &dyn WorkflowLlmClient,
        subset: &AcceptanceContract,
        expected: &BTreeSet<String>,
        roots: &super::judge::JudgeRoots,
    ) -> Result<String> {
        let input = serde_json::json!([
            "acceptance-judge-batch-v1",
            super::judge::batched_judge_prompt_with_roots(subset, roots)?,
            expected,
            client.resolve_model_alias("sonnet"),
            client.provider_id(),
            current_logic_identity(client),
        ]);
        Ok(content_digest(input.to_string().as_bytes()))
    }

    fn load(
        &self,
        key: &str,
        subset: &AcceptanceContract,
        identity: Option<&serde_json::Value>,
    ) -> Option<AcceptanceContract> {
        let path = self.dir.join(format!("{key}.json"));
        let saved: Saved = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
        let ids = |contract: &AcceptanceContract| {
            (contract.acceptance.iter())
                .chain(&contract.supplementary)
                .map(|entry| entry.id.clone())
                .collect::<Vec<_>>()
        };
        let Some(identity) = identity else {
            return None;
        };
        (saved.schema == SCHEMA
            && saved.key == key
            && ids(&saved.judged) == ids(subset)
            && stamps_match(&saved.judged, identity, &self.roots))
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
            && serde_json::to_vec(&saved).is_ok_and(|bytes| {
                use std::io::Write;
                std::fs::File::create(&staging)
                    .is_ok_and(|mut file| file.write_all(&bytes).is_ok() && file.sync_all().is_ok())
            })
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
    #[cfg(test)]
    pub(super) async fn judge(
        &self,
        client: &dyn WorkflowLlmClient,
        subset: AcceptanceContract,
        expected: &BTreeSet<String>,
    ) -> Result<AcceptanceContract> {
        let key = Self::key_with_roots(client, &subset, expected, &self.roots)?;
        // Issue 260: a partial reply an earlier attempt saved is continued,
        // and the chunks it holds count as progress again, so the count the
        // executor reads never goes back.
        let partial = PartialReply::new(&self.dir, &key, &self.progress);
        let identity = current_logic_identity(client);
        let saved = self.load(&key, &subset, identity.as_ref());
        partial.count_saved(saved.is_some());
        if let Some(judged) = saved {
            // Repair a crash after verdict persistence but before marking
            // the partial complete. The verdict is already durable here.
            partial.completed();
            eprintln!(
                "acceptance judge: reused the saved verdicts of an identical batch ({key}); no provider call made"
            );
            self.progress.reused(true);
            return Ok(judged);
        }
        let judged = super::judge::judge_contract_resumable_with_roots(
            client,
            subset,
            expected,
            Some(&partial),
            &self.roots,
        )
        .await?;
        if self.save(&key, &judged) {
            // Never destroy the continuation before the verdict is on disk.
            partial.completed();
            self.progress.saved(true);
        }
        Ok(judged)
    }

    /// Judge an incremental subset, whose shape is not a complete contract.
    pub(super) async fn judge_subset(
        &self,
        client: &dyn WorkflowLlmClient,
        subset: AcceptanceContract,
        expected: &BTreeSet<String>,
    ) -> Result<AcceptanceContract> {
        let key = Self::key_with_roots(client, &subset, expected, &self.roots)?;
        let partial = PartialReply::new(&self.dir, &key, &self.progress);
        let identity = current_logic_identity(client);
        let saved = self
            .load(&key, &subset, identity.as_ref())
            .filter(|judged| {
                require_judged_prose(judged).is_ok()
                    && judged
                        .acceptance
                        .iter()
                        .chain(&judged.supplementary)
                        .map(|entry| entry.id.clone())
                        .collect::<BTreeSet<_>>()
                        == *expected
            });
        partial.count_saved(saved.is_some());
        if let Some(judged) = saved {
            partial.completed();
            eprintln!(
                "acceptance judge: reused the saved verdicts of an identical batch ({key}); no provider call made"
            );
            self.progress.reused(true);
            return Ok(judged);
        }
        let judged = super::judge::judge_contract_subset_resumable_with_roots(
            client,
            subset,
            Some(&partial),
            &self.roots,
        )
        .await?;
        if self.save(&key, &judged) {
            partial.completed();
            self.progress.saved(true);
        }
        Ok(judged)
    }
}

fn current_logic_identity(client: &dyn WorkflowLlmClient) -> Option<serde_json::Value> {
    crate::command::workflow_task_set::workflow_acceptance_check_reuse::logic_identity().map(
        |logic| {
            crate::command::workflow_task_set::reauthor::cache::acceptance_logic_identity(
                logic,
                &client.resolve_model_alias("sonnet"),
                client.provider_id().as_deref(),
            )
        },
    )
}

fn stamps_match(
    judged: &AcceptanceContract,
    identity: &serde_json::Value,
    roots: &super::judge::JudgeRoots,
) -> bool {
    judged
        .acceptance
        .iter()
        .chain(&judged.supplementary)
        .all(|entry| {
            let Some(sampling) = entry.judgment.sampling.as_ref() else {
                return false;
            };
            sampling.get("logic_identity") == Some(identity)
                && sampling
                    .get("named_file_digest")
                    .and_then(serde_json::Value::as_str)
                    == Some(super::judge::named_files_content_digest(entry, roots).as_str())
        })
}

#[cfg(test)]
#[path = "workflow_judge_store_tests.rs"]
mod tests;
