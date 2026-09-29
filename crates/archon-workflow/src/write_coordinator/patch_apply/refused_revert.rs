//! Batch L: put back a project-data landing a verdict of its unit refused.
//!
//! A landing moves project data two ways besides its commit: the project
//! inputs it applied or kept in step (`project_inputs_apply`, logged in
//! `project-inputs.jsonl`) and the declared ignored deliverables it copied
//! where they are verified (`materialize`, logged in `materializations.jsonl`).
//! Each logged line names the state the landing found and the one it left.
//! A refused line is put back only while the project still holds exactly
//! what that line left: anything else there is a later change that stands,
//! and overwriting it would destroy it, so that is a conflict for a person,
//! never a silent keep and never a silent overwrite.
//!
//! The replaced bytes come from what the landing kept: its first kept copy
//! under `project-inputs-replaced/<stage>/<item>/`, or the run's content
//! store of every project-input state it has seen (`input_tripwire`), both
//! checked against the logged hash. Every put-back is logged in the same
//! ledger the landing wrote, so a resume, the divergence repair and the
//! reuse checks read the restored state as the host's own.

use std::path::{Path, PathBuf};

use super::materialize_ledger::{RunMaterialization, append_revert, run_materializations};
use super::project_inputs_ledger::{ProjectInputLanding, append};
use crate::write_coordinator::input_tripwire::{host_write_section, kept_object, remove_input};
use crate::write_coordinator::project_inputs::{
    ProjectInputPolicy, file_state, read_no_follow, write_file,
};

/// What putting one refused data landing back did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DataRevert {
    /// Put back: the state taken out, and the one restored.
    Reverted { from: String, to: String },
    /// The project already held what the landing replaced.
    Already,
    /// Not put back, and why: the landing stays in place.
    Conflict(String),
}

fn now() -> i64 {
    chrono::Utc::now().timestamp_nanos_opt().unwrap_or(i64::MAX)
}

/// Two recorded states agree: `absent` and `deleted` both mean no file.
fn same(left: &str, right: &str) -> bool {
    let none = |state: &str| matches!(state, "absent" | "deleted");
    left == right || (none(left) && none(right))
}

/// The bytes of state `state` for `rel`, from what the run kept.
fn replaced_bytes(
    run_root: &Path,
    stage: &str,
    item: &str,
    rel: &str,
    state: &str,
) -> Option<Vec<u8>> {
    let kept: PathBuf = run_root
        .join("write-coordination")
        .join("project-inputs-replaced")
        .join(stage)
        .join(item)
        .join(rel);
    if let Ok(bytes) = read_no_follow(&kept) {
        let fits = match state.strip_prefix("meta:") {
            Some(meta) => meta.split(':').next() == Some(bytes.len().to_string().as_str()),
            None => blake3::hash(&bytes).to_hex().to_string() == state,
        };
        if fits {
            return Some(bytes);
        }
    }
    kept_object(run_root, state)
}

/// Put `destination` back to `state` (under `root`).
fn restore(
    root: &Path,
    destination: &Path,
    state: &str,
    bytes: Option<Vec<u8>>,
) -> Result<(), String> {
    if matches!(state, "absent" | "deleted") {
        return match remove_input(destination) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.to_string()),
        };
    }
    let bytes = bytes.ok_or_else(|| {
        format!("the state it replaced ({state}) was not kept by the run, so it cannot be restored")
    })?;
    write_file(root, destination, &bytes)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

/// Put back one refused project-input landing `line` (an `applied` or a
/// `synced` line), logging the decision with `why`.
pub(crate) fn revert_input(run_root: &Path, line: &ProjectInputLanding, why: &str) -> DataRevert {
    let Some(policy) = ProjectInputPolicy::recorded(run_root) else {
        return DataRevert::Conflict("the run's project input policy cannot be read".into());
    };
    let destination = match policy.placed(&line.path, true) {
        Ok(destination) => destination,
        Err(why) => return DataRevert::Conflict(format!("{}: {why}", line.path)),
    };
    let _section = host_write_section();
    let current = file_state(&destination);
    let outcome = if same(&current, &line.before) {
        DataRevert::Already
    } else if current != line.after {
        return DataRevert::Conflict(format!(
            "{} holds {current}, not what the landing left ({}): a later change stands there",
            line.path, line.after
        ));
    } else {
        let bytes = replaced_bytes(
            run_root,
            &line.stage_id,
            &line.item_id,
            &line.path,
            &line.before,
        );
        if let Err(error) = restore(&policy.project, &destination, &line.before, bytes) {
            return DataRevert::Conflict(format!("{}: {error}", line.path));
        }
        DataRevert::Reverted {
            from: current.clone(),
            to: line.before.clone(),
        }
    };
    let logged = ProjectInputLanding {
        outcome: "reverted".into(),
        before: current,
        after: file_state(&destination),
        reason: why.to_string(),
        at: now(),
        ..line.clone()
    };
    if let Err(error) = append(run_root, &[logged]) {
        return DataRevert::Conflict(format!(
            "{}: restored, but the project input log could not be written: {error}",
            line.path
        ));
    }
    outcome
}

/// Put back one refused materialized copy `entry`, logging the decision in
/// the run's copy order.
pub(crate) fn revert_copy(run_root: &Path, entry: &RunMaterialization, why: &str) -> DataRevert {
    let Some(root) = super::materialize_scope::project_root(run_root) else {
        return DataRevert::Conflict("the run has no project root".into());
    };
    let root = PathBuf::from(root);
    let destination = PathBuf::from(&entry.receipt.destination);
    if !destination.starts_with(&root) {
        return DataRevert::Conflict(format!(
            "{} is outside the project root",
            destination.display()
        ));
    }
    let _section = host_write_section();
    let current = file_state(&destination);
    let before = &entry.receipt.pre_hash;
    let outcome = if same(&current, before) {
        DataRevert::Already
    } else if current != entry.receipt.post_hash {
        return DataRevert::Conflict(format!(
            "{} holds {current}, not what the copy left ({}): a later change stands there",
            destination.display(),
            entry.receipt.post_hash
        ));
    } else {
        let bytes = kept_object(run_root, before);
        if let Err(error) = restore(&root, &destination, before, bytes) {
            return DataRevert::Conflict(format!("{}: {error}", destination.display()));
        }
        DataRevert::Reverted {
            from: current.clone(),
            to: before.clone(),
        }
    };
    let sequence = match run_materializations(run_root) {
        Ok(ledger) => ledger.iter().map(|e| e.receipt.sequence).max().unwrap_or(0) + 1,
        Err(error) => {
            return DataRevert::Conflict(format!("the copy ledger is unreadable: {error}"));
        }
    };
    let restored = match file_state(&destination).as_str() {
        "absent" => "deleted".to_string(),
        state => state.to_string(),
    };
    let mut line = entry.clone();
    line.receipt.pre_hash = current;
    line.receipt.post_hash = restored;
    line.receipt.sequence = sequence;
    line.at = now();
    line.reverted = true;
    if let Err(error) = append_revert(run_root, &line) {
        return DataRevert::Conflict(format!(
            "{}: restored, but the copy ledger could not be written ({why}): {error}",
            destination.display()
        ));
    }
    outcome
}

#[cfg(test)]
#[path = "refused_revert_tests.rs"]
mod tests;
