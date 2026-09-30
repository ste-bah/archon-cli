//! Batch O: the sanctioned in-run amendment of a frozen task set's write
//! scope.
//!
//! A frozen task set's scopes can be wrong in three ways the run itself
//! discovers: the authored script dropped a file a task declares (the
//! declared-file restore), a load-bearing file is declared by no task (the
//! ownerless-file assignment), or the file to fix lies under a deliverable
//! root -- documentation, a task-set artifact path, project data -- that no
//! expansion opens (the deliverable-root grant). Refusing, dropping or merely
//! reporting any of them leaves work that can never be done, so the host
//! amends the scope instead, the way the acceptance republish amends a check
//! (`task_set_lineage`): host-validated, logged, and chained.
//!
//! - **Validated by the host, never by agent text.** A grant names a task of
//!   the universe and one clean path that exists (or that the task already
//!   declares); never engine or run state or the frozen task set
//!   (`residual_paths::protected`); never a file the grantee forbids by a
//!   wider pattern; and a file another task already declares is granted only
//!   as a SHARED grant that records those tasks. Project data
//!   (`residual_paths::project_data`) must be a path the run's project-input
//!   landing can place (`ProjectInputPolicy::placed`), so a change to it
//!   lands through the audited project-input ledger with its backups.
//! - **Chained exactly like the acceptance lineage.** The current grant set
//!   is one document; each amendment appends a link carrying the digest the
//!   set had before and after it, the tasks it changed, what triggered it and
//!   the digest of the link before it. Every version is filed write-once by
//!   its own blake3 digest ([`ChainHistory`]), so any set a link names reads
//!   back and authenticates itself. A reader refuses a chain that does not
//!   hash-link or does not end at the current set.
//! - **Honoured in-run.** The write fan-out overlays the set on the task
//!   universe it plans with ([`amended_universe`]), so the declared-scope
//!   floor, the owner claims and the forbidden lists all see the grant; and
//!   each project-data grant becomes a declared project artifact of the
//!   grantee's branches ([`project_data_grants`]).
//!
//! Every rule is generic: task ids, paths and roots come from the universe
//! and the run; nothing here knows a PRD, a language or a domain.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::task_set_contract::content_digest;
use crate::task_set_lineage::{ChainHistory, ChainRefusal};
use crate::task_universe::WorkflowV2TaskUniverse;

pub const SCOPE_AMENDMENT_SCHEMA: &str = "task-scope-amendments-v1";

/// Why a file is granted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScopeGrantKind {
    /// A file the task declares that its authored scope left out.
    DeclaredRestore,
    /// A load-bearing file no task declares, given to the task whose landing
    /// touched it, else whose focused test runs it, else whose text names it.
    OwnerlessAssignment,
    /// A file under a deliverable root (`residual_paths::deliverable_root`).
    DeliverableRoot,
}

/// Which root a grant's path is relative to, and so how its change lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScopeGrantRoot {
    /// Repository-relative: lands through the branch's patch.
    Repository,
    /// Project-relative project data: lands through the project-input
    /// ledger (`patch_apply::project_inputs_apply`), with backups.
    Project,
}

/// One file granted to one task.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ScopeAmendment {
    pub task_id: String,
    pub path: String,
    pub kind: ScopeGrantKind,
    pub root: ScopeGrantRoot,
    /// Other tasks that already declare the path: the grant is shared with
    /// them, never taken from them. Host-computed on validation.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub shared_with: BTreeSet<String>,
    /// What established the grant (a landing, a finding, a declaration).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub evidence: String,
}

impl ScopeAmendment {
    fn key(&self) -> (&str, &str) {
        (&self.task_id, &self.path)
    }
}

/// Every grant in force, sorted by (task, path), one per pair.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopeAmendmentSet {
    pub schema_version: String,
    pub grants: Vec<ScopeAmendment>,
}

impl Default for ScopeAmendmentSet {
    fn default() -> Self {
        Self {
            schema_version: SCOPE_AMENDMENT_SCHEMA.into(),
            grants: Vec::new(),
        }
    }
}

