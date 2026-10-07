//! A freeze probe that spans attempts (Issue 255).
//!
//! A freeze's probe used to keep its verdicts in process memory only, so a
//! freeze the host killed at its wall clock lost every check it had run.
//! Here a freeze probe given a staged [`FreezeResume`]:
//!
//! - stages each verdict to disk the moment its check finishes, under the
//!   probe's existing memo key (the site's policy, the tree, the project
//!   data's state, the check's id and exact content: `sites::memo_key`), so
//!   a changed check, commit or data set never meets an old verdict; a
//!   verdict is committed only after final observation validation;
//! - bounds each check by its site's own no-progress window, watching its
//!   output and process-tree activity; there is no total budget or share;
//! - keeps the nonce of each input mutation it draws, so a retry builds the
//!   same mutated check and meets its saved verdict too.
//!
//! Only verdicts are saved, never an operational result: a check that
//! timed out or could not run -- or failed for its host, gave no verdict
//! (Issue 328: `verdict::may_be_host_failure`) -- is run again by the next attempt.

use std::path::PathBuf;

use archon_workflow::acceptance_scratch::{AllowanceHook, CheckHook, ObserveHooks};
use serde::{Deserialize, Serialize};

use super::*;
use crate::command::workflow_freeze_budget::{
    FREEZE_CACHE_DIR, FreezeBudget, FreezeIncomplete, FreezeResume,
};
use crate::command::workflow_task_set::passability::evidence::Redactor;

const SCHEMA: u32 = 3;

#[derive(Serialize, Deserialize)]
struct Saved {
    schema: u32,
    key: String,
    result: CheckResult,
}

/// Saved verdicts, one file per memo key.
///
/// Issue 277: a saved verdict is persisted outside the run and outlives it,
/// so what its check printed is redacted first, of every credential value
/// the check's site could hold (credential-named host values, the engine's
/// own credentials, and the names its policy forwards), and the file is
/// readable by its owner only. The host computes classification from raw
/// streams before redaction and persists it alongside the evidence. Reuse
/// never reclassifies redacted output. Older raw-stream caches are discarded.
#[derive(Clone)]
pub(super) struct ResultStore {
    dir: PathBuf,
    redactor: Arc<Redactor>,
}

impl ResultStore {
    pub(super) fn new(dir: PathBuf, redactor: Redactor) -> Self {
        Self {
            dir,
            redactor: Arc::new(redactor),
        }
    }

    fn path(&self, key: &str) -> Option<PathBuf> {
        (key.len() == 64 && key.bytes().all(|b| b.is_ascii_hexdigit()))
            .then(|| self.dir.join(format!("{key}.json")))
    }

    /// The verdict saved under `key`; anything unreadable is no verdict.
    pub(super) fn load(&self, key: &str) -> Option<CheckResult> {
        let _ = std::fs::remove_file(self.path(key)?.with_extension("provisional"));
        let path = self.path(key)?;
        let saved = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Saved>(&bytes).ok());
        match saved {
            Some(saved)
                if saved.schema == SCHEMA
                    && saved.key == key
                    && saved.result.operational_error.is_none()
                    && saved.result.classification.is_some()
                    && !super::verdict::may_be_host_failure(&saved.result) =>
            {
                Some(saved.result)
            }
            _ => {
                let _ = std::fs::remove_file(path);
                None
            }
        }
    }

    /// Save `result` under `key`, atomically; false when it was not saved.
    pub(super) fn save(&self, key: &str, result: &CheckResult) -> bool {
        let Some(path) = self.path(key) else {
            return false;
        };
        // Issue 328: nor a run that failed for its host (a program that
        // could not start, a tree that did not build): no verdict.
        if result.operational_error.is_some() || super::verdict::may_be_host_failure(result) {
            return false;
        }
        let mut result = result.clone();
        // Native command results are already classified at capture. Declarative
        // results have no command and cannot be a script crash.
        result.classify_raw("");
        if let Some(classification) = &mut result.classification
            && let CheckRunClass::ScriptDefect(defect) = &mut classification.crash
        {
            defect.signal = "see the fenced stderr below".into();
            defect.rule =
                String::from_utf8_lossy(&self.redactor.redact_bytes(defect.rule.as_bytes()))
                    .into_owned();
        }
        result.stdout = self.redactor.redact_bytes(&result.stdout);
        result.stderr = self.redactor.redact_bytes(&result.stderr);
        let saved = Saved {
            schema: SCHEMA,
            key: key.to_string(),
            result,
        };
        let path = path.with_extension("provisional");
        let staging = self.dir.join(format!(".{key}.{}.tmp", std::process::id()));
        let written = std::fs::create_dir_all(&self.dir).is_ok()
            && serde_json::to_vec(&saved).is_ok_and(|bytes| write_owner_only(&staging, &bytes))
            && std::fs::rename(&staging, &path).is_ok();
        if !written {
            let _ = std::fs::remove_file(&staging);
        }
        written
    }

    /// The mutation nonce saved under `key` (see `mutation_markers`).
    fn load_nonce(&self, key: &str) -> Option<String> {
        let path = self.path(key)?.with_extension("nonce");
        Some(std::fs::read_to_string(path).ok()?.trim().to_string())
    }

    fn save_nonce(&self, key: &str, nonce: &str) {
        if let Some(path) = self.path(key) {
            let path = path.with_extension("nonce");
            let staging = path.with_extension(format!("nonce.{}.tmp", std::process::id()));
            let _ = std::fs::create_dir_all(&self.dir)
                .and_then(|()| std::fs::write(&staging, nonce))
                .and_then(|()| std::fs::rename(&staging, &path));
            let _ = std::fs::remove_file(&staging);
        }
    }

    pub(super) fn remove(&self, key: &str) {
        if let Some(path) = self.path(key) {
            let _ = std::fs::remove_file(path.with_extension("provisional"));
        }
    }
}

