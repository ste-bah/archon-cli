//! Seed a write branch's worktree with the project's acceptance inputs,
//! and capture what the branch changed there (Batch E; the records and the
//! placement rule are `write_coordinator::project_inputs`, the landing is
//! `patch_apply::project_inputs_apply`).
//!
//! Seeded: every regular file under an input that the worktree's git
//! ignores and does not track, copied from the project root with its state
//! there recorded as the file's baseline. A path git carries is left to git
//! -- a tracked file is the repository's and lands as a patch -- and a path
//! git would see but does not carry is never seeded, so nothing seeded can
//! enter the patch. A link is never followed or copied. A host dependency
//! share on the way (`ignored_deps` mirrors an ignored directory by linking
//! its children into the canonical checkout) is replaced by a private
//! directory, so a seeded file is never written through into the checkout.
//! Nothing is seeded past the byte cap; what is not seeded is recorded and
//! told to the agent.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::task_set_contract::ACCEPTANCE_CONTRACT_FILE;
use crate::write_coordinator::project_inputs::{
    CaptureRecord, InputChange, ProjectInputPolicy, SeedRecord, capture_path, captured_bytes_dir,
    file_state, meta_state, read_json, read_no_follow, refuse_links, seed_path, write_file,
    write_json,
};
use crate::write_coordinator::worktree_isolation::{check_ignore, run_git};
use crate::{WorkflowError, WorkflowResult};

/// Most files read from one input tree; a tree past it is marked, never
/// silently cut.
const MAX_FILES: usize = 200_000;

fn io_error(path: &Path, error: std::io::Error) -> WorkflowError {
    WorkflowError::io(path, error)
}

fn git_error(error: impl std::fmt::Display) -> WorkflowError {
    WorkflowError::StageFailed(format!("project input seeding: {error}"))
}

/// Every regular file under `root/rel`, relative, sorted; links and other
/// non-files under it are recorded in `skipped` (never followed).
fn walk(
    root: &Path,
    rel: &Path,
    policy: &ProjectInputPolicy,
    out: &mut Vec<String>,
    skipped: &mut Vec<(String, String)>,
) {
    if policy.excluded(rel) || out.len() >= MAX_FILES {
        return;
    }
    let path = root.join(rel);
    let Ok(meta) = std::fs::symlink_metadata(&path) else {
        return;
    };
    let name = rel
        .to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, "/");
    if meta.is_dir() {
        let Ok(entries) = std::fs::read_dir(&path) else {
            skipped.push((name, "unreadable directory".into()));
            return;
        };
        let mut children: Vec<_> = entries.flatten().map(|e| e.file_name()).collect();
        children.sort();
        for child in children {
            walk(root, &rel.join(child), policy, out, skipped);
        }
    } else if meta.is_file() {
        out.push(name);
    } else {
        skipped.push((name, "not a regular file (a link is never followed)".into()));
    }
}

/// Batch I2: the run's frozen acceptance contract (under the task root the
/// run recorded), which no seeded file may shadow. A project input carrying
/// a file of its name that is not byte-identical to it -- a draft, a stale
/// copy -- is never copied into a worktree, where an agent would take it
/// for the contract the harness runs; it is recorded as excluded instead.
/// How a seed record marks a file excluded for shadowing the frozen
/// contract; the capture never lands a change to such a path.
pub(super) const SHADOWS_FROZEN_CONTRACT: &str =
    "excluded: it shadows the run's frozen acceptance contract";

struct FrozenContract {
    path: PathBuf,
    bytes: Option<Vec<u8>>,
}

impl FrozenContract {
    fn of(policy: &ProjectInputPolicy) -> Self {
        let path = policy.task_root.join(ACCEPTANCE_CONTRACT_FILE);
        let bytes = std::fs::read(&path).ok();
        Self { path, bytes }
    }

    /// Why `rel` (with `bytes`) may not be seeded, when it shadows it.
    fn shadowed_by(&self, rel: &str, bytes: &[u8]) -> Option<String> {
        // Case-blind: on a case-insensitive filesystem the names are one file.
        let name = |path: &Path| path.file_name().map(|n| n.to_string_lossy().to_lowercase());
        if name(Path::new(rel)) != name(&self.path) {
            return None;
        }
        match &self.bytes {
            Some(frozen) if frozen.as_slice() == bytes => None,
            Some(_) => Some(format!(
                "{SHADOWS_FROZEN_CONTRACT} {} (same name, different bytes); only the frozen contract is authoritative, and nothing written at this path ever lands",
                self.path.display()
            )),
            None => Some(format!(
                "{SHADOWS_FROZEN_CONTRACT} {}, which cannot be read to prove it identical; nothing written at this path ever lands",
                self.path.display()
            )),
        }
    }
}

/// The inputs' files the worktree's git ignores and does not track.
fn git_free(worktree: &Path, files: &[String]) -> WorkflowResult<BTreeSet<String>> {
    Ok(check_ignore(worktree, files)
        .map_err(git_error)?
        .into_iter()
        .collect())
}