impl ScopeAmendmentSet {
    pub fn bytes(&self) -> Vec<u8> {
        serde_json::to_vec_pretty(self).expect("a scope amendment set serializes")
    }

    pub fn digest(&self) -> String {
        content_digest(&self.bytes())
    }

    /// The grants of `task_ids`.
    pub fn grants_of<'a>(
        &'a self,
        task_ids: &'a [String],
    ) -> impl Iterator<Item = &'a ScopeAmendment> + 'a {
        self.grants
            .iter()
            .filter(move |grant| task_ids.contains(&grant.task_id))
    }
}

/// One amendment: the set it replaced, the set it wrote, the tasks it
/// changed. Each link carries the digest of the link before it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopeAmendmentLink {
    pub from_digest: String,
    pub to_digest: String,
    pub changed_task_ids: BTreeSet<String>,
    pub trigger: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prior_link_digest: Option<String>,
}

impl ScopeAmendmentLink {
    pub fn digest(&self) -> String {
        content_digest(&serde_json::to_vec(self).expect("a scope amendment link serializes"))
    }
}

/// The run's amendment record: the grants in force and the chain that
/// reached them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopeAmendmentLedger {
    pub set: ScopeAmendmentSet,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lineage: Vec<ScopeAmendmentLink>,
}

/// Why the ledger could not be read, verified or written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeAmendmentError(pub String);

impl std::fmt::Display for ScopeAmendmentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "scope amendment ledger: {}", self.0)
    }
}

impl std::error::Error for ScopeAmendmentError {}

impl From<ChainRefusal> for ScopeAmendmentError {
    fn from(refusal: ChainRefusal) -> Self {
        Self(refusal.to_string())
    }
}

fn error(detail: impl Into<String>) -> ScopeAmendmentError {
    ScopeAmendmentError(detail.into())
}

/// `<run>/v2/scope-amendments.json`: host-written run state no branch sees.
pub fn ledger_path(run_root: &Path) -> PathBuf {
    run_root.join("v2").join("scope-amendments.json")
}

/// The append-only log of every amendment decision beside the ledger.
pub fn log_path(run_root: &Path) -> PathBuf {
    run_root.join("v2").join("scope-amendments.jsonl")
}

/// The write-once store every version of the set is filed in by digest.
pub fn history(run_root: &Path) -> ChainHistory {
    ChainHistory::for_pin(&ledger_path(run_root))
}

impl ScopeAmendmentLedger {
    /// The run's ledger, verified; empty when the run has amended nothing.
    pub fn load(run_root: &Path) -> Result<Self, ScopeAmendmentError> {
        let path = ledger_path(run_root);
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(err) => return Err(error(format!("{} unreadable: {err}", path.display()))),
        };
        let ledger: Self = serde_json::from_slice(&bytes)
            .map_err(|err| error(format!("{} is not a ledger: {err}", path.display())))?;
        ledger.verify(&history(run_root))?;
        Ok(ledger)
    }

    /// The chain hash-links, each link starts where the one before ended, it
    /// ends at the current set, and every set it names is filed by digest.
    pub fn verify(&self, history: &ChainHistory) -> Result<(), ScopeAmendmentError> {
        if self.set.schema_version != SCOPE_AMENDMENT_SCHEMA {
            return Err(error(format!(
                "schema {} is not {SCOPE_AMENDMENT_SCHEMA}",
                self.set.schema_version
            )));
        }
        let empty = ScopeAmendmentSet::default().digest();
        let mut at = empty.clone();
        for (index, link) in self.lineage.iter().enumerate() {
            let expected = index.checked_sub(1).map(|p| self.lineage[p].digest());
            if link.prior_link_digest != expected {
                return Err(error(format!(
                    "link {index} does not carry the digest of the link before it"
                )));
            }
            if link.from_digest != at {
                return Err(error(format!(
                    "link {index} starts at {} but the chain stood at {at}",
                    link.from_digest
                )));
            }
            for digest in [&link.from_digest, &link.to_digest] {
                if *digest != empty && history.get(digest)?.is_none() {
                    return Err(error(format!("set {digest} is not in the chain history")));
                }
            }
            at = link.to_digest.clone();
        }
        let current = self.set.digest();
        if current != at {
            return Err(error(format!(
                "the current set hashes to {current}, but the chain ends at {at}"
            )));
        }
        Ok(())
    }
}

