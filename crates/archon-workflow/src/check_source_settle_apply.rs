//! Applying a settled check-source request (PLAN-11,
//! [`super`]): an accepted held change written and committed and its checks
//! re-pinned; a refused tree change restored to its pinned bytes.

use std::path::Path;

use super::Settle;
use crate::check_source_pins::{CheckSourcePins, PinStore, PinnedSource, Repin, RepinLink};
use crate::check_source_requests::{self as requests, ORIGIN_LANDING, SourceChangeRequest};
use crate::check_source_resolve::SourceRoot;
use crate::check_source_rust::splice_item;
use crate::task_set_contract::content_digest;
use crate::task_set_publish_lock::PublishLockError;

/// Why a settled request was not applied.
#[derive(Debug)]
pub(super) enum ApplyError {
    /// A publish of the task set left a journal no settlement could settle
    /// (Issue 338): the round pauses the run on this evidence.
    Unsettled(String),
    /// Anything else: the request stays pending.
    Failed(String),
}

impl From<String> for ApplyError {
    fn from(error: String) -> Self {
        Self::Failed(error)
    }
}

impl From<&str> for ApplyError {
    fn from(error: &str) -> Self {
        Self::Failed(error.to_string())
    }
}

/// How many times a repin is computed again over pins that changed under it
/// before the request is left pending.
const REPIN_ATTEMPTS: usize = 3;

/// Apply an accepted held change (a change found in the tree is already
/// there) and re-pin every check it serves. Returns whether the tree changed.
/// The re-pin is computed over the pins the store binds when it is written
/// (Issue 338): pins a settled publish or another repin changed under it are
/// re-pinned again from what is bound now, never overwritten.
pub(super) fn accept(
    ctx: &Settle<'_>,
    pins: &mut CheckSourcePins,
    request: &SourceChangeRequest,
    proposed: Option<&[u8]>,
    reason: &str,
) -> Result<bool, ApplyError> {
    crate::stage_write::mapped(
        || accept_owned(ctx, pins, request, proposed, reason),
        |error| ApplyError::Failed(error.to_string()),
    )
}

fn accept_owned(
    ctx: &Settle<'_>,
    pins: &mut CheckSourcePins,
    request: &SourceChangeRequest,
    proposed: Option<&[u8]>,
    reason: &str,
) -> Result<bool, ApplyError> {
    let mut applied = false;
    if request.origin == ORIGIN_LANDING {
        let bytes = match &request.item {
            None => proposed.map(<[u8]>::to_vec),
            // A changed or deleted test function goes in over its pinned
            // text wherever the file has moved since; a new one goes in with
            // the file it was proposed in, which must still be the file that
            // landed around it (`settle_one` makes it stale otherwise).
            Some(key) if request.pinned_digest.is_some() => {
                let file = ctx.roots.of(request.root).join(&request.path);
                let text = std::fs::read_to_string(&file)
                    .map_err(|error| format!("{} could not be read: {error}", request.path))?;
                let replacement = proposed.map(|bytes| String::from_utf8_lossy(bytes).into_owned());
                let spliced = splice_item(&text, key, replacement.as_deref())
                    .ok_or_else(|| format!("{key} is no longer in {}", request.path))?;
                Some(spliced.into_bytes())
            }
            Some(_) => {
                let digest = request
                    .proposed_file_digest
                    .as_ref()
                    .ok_or("no proposed file")?;
                Some(
                    requests::blobs(ctx.run_root)
                        .get(digest)
                        .ok_or("the proposed file is not in the request store")?,
                )
            }
        };
        written(
            ctx,
            request,
            bytes.as_deref(),
            "accepted check-source change",
        )?;
        applied = true;
    }
    if let Some(bytes) = proposed {
        ctx.store.blobs.put(bytes);
    }
    let mut base = pins.clone();
    for _ in 0..REPIN_ATTEMPTS {
        let mut next = base.clone();
        repin(ctx, &mut next, request, reason);
        match ctx.store.write_over(&base, &next) {
            Ok(Repin::Written) => {
                *pins = next;
                return Ok(applied);
            }
            Ok(Repin::Stale(now)) if now.acceptance_digest == base.acceptance_digest => {
                base = now;
            }
            Ok(Repin::Stale(now)) => {
                return Err(ApplyError::Failed(format!(
                    "the task set's check-source pins now bind the contract {} instead of the one this round read ({}): it was republished while the change was settled, so nothing was re-pinned; the request stays pending and the next round settles it against the set it reads",
                    now.acceptance_digest, base.acceptance_digest
                )));
            }
            Err(PublishLockError::Unsettled(evidence)) => {
                return Err(ApplyError::Unsettled(evidence));
            }
            Err(PublishLockError::Failed(error)) => return Err(ApplyError::Failed(error)),
        }
    }
    Err(ApplyError::Failed(format!(
        "the check-source pins changed under the re-pin {REPIN_ATTEMPTS} times; nothing was re-pinned and the request stays pending"
    )))
}

