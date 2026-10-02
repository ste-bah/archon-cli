//! Capture what a write branch changed under its seeded project inputs and
//! its declared project artifacts, and keep the bytes for the landing
//! (`project_inputs_seed` seeds them; `patch_apply::project_inputs_apply`
//! lands them).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use super::{MAX_FILES, declared, git_free, io_error, walk};
use crate::WorkflowResult;
use crate::task_set_contract::ACCEPTANCE_CONTRACT_FILE;
use crate::write_coordinator::project_inputs::{
    CaptureRecord, InputChange, ProjectInputPolicy, SeedRecord, capture_path, captured_bytes_dir,
    file_state, read_json, read_no_follow, refuse_links, seed_path, write_json,
};

/// Why a change to `rel` can never land, when it cannot: a path the
/// branch's tasks forbid, or one no landing may write (`destination`).
/// Left out with a gap, so it never refuses the rest of the landing.
fn unlandable(
    policy: &ProjectInputPolicy,
    forbidden: &archon_write_plan::ForbiddenPaths,
    rel: &str,
) -> Option<String> {
    if forbidden.matches(rel) {
        return Some("a path the branch's tasks forbid".into());
    }
    policy
        .destination(rel)
        .err()
        .map(|why| format!("no landing writes it: {why}"))
}

/// Why a change to `rel` never lands when it bears the frozen contract's
/// name (case-blind): a landing there would put a divergent copy of the
/// contract in the project -- a stale one edited, an identical one changed,
/// or a new one -- for the next agent to take for the real one.
fn shadowing(rel: &str) -> Option<String> {
    Path::new(rel)
        .file_name()
        .is_some_and(|name| {
            name.to_string_lossy()
                .eq_ignore_ascii_case(ACCEPTANCE_CONTRACT_FILE)
        })
        .then(|| {
            "named like the frozen acceptance contract, which only the freeze writes: never landed"
                .into()
        })
}

/// What a capture kept for the landing, and what it did not.
#[derive(Debug, Default)]
pub(in crate::v2::write) struct InputCapture {
    /// The changed paths kept for the landing.
    pub(in crate::v2::write) changed: Vec<String>,
    /// Changes left out, and why (a forbidden path, not a regular file).
    pub(in crate::v2::write) dropped: Vec<(String, String)>,
    /// Why nothing was kept at all (over a cap); the patch is unaffected.
    pub(in crate::v2::write) refused: Option<String>,
}

/// Capture what the branch changed under its seeded inputs -- judged
/// against each file's seed baseline, never through a link, never a path
/// its tasks forbid -- keep the bytes for the landing and record them.
/// Nothing is recorded when the branch was not seeded, changed nothing, or
/// changed more than can land.
pub(in crate::v2::write) fn capture(
    run_root: &Path,
    ids: (&str, &str),
    worktree: &Path,
    task_ids: &[String],
    forbidden: &archon_write_plan::ForbiddenPaths,
) -> WorkflowResult<InputCapture> {
    let (stage_id, item_id) = ids;
    let record_file = capture_path(run_root, stage_id, item_id);
    let bytes_dir = captured_bytes_dir(run_root, stage_id, item_id);
    discard(run_root, stage_id, item_id);
    let mut out = InputCapture::default();
    let (Some(seed), Some(policy)) = (
        read_json::<SeedRecord>(&seed_path(run_root, stage_id, item_id)),
        ProjectInputPolicy::for_landing(run_root),
    ) else {
        return Ok(out);
    };
    let (mut present, mut others) = (Vec::new(), Vec::new());
    for input in &seed.inputs {
        if let Err(error) = refuse_links(worktree, &worktree.join(input)) {
            out.dropped
                .push((input.clone(), format!("not read: {error}")));
            continue;
        }
        let mut files = Vec::new();
        walk(worktree, Path::new(input), &policy, &mut files, &mut others);
        if files.len() >= MAX_FILES {
            let why = format!("more than {MAX_FILES} files: changes past them were not read");
            out.dropped.push((input.clone(), why));
        }
        present.extend(files);
    }
    out.dropped.extend(others);
    let free = git_free(worktree, &present)?;
    let mut changes: BTreeMap<String, InputChange> = BTreeMap::new();
    let mut total = 0u64;
    for rel in present.iter().filter(|rel| free.contains(*rel)) {
        let post = file_state(&worktree.join(rel));
        let baseline = seed.baseline(rel);
        if post == baseline {
            continue;
        }
        if let Some(why) = unlandable(&policy, forbidden, rel).or_else(|| shadowing(rel)) {
            out.dropped.push((rel.clone(), why));
            continue;
        }
        let bytes =
            read_no_follow(&worktree.join(rel)).map_err(|e| io_error(&worktree.join(rel), e))?;
        total += bytes.len() as u64;
        if total > policy.limit {
            discard(run_root, stage_id, item_id);
            out.refused = Some(format!(
                "the project input changes exceed the {} byte cap (at {rel})",
                policy.limit
            ));
            return Ok(out);
        }
        let kept = bytes_dir.join(rel);
        if let Some(parent) = kept.parent() {
            std::fs::create_dir_all(parent).map_err(|e| io_error(parent, e))?;
        }
        std::fs::write(&kept, &bytes).map_err(|e| io_error(&kept, e))?;
        changes.insert(rel.clone(), InputChange { baseline, post });
    }
    // Only a seeded file that is GONE was deleted: one the agent replaced by
    // a link or anything else is reported, and the project's copy stays.
    let present: BTreeSet<&String> = present.iter().collect();
    for (rel, baseline) in &seed.files {
        let gone = matches!(
            std::fs::symlink_metadata(worktree.join(rel)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound
        );
        if present.contains(rel) || !gone {
            continue;
        }
        if let Some(why) = unlandable(&policy, forbidden, rel) {
            out.dropped.push((rel.clone(), why));
            continue;
        }
        let post = "deleted".to_string();
        changes.insert(
            rel.clone(),
            InputChange {
                baseline: baseline.clone(),
                post,
            },
        );
    }
    // Batch G2: the branch's copies of its declared project artifacts.
    let mut declared_changed = BTreeSet::new();
    if let Some(refused) = declared::capture_declared(
        &policy,
        &seed,
        forbidden,
        (&bytes_dir, &mut total),
        &mut changes,
        &mut declared_changed,
        &mut out.dropped,
    )? {
        discard(run_root, stage_id, item_id);
        out.refused = Some(refused);
        return Ok(out);
    }
    if changes.is_empty() {
        return Ok(out);
    }
    out.changed = changes.keys().cloned().collect();
    let record = CaptureRecord {
        task_ids: task_ids.to_vec(),
        changes,
        declared: declared_changed,
    };
    write_json(&record_file, &record).map_err(|e| io_error(&record_file, e))?;
    Ok(out)
}

/// Forget a capture whose branch produced no manifest: nothing of it lands.
pub(in crate::v2::write) fn discard(run_root: &Path, stage_id: &str, item_id: &str) {
    let _ = std::fs::remove_file(capture_path(run_root, stage_id, item_id));
    let _ = std::fs::remove_dir_all(captured_bytes_dir(run_root, stage_id, item_id));
}