/// `universe` with every grant of `set` in force: a repository grant is a
/// file its task declares (a forbidden one is never granted); a project-data
/// grant is an artifact requirement of its task.
pub fn amended_universe(
    universe: &WorkflowV2TaskUniverse,
    set: &ScopeAmendmentSet,
) -> WorkflowV2TaskUniverse {
    let mut amended = universe.clone();
    for grant in &set.grants {
        let Some(task) = amended
            .tasks
            .iter_mut()
            .find(|task| task.canonical_task_id == grant.task_id)
        else {
            continue;
        };
        match grant.root {
            ScopeGrantRoot::Repository => {
                let declared = format!("`{}` — granted by scope amendment", grant.path);
                if !task.files_expected_to_change.contains(&declared) {
                    task.files_expected_to_change.push(declared);
                }
                // Nothing the task forbids is lifted: validation refuses such
                // a grant (Batch O review).
            }
            ScopeGrantRoot::Project => {
                if !task.artifact_requirements.contains(&grant.path) {
                    task.artifact_requirements.push(grant.path.clone());
                }
            }
        }
    }
    amended
}

/// The run's amended universe, or `None` when the run amended nothing.
pub fn amended_universe_for_run(
    run_root: &Path,
    universe: &WorkflowV2TaskUniverse,
) -> Result<Option<WorkflowV2TaskUniverse>, ScopeAmendmentError> {
    let ledger = ScopeAmendmentLedger::load(run_root)?;
    Ok((!ledger.set.grants.is_empty()).then(|| amended_universe(universe, &ledger.set)))
}

/// The project-data paths granted to any of `task_ids`, project-relative,
/// sorted: each is a declared project artifact of their branches.
pub fn project_data_grants(set: &ScopeAmendmentSet, task_ids: &[String]) -> Vec<String> {
    let paths: BTreeSet<String> = set
        .grants_of(task_ids)
        .filter(|grant| grant.root == ScopeGrantRoot::Project)
        .map(|grant| grant.path.clone())
        .collect();
    paths.into_iter().collect()
}

/// One amendment transaction's inputs.
pub struct ScopeAmendmentRequest<'a> {
    pub run_root: &'a Path,
    pub universe: &'a WorkflowV2TaskUniverse,
    pub repository_root: &'a Path,
    pub grants: Vec<ScopeAmendment>,
    /// What asked for the amendment, recorded on its link.
    pub trigger: &'a str,
}

/// What one transaction did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScopeAmendmentOutcome {
    /// The digest of the set in force afterwards.
    pub digest: String,
    /// The link appended, or `None` when every grant was already in force.
    pub link: Option<ScopeAmendmentLink>,
    pub applied: Vec<ScopeAmendment>,
    /// Grants the host refused, each with its reason: never silently lost.
    pub refused: Vec<(ScopeAmendment, String)>,
}