/// Make every directory on the way to `rel` in the worktree a private real
/// directory: an ignored, untracked link there is a host share and is
/// removed; any other link or a file in the way refuses the path.
fn private_parents(worktree: &Path, rel: &str) -> Result<(), String> {
    let mut cursor = PathBuf::new();
    let parts: Vec<&str> = rel.split('/').collect();
    for (at, part) in parts.iter().enumerate() {
        cursor.push(part);
        let path = worktree.join(&cursor);
        match std::fs::symlink_metadata(&path) {
            Ok(meta) if meta.file_type().is_symlink() => {
                let spelled = cursor
                    .to_string_lossy()
                    .replace(std::path::MAIN_SEPARATOR, "/");
                let share = check_ignore(worktree, std::slice::from_ref(&spelled))
                    .map(|found| !found.is_empty())
                    .unwrap_or(false);
                if !share {
                    return Err(format!("{spelled} is a link git carries"));
                }
                std::fs::remove_file(&path).map_err(|e| e.to_string())?;
            }
            Ok(meta) if at + 1 < parts.len() && !meta.is_dir() => {
                return Err(format!("{} is not a directory", cursor.display()));
            }
            _ => {}
        }
    }
    Ok(())
}

/// Remove everything under the inputs in `worktree` that its git ignores
/// and does not track -- files and links alike, never following a link.
fn reset(worktree: &Path, policy: &ProjectInputPolicy) -> WorkflowResult<()> {
    for input in &policy.inputs {
        let rel = input
            .to_string_lossy()
            .replace(std::path::MAIN_SEPARATOR, "/");
        if policy.excluded(input) || private_parents(worktree, &rel).is_err() {
            continue;
        }
        let (mut files, mut others) = (Vec::new(), Vec::new());
        walk(worktree, input, policy, &mut files, &mut others);
        files.extend(others.into_iter().map(|(path, _)| path).filter(|path| {
            std::fs::symlink_metadata(worktree.join(path)).is_ok_and(|m| m.file_type().is_symlink())
        }));
        for path in git_free(worktree, &files)? {
            let target = worktree.join(&path);
            std::fs::remove_file(&target).map_err(|e| io_error(&target, e))?;
        }
    }
    Ok(())
}

/// Seed `worktree` for (`stage_id`, `item_id`) and record it; `None` when
/// the run records no project inputs. Seeding again first removes what the
/// last seed and anything run since left, so it can be repeated.
#[cfg(test)]
pub(super) fn seed(
    run_root: &Path,
    stage_id: &str,
    item_id: &str,
    worktree: &Path,
) -> WorkflowResult<Option<SeedRecord>> {
    seed_with(run_root, (stage_id, item_id), worktree, None)
}