/// Re-pin, in `pins`, every check `request` serves to its proposed source.
fn repin(
    ctx: &Settle<'_>,
    pins: &mut CheckSourcePins,
    request: &SourceChangeRequest,
    reason: &str,
) {
    let prior_digest = content_digest(&PinStore::bytes(pins));
    ctx.store.blobs.put(&PinStore::bytes(pins));
    for id in &request.check_ids {
        let Some(check) = pins.checks.get_mut(id) else {
            continue;
        };
        let item = request.item.as_deref();
        match check
            .sources
            .iter_mut()
            .find(|s| s.same_source(request.root, &request.path, item))
        {
            Some(source) => source.digest = request.proposed_digest.clone(),
            None => {
                check.sources.push(PinnedSource {
                    root: request.root,
                    path: request.path.clone(),
                    item: request.item.clone(),
                    digest: request.proposed_digest.clone(),
                    role: "re-pinned by a judged re-author request".into(),
                });
                check.sources.sort();
            }
        }
    }
    // The accepted source's place in its crate -- a module declaration or
    // cfg it brought with it -- is pinned with it, so it cannot be switched
    // off afterwards.
    for id in &request.check_ids {
        let Some(check) = pins.checks.get_mut(id) else {
            continue;
        };
        let now = crate::check_source_resolve::resolve(
            &check.command,
            ctx.roots.of(check.cwd),
            &ctx.roots,
        );
        for found in now.found {
            let chain = matches!(
                found.role.as_str(),
                crate::check_source_chain::ROLE_DECLARATION
                    | crate::check_source_chain::ROLE_CFG
                    | "manifest test entry"
            );
            let pinned = check
                .sources
                .iter()
                .any(|s| s.same_source(found.root, &found.path, found.item.as_deref()));
            if chain && !pinned {
                check.sources.push(crate::check_source_pins::pin_found(
                    &found,
                    &ctx.roots,
                    &ctx.store.blobs,
                ));
            }
        }
        check.sources.sort();
    }
    // Settling the same request again (after a crash) re-pins nothing twice.
    if pins
        .repins
        .iter()
        .any(|link| link.request_id == request.request_id)
    {
        return;
    }
    pins.repins.push(RepinLink {
        request_id: request.request_id.clone(),
        check_ids: request.check_ids.clone(),
        root: request.root,
        path: request.path.clone(),
        item: request.item.clone(),
        from: request.pinned_digest.clone(),
        to: request.proposed_digest.clone(),
        reason: reason.to_string(),
        at: chrono::Utc::now().to_rfc3339(),
        prior_digest,
    });
}

/// A refused held change simply stays out. A refused change found in the
/// tree is put back to its pinned bytes (removed, when it was not pinned or
/// pinned absent).
pub(super) fn refuse(
    ctx: &Settle<'_>,
    request: &SourceChangeRequest,
    pinned: Option<Option<Vec<u8>>>,
) -> Result<bool, String> {
    crate::stage_write::mapped(
        || refuse_owned(ctx, request, pinned),
        |error| error.to_string(),
    )
}