/// Validate `request.grants`, then publish every valid one in one chained
/// step: the prior set filed by digest, the new set filed by digest, the
/// ledger replaced by rename, the decision appended to the log. Holds the
/// ledger lock throughout.
pub fn amend_task_scope(
    request: ScopeAmendmentRequest<'_>,
) -> Result<ScopeAmendmentOutcome, ScopeAmendmentError> {
    let _lock = LedgerLock::acquire(&ledger_path(request.run_root))?;
    let ledger = ScopeAmendmentLedger::load(request.run_root)?;
    let policy =
        crate::write_coordinator::project_inputs::ProjectInputPolicy::for_landing(request.run_root);
    let mut by_key: BTreeMap<(String, String), ScopeAmendment> = ledger
        .set
        .grants
        .iter()
        .map(|grant| ((grant.task_id.clone(), grant.path.clone()), grant.clone()))
        .collect();
    let (mut applied, mut refused) = (Vec::new(), Vec::new());
    for grant in request.grants {
        match validate::grant(
            request.universe,
            request.repository_root,
            policy.as_ref(),
            &grant,
        ) {
            Ok(valid) => {
                let key = (valid.task_id.clone(), valid.path.clone());
                if by_key.get(&key) != Some(&valid) {
                    by_key.insert(key, valid.clone());
                    applied.push(valid);
                }
            }
            Err(reason) => refused.push((grant, reason)),
        }
    }
    let prior = ledger.set.digest();
    let mut next = ScopeAmendmentSet {
        grants: by_key.into_values().collect(),
        ..ScopeAmendmentSet::default()
    };
    next.grants.sort_by(|a, b| a.key().cmp(&b.key()));
    let link = (!applied.is_empty()).then(|| ScopeAmendmentLink {
        from_digest: prior.clone(),
        to_digest: next.digest(),
        changed_task_ids: applied.iter().map(|grant| grant.task_id.clone()).collect(),
        trigger: request.trigger.to_string(),
        prior_link_digest: ledger.lineage.last().map(ScopeAmendmentLink::digest),
    });
    if let Some(link) = &link {
        let store = history(request.run_root);
        store.put(&ledger.set.bytes())?;
        store.put(&next.bytes())?;
        let mut lineage = ledger.lineage.clone();
        lineage.push(link.clone());
        let published = ScopeAmendmentLedger { set: next, lineage };
        published.verify(&store)?;
        write_atomic(
            &ledger_path(request.run_root),
            &serde_json::to_vec_pretty(&published).map_err(|err| error(err.to_string()))?,
        )?;
    }
    let outcome = ScopeAmendmentOutcome {
        digest: link.as_ref().map_or(prior, |link| link.to_digest.clone()),
        link,
        applied,
        refused,
    };
    append_log(request.run_root, request.trigger, &outcome)?;
    Ok(outcome)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), ScopeAmendmentError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let temp = parent.join(format!(
        ".scope-amendments.{}.new",
        uuid::Uuid::new_v4().simple()
    ));
    let written = std::fs::create_dir_all(parent)
        .and_then(|()| std::fs::write(&temp, bytes))
        .and_then(|()| std::fs::rename(&temp, path));
    written.map_err(|err| {
        let _ = std::fs::remove_file(&temp);
        error(format!("{} could not be written: {err}", path.display()))
    })
}

fn append_log(
    run_root: &Path,
    trigger: &str,
    outcome: &ScopeAmendmentOutcome,
) -> Result<(), ScopeAmendmentError> {
    use std::io::Write;
    let line = serde_json::json!({
        "at": chrono::Utc::now().to_rfc3339(),
        "trigger": trigger,
        "outcome": outcome,
    });
    let path = log_path(run_root);
    std::fs::create_dir_all(path.parent().unwrap_or(run_root))
        .and_then(|()| {
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
        })
        .and_then(|mut file| writeln!(file, "{line}"))
        .map_err(|err| error(format!("{} could not be appended: {err}", path.display())))
}

/// An exclusive lock beside the ledger, released on drop; one left by a
/// process that died more than ten minutes ago is broken.
struct LedgerLock(PathBuf);

impl LedgerLock {
    fn acquire(ledger: &Path) -> Result<Self, ScopeAmendmentError> {
        let path = ledger.with_extension("lock");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|err| error(err.to_string()))?;
        }
        for _ in 0..600 {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(_) => return Ok(Self(path)),
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                    let stale = std::fs::metadata(&path)
                        .and_then(|meta| meta.modified())
                        .ok()
                        .and_then(|at| at.elapsed().ok())
                        .is_some_and(|age| age.as_secs() > 600);
                    if stale {
                        let _ = std::fs::remove_file(&path);
                        continue;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                Err(err) => return Err(error(format!("{}: {err}", path.display()))),
            }
        }
        Err(error(format!(
            "{} is held by another amendment",
            path.display()
        )))
    }
}

impl Drop for LedgerLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[path = "task_scope_amendment_validate.rs"]
mod validate;

#[path = "task_scope_amendment_plan.rs"]
mod plan;
#[path = "task_scope_amendment_refs.rs"]
pub mod refs;
pub use plan::{ScopeAmendmentPlan, ScopePlanInputs, plan_scope_amendments};

#[cfg(test)]
#[path = "task_scope_amendment_tests.rs"]
mod tests;
