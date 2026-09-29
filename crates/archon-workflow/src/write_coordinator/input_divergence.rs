//! Batch G: the host puts back a tracked project input that diverged from
//! the repository's copy when nothing it recorded explains the change.
//!
//! Acceptance scratch in the combined view overlays the project's inputs on
//! the repository at the round's commit and refuses a path the two give
//! different bytes ("nonidentical scratch path collision"). A project copy
//! diverges that way only when something rewrote it outside the host's
//! landings (a landing that changes a tracked input syncs the project's copy,
//! `project_inputs_apply::sync_tracked`). Live, it was a read-only verifier;
//! the whole round then failed and reached the tasks. That is the host's
//! environment, so the host repairs it where it deterministically can: the
//! diverged copy is kept, the repository's tracked copy at the round's commit
//! is written in its place, and the decision is logged. A copy a recorded
//! landing, materialization or write call's declared delivery put there is
//! left alone and named, for a person.
//!
//! Only the acceptance stage's own tip is repaired: a regression probe at an
//! older commit legitimately sees older tracked copies.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;

use super::input_tripwire::host_write_section;
use super::patch_apply::{run_materializations, run_project_input_landings};
use super::project_inputs::{file_state, read_no_follow, write_file};
use crate::acceptance_scratch::{ScratchPolicy, project_input_excluded};

/// One tracked input whose project copy differs from the repository's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InputDivergence {
    pub path: String,
    pub project_state: String,
    pub tracked_state: String,
    /// Where the diverged copy was kept, when it was restored.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kept_at: Option<PathBuf>,
    pub restored: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub reason: String,
}

impl InputDivergence {
    pub fn describe(&self, commit: &str) -> String {
        if self.restored {
            format!(
                "host restored project input {} to the repository's tracked copy at {commit}: the project copy ({}) had diverged with no recorded landing; the diverged copy is kept at {}",
                self.path,
                short(&self.project_state),
                self.kept_at
                    .as_ref()
                    .map_or_else(String::new, |p| p.display().to_string())
            )
        } else {
            format!(
                "project input {} differs from the repository's tracked copy at {commit} and was NOT restored: {}",
                self.path, self.reason
            )
        }
    }
}

fn short(state: &str) -> &str {
    state.get(..12).unwrap_or(state)
}

fn git(repository: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(args)
        .output()
        .map_err(|e| format!("git {}: {e}", args.join(" ")))?;
    if !output.status.success() {
        return Err(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(output.stdout)
}

/// Every tracked input under the policy's inputs whose project copy differs
/// from the repository's copy at `commit`, restored unless a landing this
/// run recorded (`run_root`'s project-input log) put that copy there.
/// Nothing to do outside the combined view: no collision is possible.
pub fn restore_diverged_tracked_inputs(
    run_root: &Path,
    policy: &ScratchPolicy,
    commit: &str,
) -> Result<Vec<InputDivergence>, String> {
    if !policy.combined || policy.project_inputs.is_empty() {
        return Ok(Vec::new());
    }
    let project = policy.project.canonicalize().map_err(|e| e.to_string())?;
    let tasks = policy
        .task_root
        .canonicalize()
        .unwrap_or_else(|_| policy.task_root.clone());
    let mut args = vec!["ls-tree", "-r", "-z", "--name-only", commit, "--"];
    let inputs: Vec<String> = (policy.project_inputs.iter())
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    args.extend(inputs.iter().map(String::as_str));
    let listed = git(&policy.repository, &args)?;
    let ledger = run_project_input_landings(run_root)?;
    let copies = run_materializations(run_root)?;
    let delivered = super::input_tripwire::delivered_inputs(run_root);
    let mut out = Vec::new();
    let _section = host_write_section();
    for rel in listed.split(|b| *b == 0).filter(|name| !name.is_empty()) {
        let rel = String::from_utf8_lossy(rel).into_owned();
        let relative = Path::new(&rel);
        if project_input_excluded(relative, &policy.project_input_excludes)
            || !relative
                .components()
                .all(|c| matches!(c, std::path::Component::Normal(_)))
        {
            continue;
        }
        let destination = project.join(relative);
        // The task set is never written by the host's input repairs.
        if destination.starts_with(&tasks) {
            continue;
        }
        let project_state = file_state(&destination);
        if project_state == "absent" {
            continue;
        }
        let tracked = git(
            &policy.repository,
            &["cat-file", "blob", &format!("{commit}:{rel}")],
        )?;
        let tracked_state = blake3::hash(&tracked).to_hex().to_string();
        if project_state == tracked_state {
            continue;
        }
        let mut divergence = InputDivergence {
            path: rel.clone(),
            project_state: project_state.clone(),
            tracked_state,
            kept_at: None,
            restored: false,
            reason: String::new(),
        };
        // Batch L: a refused landing the host put back is the host's copy too.
        let landed = ledger
            .iter()
            .rev()
            .find(|line| line.path == rel && (line.landed() || line.reverted()));
        let placed = landed
            .filter(|line| line.after == project_state)
            .map(|line| (line.stage_id.clone(), line.item_id.clone()))
            .or_else(|| {
                (copies.iter().rev())
                    .find(|copy| copy.path == rel && copy.receipt.post_hash == project_state)
                    .map(|copy| (copy.stage_id.clone(), copy.item_id.clone()))
            })
            .or_else(|| {
                (delivered.iter())
                    .any(|(path, after)| *path == rel && *after == project_state)
                    .then(|| {
                        (
                            "a write call".to_string(),
                            "its declared delivery".to_string(),
                        )
                    })
            });
        if let Some((stage, item)) = placed {
            divergence.reason = format!(
                "landing {stage}/{item} recorded putting this copy there; a person must decide which copy is right"
            );
            out.push(divergence);
            continue;
        }
        match keep_and_restore(run_root, &project, &destination, &rel, &tracked) {
            Ok(kept) => {
                divergence.kept_at = Some(kept);
                divergence.restored = true;
            }
            Err(why) => divergence.reason = why,
        }
        out.push(divergence);
    }
    drop(_section);
    let _ = log(run_root, commit, &out);
    Ok(out)
}

fn keep_and_restore(
    run_root: &Path,
    project: &Path,
    destination: &Path,
    rel: &str,
    tracked: &[u8],
) -> Result<PathBuf, String> {
    let stamp = chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default();
    let kept = run_root
        .join("write-coordination")
        .join("project-inputs-restored")
        .join(stamp.to_string())
        .join(rel);
    let current =
        read_no_follow(destination).map_err(|e| format!("its current copy cannot be read: {e}"))?;
    kept.parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(&kept, &current))
        .map_err(|e| format!("its current copy could not be kept: {e}"))?;
    write_file(project, destination, tracked).map_err(|e| format!("the restore failed: {e}"))?;
    Ok(kept)
}

fn log(run_root: &Path, commit: &str, divergences: &[InputDivergence]) -> std::io::Result<()> {
    use std::io::Write;
    if divergences.is_empty() {
        return Ok(());
    }
    let path = run_root
        .join("write-coordination")
        .join("project-inputs-restored.jsonl");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    for divergence in divergences {
        let mut line = serde_json::to_vec(&serde_json::json!({
            "at": chrono::Utc::now().to_rfc3339(),
            "commit": commit,
            "divergence": divergence,
        }))?;
        line.push(b'\n');
        file.write_all(&line)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "input_divergence_tests.rs"]
mod tests;