/// [`seed`], and -- Batch G2 -- the branch's copy of each project artifact
/// its call `declared` outside the inputs (`declared::seed_declared`): `None`
/// only when there is neither.
pub(super) fn seed_with(
    run_root: &Path,
    ids: (&str, &str),
    worktree: &Path,
    declared: Option<&declared::DeclaredArtifacts<'_>>,
) -> WorkflowResult<Option<SeedRecord>> {
    let (stage_id, item_id) = ids;
    let seed_file = seed_path(run_root, stage_id, item_id);
    let staging = crate::write_coordinator::project_inputs::staging_dir(worktree);
    if staging.symlink_metadata().is_ok() {
        std::fs::remove_dir_all(&staging).map_err(|e| io_error(&staging, e))?;
    }
    let wants_declared = declared.is_some_and(|declared| !declared.rels.is_empty());
    let policy = ProjectInputPolicy::for_run(run_root).or_else(|| {
        wants_declared
            .then(|| ProjectInputPolicy::for_landing(run_root))
            .flatten()
    });
    let Some(policy) = policy else {
        let _ = std::fs::remove_file(&seed_file);
        return Ok(None);
    };
    let mut record = SeedRecord {
        project: policy.project.clone(),
        worktree: worktree.to_path_buf(),
        ..SeedRecord::default()
    };
    // Anything already there that git does not carry -- a host dependency
    // share, or what a command run since the last seed wrote -- goes first:
    // the worktree holds exactly the project's data, as acceptance does.
    reset(worktree, &policy)?;
    let frozen = FrozenContract::of(&policy);
    let mut budget = policy.limit;
    for input in &policy.inputs {
        let rel = input
            .to_string_lossy()
            .replace(std::path::MAIN_SEPARATOR, "/");
        if policy.excluded(input) {
            record
                .skipped
                .push((rel, "excluded from project inputs".into()));
            continue;
        }
        record.inputs.push(rel.clone());
        // Whole-ignored only when git says so plainly; anything it cannot
        // answer (a link on the way it will not look beyond) is not.
        if let Err(why) = private_parents(worktree, &rel) {
            record.skipped.push((rel.clone(), why));
        }
        let untracked = run_git(&["ls-files", "-z", "--", &rel], worktree)
            .is_ok_and(|output| output.stdout.is_empty());
        let whole = [rel.clone(), format!("{rel}/")];
        if untracked && git_free(worktree, &whole).is_ok_and(|free| !free.is_empty()) {
            record.ignored_roots.push(rel.clone());
        }
        let mut files = Vec::new();
        if let Err(error) = refuse_links(&policy.project, &policy.project.join(input)) {
            record
                .skipped
                .push((rel.clone(), format!("not read: {error}")));
            continue;
        }
        walk(
            &policy.project,
            input,
            &policy,
            &mut files,
            &mut record.skipped,
        );
        if files.len() >= MAX_FILES {
            record.truncated = true;
            record.skipped.push((
                rel.clone(),
                format!("more than {MAX_FILES} files: the rest were not copied and cannot land"),
            ));
        }
        // Private directories first: git answers nothing about a path beyond
        // a link, and a host share on the way is replaced, never followed.
        let mut candidates = Vec::new();
        for file in files {
            let parent = Path::new(&file)
                .parent()
                .map(|p| p.to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/"));
            match parent
                .filter(|p| !p.is_empty())
                .map(|p| private_parents(worktree, &p))
            {
                Some(Err(why)) => {
                    let state = meta_state(&policy.project.join(&file));
                    record.unseeded.insert(file.clone(), state);
                    record.skipped.push((file, why));
                }
                _ => candidates.push(file),
            }
        }
        let free = git_free(worktree, &candidates)?;
        for file in candidates {
            if !free.contains(&file) {
                record.skipped.push((
                    file,
                    "the repository tracks it or would see it: git carries it, not the seed".into(),
                ));
                continue;
            }
            let source = policy.project.join(&file);
            let size = std::fs::symlink_metadata(&source).map_or(u64::MAX, |m| m.len());
            // Not copied, but its state is still the baseline a landing of it
            // must find: a branch that regenerates it can land it.
            let mut unseeded = |file: String, why: &str| {
                record.unseeded.insert(file.clone(), meta_state(&source));
                record.skipped.push((file, why.to_string()));
            };
            if size > budget {
                unseeded(file, "over the project input size cap");
                continue;
            }
            let Ok(bytes) = read_no_follow(&source) else {
                unseeded(file, "unreadable in the project root");
                continue;
            };
            if let Some(why) = frozen.shadowed_by(&file, &bytes) {
                unseeded(file, &why);
                continue;
            }
            if let Err(why) = private_parents(worktree, &file) {
                unseeded(file, &why);
                continue;
            }
            let destination = worktree.join(&file);
            write_file(worktree, &destination, &bytes).map_err(|e| io_error(&destination, e))?;
            budget = budget.saturating_sub(bytes.len() as u64);
            let state = blake3::hash(&bytes).to_hex().to_string();
            record.files.insert(file, state);
        }
    }
    if let Some(declared) = declared {
        declared::seed_declared(&policy, &mut record, declared, &staging, &mut budget)?;
    }
    if record.inputs.is_empty() && record.declared.is_empty() {
        let _ = std::fs::remove_file(&seed_file);
        return Ok(None);
    }
    write_json(&seed_file, &record).map_err(|e| io_error(&seed_file, e))?;
    Ok(Some(record))
}

/// The worktree paths the branch may write as project data: each input its
/// git ignores whole, and each seeded file.
pub(super) fn writable(record: &SeedRecord) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = record
        .ignored_roots
        .iter()
        .map(|root| record.worktree.join(root))
        .collect();
    for file in record.files.keys() {
        if !record
            .ignored_roots
            .iter()
            .any(|root| Path::new(file).starts_with(root))
        {
            paths.push(record.worktree.join(file));
        }
    }
    paths.extend(declared::writable(record));
    paths
}

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
pub(super) struct InputCapture {
    /// The changed paths kept for the landing.
    pub(super) changed: Vec<String>,
    /// Changes left out, and why (a forbidden path, not a regular file).
    pub(super) dropped: Vec<(String, String)>,
    /// Why nothing was kept at all (over a cap); the patch is unaffected.
    pub(super) refused: Option<String>,
}

/// Capture what the branch changed under its seeded inputs -- judged
/// against each file's seed baseline, never through a link, never a path
/// its tasks forbid -- keep the bytes for the landing and record them.
/// Nothing is recorded when the branch was not seeded, changed nothing, or
/// changed more than can land.
pub(super) fn capture(
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
pub(super) fn discard(run_root: &Path, stage_id: &str, item_id: &str) {
    let _ = std::fs::remove_file(capture_path(run_root, stage_id, item_id));
    let _ = std::fs::remove_dir_all(captured_bytes_dir(run_root, stage_id, item_id));
}

#[path = "project_inputs_seed_declared.rs"]
pub(super) mod declared;

#[cfg(test)]
#[path = "project_inputs_seed_tests.rs"]
mod tests;