/// Write `bytes` to `path`, readable by its owner only (a stale file left
/// at `path` by an earlier process keeps no wider mode).
fn write_owner_only(path: &std::path::Path, bytes: &[u8]) -> bool {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let restrict = |file: &std::fs::File| {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))
        }
        #[cfg(not(unix))]
        {
            let _ = file;
            Ok(())
        }
    };
    options
        .open(path)
        .and_then(|mut file| restrict(&file).and_then(|()| file.write_all(bytes)))
        .is_ok()
}

impl HostProbe {
    /// Run under `resume`'s budget, saving verdicts when it says so.
    pub(crate) fn with_resume(mut self, resume: &FreezeResume) -> Self {
        self.resume = resume.clone();
        self
    }

    /// Bound every check by `secs` instead of its site's own limit.
    #[cfg(test)]
    pub(crate) fn with_check_cap(mut self, secs: u64) -> Self {
        self.check_cap_secs = secs;
        self
    }

    /// Stand for a new process: verdicts come only from disk.
    #[cfg(test)]
    pub(crate) fn without_process_memo(mut self) -> Self {
        self.process_memo = false;
        self
    }

    #[cfg(test)]
    pub(super) fn process_memo(&self) -> bool {
        self.process_memo
    }

    #[cfg(not(test))]
    pub(super) fn process_memo(&self) -> bool {
        true
    }

    /// The site's full per-check no-progress window at every probe site.
    pub(super) fn check_bound_secs(&self) -> u64 {
        if self.memo {
            self.resume.budget.check_bound(self.check_cap_secs)
        } else {
            self.check_cap_secs
        }
    }

    pub(super) fn budget(&self) -> &FreezeBudget {
        &self.resume.budget
    }

    /// Where this probe's verdicts are saved, when they are: a freeze probe
    /// at a hermetic site, under its build cache (outside every live root)
    /// or the project's freeze cache.
    pub(super) fn store(&self) -> Option<ResultStore> {
        if !(self.memo && self.resume.persist) {
            return None;
        }
        let dir = match &self.site {
            Site::Scratch(binding) => (binding.policy.build_cache.clone())
                .unwrap_or_else(|| binding.policy.scratch_parent.join("freeze-probe"))
                .join("probe-results"),
            Site::Hermetic => self.project.join(FREEZE_CACHE_DIR).join("probe-results"),
            Site::Direct | Site::Unavailable(_) => return None,
        };
        let (environment, forwarded) = sites::site_environment(self);
        Some(ResultStore::new(
            dir,
            Redactor::for_environment(environment, &forwarded),
        ))
    }

