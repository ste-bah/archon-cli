//! The tripwire around one host-side span, and the landing's hold on the
//! host's write section. Split from `input_tripwire` for size.

use std::path::{Path, PathBuf};

use super::{EnvironmentViolation, InputTripwire, host_write_section, note_host_write, state_of};
use crate::write_coordinator::PatchManifest;

/// Run the host-run command `work` under the tripwire for the run at
/// `run_root` (none: unwatched). `Some` violation when it changed an input;
/// the change is already restored and logged.
pub fn watch_sync<T>(
    run_root: Option<&Path>,
    label: &str,
    work: impl FnOnce() -> T,
) -> (T, Option<EnvironmentViolation>) {
    let tripwire = run_root.and_then(InputTripwire::arm);
    let out = work();
    (out, tripwire.and_then(|tripwire| tripwire.check(label)))
}

/// [`watch_sync`] for an async `work`; arming and checking run on the
/// blocking pool (both read the inputs, and a check waits out a landing).
pub async fn watch<T>(
    run_root: Option<&Path>,
    label: &str,
    work: impl std::future::Future<Output = T>,
) -> (T, Option<EnvironmentViolation>) {
    watch_inner(run_root, label, &[], work).await
}

/// Reconcile an earlier evaluation before any new round reserves evidence
/// or repairs inputs. Only the current owner can settle the pending map.
pub fn reconcile_owned(
    writer: &crate::stage_write::StageWriter,
    root: &Path,
) -> crate::WorkflowResult<()> {
    crate::stage_write::with_writer(writer, || {
        super::pending::reconcile(root).map_err(|error| {
            crate::WorkflowError::ControlPaused(format!(
                "input comparison could not reconcile: {error}"
            ))
        })
    })
}

/// A stage's tripwire mutations use its owner even on the blocking pool.
/// A refused arm or check stops the stage; it is never an unwatched success.
pub fn watch_owned<T>(
    writer: crate::stage_write::StageWriter,
    run_root: Option<&Path>,
    label: &str,
    work: impl std::future::Future<Output = T>,
) -> impl std::future::Future<Output = crate::WorkflowResult<(T, Option<EnvironmentViolation>)>> {
    // Box before entering the async frame, as the run-control race does.
    watch_owned_inner(
        writer,
        run_root.map(Path::to_path_buf),
        label.to_string(),
        Box::pin(work),
    )
}

async fn watch_owned_inner<T>(
    writer: crate::stage_write::StageWriter,
    root: Option<PathBuf>,
    label: String,
    work: std::pin::Pin<Box<impl std::future::Future<Output = T>>>,
) -> crate::WorkflowResult<(T, Option<EnvironmentViolation>)> {
    let arm_writer = writer.clone();
    let arm_label = label.clone();
    let tripwire = tokio::task::spawn_blocking(move || {
        crate::stage_write::with_writer(&arm_writer, || {
            root.as_deref()
                .map(|root| super::pending::arm(root, &arm_label))
                .transpose()
                .map(Option::flatten)
                .map_err(|error| {
                    crate::WorkflowError::ControlPaused(format!(
                        "input comparison could not arm: {error}"
                    ))
                })
        })
    })
    .await
    .map_err(|error| {
        crate::WorkflowError::HostOperational(format!("input tripwire could not arm: {error}"))
    })??;
    let out = crate::stage_write::scope(writer.clone(), work).await;
    let violation = tokio::task::spawn_blocking(move || {
        crate::stage_write::with_writer(&writer, || {
            tripwire
                .map(|tripwire| tripwire.check())
                .transpose()
                .map(Option::flatten)
                .map_err(|error| {
                    crate::WorkflowError::ControlPaused(format!(
                        "input comparison could not complete: {error}"
                    ))
                })
        })
    })
    .await
    .map_err(|error| {
        crate::WorkflowError::HostOperational(format!("input tripwire could not check: {error}"))
    })??;
    if let Some(violation) = &violation {
        eprintln!("{label}: {}", violation.message());
    }
    Ok((out, violation))
}

/// [`watch`] for a write-capable call, whose changes under `own` (its
/// working tree and stamped deliveries) are its work
/// ([`InputTripwire::exempting`]).
pub async fn watch_exempting<T>(
    run_root: Option<&Path>,
    label: &str,
    own: &[PathBuf],
    work: impl std::future::Future<Output = T>,
) -> (T, Option<EnvironmentViolation>) {
    watch_inner(run_root, label, own, work).await
}

async fn watch_inner<T>(
    run_root: Option<&Path>,
    label: &str,
    own: &[PathBuf],
    work: impl std::future::Future<Output = T>,
) -> (T, Option<EnvironmentViolation>) {
    let tripwire = match run_root.map(Path::to_path_buf) {
        Some(root) => tokio::task::spawn_blocking(move || InputTripwire::arm(&root))
            .await
            .ok()
            .flatten(),
        None => None,
    };
    let tripwire = tripwire
        .map(|tripwire| (own.iter()).fold(tripwire, |tripwire, root| tripwire.exempting(root)));
    let in_flight = (!own.is_empty()).then(|| super::InFlight::register(own));
    let out = work.await;
    let Some(tripwire) = tripwire else {
        return (out, None);
    };
    let owned = label.to_string();
    let violation = tokio::task::spawn_blocking(move || tripwire.check(&owned))
        .await
        .ok()
        .flatten();
    drop(in_flight);
    if let Some(violation) = &violation {
        eprintln!("{label}: {}", violation.message());
    }
    (out, violation)
}

/// A landing's hold on the host's write section. When dropped it first
/// notes the state of every path the landing may have written in the
/// canonical checkout (a project that is its own repository has its tracked
/// inputs written by `git apply`, which [`note_host_write`] never sees), then
/// releases the section: no check can run in between.
pub struct LandingSection {
    paths: Vec<PathBuf>,
    _section: std::sync::MutexGuard<'static, ()>,
}

impl Drop for LandingSection {
    fn drop(&mut self) {
        for path in &self.paths {
            let (state, _) = state_of(path);
            note_host_write(path, &state);
        }
    }
}

/// Hold the host's write section for the landing of `manifests`.
pub fn landing_section(canonical_root: &Path, manifests: &[PatchManifest]) -> LandingSection {
    let _section = host_write_section();
    let paths = (manifests.iter())
        .flat_map(|m| {
            (m.changed_files.iter())
                .chain(&m.created_files)
                .chain(&m.deleted_files)
        })
        .map(|rel| canonical_root.join(rel))
        .collect();
    LandingSection { paths, _section }
}