fn refuse_owned(
    ctx: &Settle<'_>,
    request: &SourceChangeRequest,
    pinned: Option<Option<Vec<u8>>>,
) -> Result<bool, String> {
    if request.origin == ORIGIN_LANDING {
        return Ok(false);
    }
    let pinned = match (request.was_pinned, pinned) {
        (false, _) => None,
        (true, Some(bytes)) => bytes,
        (true, None) => {
            return Err(format!(
                "the judge refused the change, and the pinned version of {} was not retained, so it cannot be restored; restore it or re-author the check",
                request.label()
            ));
        }
    };
    let base = ctx.roots.of(request.root);
    let bytes = match &request.item {
        None => pinned,
        Some(key) => {
            let text = std::fs::read_to_string(base.join(&request.path))
                .map_err(|error| format!("{} could not be read: {error}", request.path))?;
            let replacement = pinned
                .as_ref()
                .map(|b| String::from_utf8_lossy(b).into_owned());
            let spliced = splice_item(&text, key, replacement.as_deref()).ok_or_else(|| {
                format!(
                    "{key} is no longer in {}, so its pinned version cannot be put back",
                    request.path
                )
            })?;
            Some(spliced.into_bytes())
        }
    };
    written(
        ctx,
        request,
        bytes.as_deref(),
        "restored a pinned check source",
    )?;
    Ok(true)
}

/// Write `bytes` at the request's source and commit it -- both under the
/// repository's write lock, so no landing interleaves -- and if the commit
/// fails put the tree back as it was, so nothing lands uncommitted.
fn written(
    ctx: &Settle<'_>,
    request: &SourceChangeRequest,
    bytes: Option<&[u8]>,
    what: &str,
) -> Result<(), String> {
    let base = ctx.roots.of(request.root);
    let apply = || -> Result<(), String> {
        let before = match std::fs::read(base.join(&request.path)) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(format!("{}: {error}", request.path)),
        };
        write_source(base, &request.path, bytes)?;
        if let Err(error) = commit(ctx, request, what) {
            let undo = write_source(base, &request.path, before.as_deref());
            return Err(match undo {
                Ok(()) => format!("{error}; the change was taken back out"),
                Err(undo) => format!("{error}; and it could not be taken back out: {undo}"),
            });
        }
        Ok(())
    };
    if !in_git(ctx, request) {
        return apply();
    }
    let mut outcome = Ok(());
    crate::write_coordinator::patch_apply::with_repo_lock(ctx.roots.repository, || {
        outcome = apply();
        Ok(())
    })
    .map_err(|error| format!("the repository write lock: {error}"))?;
    outcome
}

fn in_git(ctx: &Settle<'_>, request: &SourceChangeRequest) -> bool {
    request.root == SourceRoot::Repository && ctx.roots.repository.join(".git").exists()
}

fn write_source(base: &Path, path: &str, bytes: Option<&[u8]>) -> Result<(), String> {
    let target = base.join(path);
    // Never write through a link: it is replaced, not followed.
    if std::fs::symlink_metadata(&target).is_ok_and(|meta| meta.file_type().is_symlink()) {
        std::fs::remove_file(&target).map_err(|error| format!("{path}: {error}"))?;
    }
    match bytes {
        Some(bytes) => {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
            }
            std::fs::write(&target, bytes).map_err(|error| format!("{path}: {error}"))
        }
        None => match std::fs::remove_file(&target) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("{path}: {error}")),
        },
    }
}

/// Commit exactly `request.path`, so later worktrees are cut from the
/// settled tree; called under the repository lock. Not a Git work tree, or
/// nothing to commit: nothing to do. Every git failure is the commit's.
fn commit(ctx: &Settle<'_>, request: &SourceChangeRequest, what: &str) -> Result<(), String> {
    if !in_git(ctx, request) {
        return Ok(());
    }
    let repo = ctx.roots.repository;
    let git = |args: &[&str]| {
        crate::write_coordinator::worktree_isolation::run_git(args, repo)
            .map(|output| output.stdout)
            .map_err(|error| format!("git {}: {error}", args.join(" ")))
    };
    let message = format!(
        "archon: {what} {} ({})",
        request.label(),
        request.request_id
    );
    git(&["add", "-A", "--", &request.path])?;
    let status = git(&["status", "--porcelain", "--", &request.path])?;
    if status.iter().all(u8::is_ascii_whitespace) {
        return Ok(());
    }
    git(&[
        "-c",
        "user.name=archon-workflow",
        "-c",
        "user.email=archon-workflow@local",
        "commit",
        "--no-gpg-sign",
        "--no-verify",
        "-q",
        "-m",
        &message,
        "--",
        &request.path,
    ])
    .map(|_| ())
    .map_err(|error| format!("committing {}: {error}", request.path))
}