    /// The markers for mutating check `id` on `tree`. Their nonce is part
    /// of the mutated check, so of its verdict's key: a saving probe keeps
    /// the nonce it first drew (saved before the run), and a retry meets
    /// the same mutated check and its saved verdict. Otherwise new ones.
    pub(super) fn mutation_markers(
        &self,
        tree: &Baseline,
        contract: &AcceptanceContract,
        id: &str,
    ) -> super::mutation::Markers {
        use super::mutation::Markers;
        let (Some(store), Some(check)) = (self.store(), sites::check_key(self, tree, contract, id))
        else {
            return Markers::new();
        };
        let key = content_digest(format!("mutation-nonce\n{check}").as_bytes());
        if let Some(markers) =
            (store.load_nonce(&key)).and_then(|nonce| Markers::with_nonce(&nonce))
        {
            return markers;
        }
        let markers = Markers::new();
        store.save_nonce(&key, markers.nonce());
        markers
    }

    pub(super) fn reused_from_disk(&self) {
        self.resume.progress.reused(false);
    }

    /// The hooks one observation (or copy) of `keys` (id to memo key) runs
    /// under: the budget's allowance per check, and each verdict saved as
    /// it finishes, its key pushed to `written`.
    pub(super) fn observe_hooks(
        &self,
        keys: &BTreeMap<String, String>,
        store: Option<&ResultStore>,
        written: &Arc<Mutex<Vec<String>>>,
    ) -> ObserveHooks {
        let (budget, cap) = (self.resume.budget.clone(), self.check_bound_secs());
        let on_check = store.map(|store| {
            let (store, keys, written) = (store.clone(), keys.clone(), written.clone());
            Arc::new(move |result: &CheckResult| {
                if let Some(key) = keys.get(&result.acceptance_id)
                    && store.save(key, result)
                {
                    written.lock().expect("written lock").push(key.clone());
                }
            }) as CheckHook
        });
        // Only a freeze probe is bounded: a round's probe keeps its site's
        // own limit, as the round itself does.
        let allowance = self
            .memo
            .then(|| Arc::new(move || budget.allowance(cap)) as AllowanceHook);
        ObserveHooks {
            allowance,
            on_check,
        }
    }

    /// Discard verdicts an observation saved before it was voided.
    pub(super) fn unsave(&self, store: Option<&ResultStore>, written: &Mutex<Vec<String>>) {
        let (Some(store), Ok(mut written)) = (store, written.lock()) else {
            return;
        };
        for key in written.drain(..) {
            store.remove(&key);
        }
    }

    /// Promote only after teardown and live-root validation succeeded.
    pub(super) fn promote(&self, store: Option<&ResultStore>, written: &Mutex<Vec<String>>) {
        let (Some(store), Ok(mut written)) = (store, written.lock()) else {
            return;
        };
        for key in written.drain(..) {
            if let Some(path) = store.path(&key)
                && std::fs::rename(path.with_extension("provisional"), path).is_ok()
            {
                self.resume.progress.saved(false);
            }
        }
    }

    /// Record that `ids` got no verdict because the budget ran out.
    pub(super) fn defer<'a>(&self, ids: impl IntoIterator<Item = &'a str>) {
        let mut deferred = self.deferred.lock().expect("deferred lock");
        deferred.extend(ids.into_iter().map(str::to_string));
    }

    pub(super) fn is_incomplete(&self) -> bool {
        !self.deferred.lock().expect("deferred lock").is_empty()
    }

    /// The freeze is incomplete: its budget ran out before every check had
    /// a verdict. Nothing else this probe found is then final.
    pub(crate) fn incomplete(&self) -> Option<FreezeIncomplete> {
        let deferred = self.deferred.lock().expect("deferred lock").clone();
        (!deferred.is_empty()).then(|| {
            FreezeIncomplete::new(
                &self.resume.budget,
                &self.resume.progress,
                deferred.into_iter().collect(),
            )
        })
    }
}

#[cfg(all(test, unix))]
#[path = "workflow_acceptance_executability_resume_tests.rs"]
mod tests;

#[cfg(all(test, unix))]
#[path = "workflow_acceptance_executability_resume_round2_tests.rs"]
mod round2_tests;

#[cfg(all(test, unix))]
#[path = "workflow_acceptance_executability_cap_tests.rs"]
mod cap_tests;

#[cfg(all(test, unix))]
#[path = "workflow_acceptance_executability_bound_tests.rs"]
mod bound_tests;
