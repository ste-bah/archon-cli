//! Issue-25: at dispatch, a tree an apply receipt already explains is the
//! post-apply audit — the one a pause interrupted, or a crash between apply
//! and audit skipped — never an unexpected change.
//!
//! Live on wf-719ff3b0, `repository-audit-7` (post_apply) was paused
//! mid-assessment; the resumed dispatch compared the audit state's snapshot
//! (still the pre-apply tree) with the sealed source (the wave's own
//! outcome, recorded as `after` in `apply-agents-4-0.json`) and charged the
//! difference to the unexpected-change allowance. Three such charges paused
//! the run.
use super::*;
use crate::repository_audit::receipts::ApplyReceipt;
use crate::repository_audit::runtime::Snapshot;

pub(super) struct RefreshTrigger {
    pub(super) trigger: &'static str,
    /// The receipt that explains the new tree, when `trigger` is `post_apply`.
    pub(super) receipt: Option<ApplyReceipt>,
}

impl RefreshTrigger {
    fn plain(trigger: &'static str) -> Self {
        Self {
            trigger,
            receipt: None,
        }
    }
    fn post_apply(receipt: &ApplyReceipt) -> Self {
        Self {
            trigger: "post_apply",
            receipt: Some(receipt.clone()),
        }
    }
    /// What the started event should carry beyond the trigger.
    pub(super) fn event_detail(&self) -> serde_json::Value {
        match &self.receipt {
            Some(receipt) => serde_json::json!({
                "apply_receipt": {"commit": receipt.commit, "before": receipt.before, "after": receipt.after,
                    "items_applied": receipt.items_applied},
                "unexpected_paths": receipt.unexpected_paths,
            }),
            None => serde_json::json!({}),
        }
    }
}

/// `initial` with no audited snapshot yet, `dispatch` when the tree is the
/// audited one, `post_apply` when a receipt's `after` is this tree or every
/// path that differs from the audited snapshot is accounted for by a
/// receipt's applied manifests, and `unexpected_change` otherwise.
pub(super) fn refresh_trigger(
    previous: Option<&Snapshot>,
    current: &Snapshot,
    receipts: &[ApplyReceipt],
    run_root: &Path,
    canonical_root: &Path,
) -> WorkflowResult<RefreshTrigger> {
    let Some(previous) = previous else {
        return Ok(RefreshTrigger::plain("initial"));
    };
    if previous.identity == current.identity {
        return Ok(RefreshTrigger::plain("dispatch"));
    }
    if let Some(receipt) = receipts
        .iter()
        .rev()
        .find(|receipt| receipt.after == current.identity)
    {
        return Ok(RefreshTrigger::post_apply(receipt));
    }
    let mut changed: Option<Vec<String>> = None;
    for receipt in receipts.iter().rev() {
        let manifests = applied_manifests(run_root, receipt)?;
        if manifests.is_empty() {
            continue;
        }
        if changed.is_none() {
            changed = Some(changed_paths(previous, current)?);
        }
        let differs = changed.as_deref().unwrap_or_default();
        let landed = super::audit_wave::LandedCommit {
            repo: canonical_root,
            commit: &receipt.commit,
        };
        if !differs.is_empty()
            && differs.iter().all(|path| {
                manifests
                    .iter()
                    .any(|m| super::audit_wave::patch_accounts_for(m, path, current, &landed))
            })
        {
            return Ok(RefreshTrigger::post_apply(receipt));
        }
    }
    Ok(RefreshTrigger::plain("unexpected_change"))
}

fn changed_paths(previous: &Snapshot, current: &Snapshot) -> WorkflowResult<Vec<String>> {
    let old = previous.content_index()?;
    let new = current.content_index()?;
    Ok(old
        .keys()
        .chain(new.keys())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|path| old.get(*path) != new.get(*path))
        .cloned()
        .collect())
}

/// The receipt's applied manifests that are still on disk. A receipt written
/// before `call_id` was recorded names no manifest path and yields none.
fn applied_manifests(
    run_root: &Path,
    receipt: &ApplyReceipt,
) -> WorkflowResult<Vec<PatchManifest>> {
    let mut manifests = Vec::new();
    if receipt.call_id.is_empty() {
        return Ok(manifests);
    }
    for item_id in &receipt.items_applied {
        let path = PathBuf::from(manifest_path_for(run_root, &receipt.call_id, item_id));
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(WorkflowError::io(&path, error)),
        };
        let manifest: PatchManifest = serde_json::from_slice(&bytes)?;
        if manifest.status == ManifestStatus::Applied {
            manifests.push(manifest);
        }
    }
    Ok(manifests)
}
