//! Marker recording around the fd-relative mutation operations.
use super::super::mutation_fd;
use super::{CheckResult, MutationPlan};

pub(super) fn move_aside_plan(
    plan: &MutationPlan,
    root: Option<&mutation_fd::SiteRoot>,
) -> Vec<mutation_fd::MoveOutcome> {
    let mut outcomes = Vec::with_capacity(plan.inputs.len());
    for input in &plan.inputs {
        let covering_ancestor = plan
            .inputs
            .iter()
            .zip(&outcomes)
            .find(|(ancestor, outcome)| {
                **outcome == mutation_fd::MoveOutcome::Moved
                    && super::super::is_strict_descendant(&input.move_path, &ancestor.move_path)
            });
        let outcome = if let Some((ancestor, _)) = covering_ancestor {
            mutation_fd::MoveOutcome::CoveredByAncestor(ancestor.named_path.clone())
        } else {
            match root {
                Some(root) => mutation_fd::move_aside(root, &input.move_path, &input.aside_path),
                None => mutation_fd::MoveOutcome::Failed("mutation site root unavailable".into()),
            }
        };
        outcomes.push(outcome);
    }
    outcomes
}

pub(super) fn record_move_outcomes(
    result: &mut CheckResult,
    plan: &MutationPlan,
    outcomes: &[mutation_fd::MoveOutcome],
) {
    for (index, (input, outcome)) in plan.inputs.iter().zip(outcomes).enumerate() {
        let state = match outcome {
            mutation_fd::MoveOutcome::Moved => {
                result.stderr.extend_from_slice(
                    format!("archon-mutation-moved-{}: {index}\n", plan.nonce).as_bytes(),
                );
                if input.site_copy_alias {
                    "moved in the site copy".to_string()
                } else {
                    "moved".to_string()
                }
            }
            mutation_fd::MoveOutcome::CoveredByAncestor(ancestor) => {
                format!("covered by moved ancestor '{ancestor}'")
            }
            mutation_fd::MoveOutcome::Absent => "absent at the selected root".into(),
            mutation_fd::MoveOutcome::SymlinkComponent => "refused: symlink component".into(),
            mutation_fd::MoveOutcome::Failed(error) => format!("move failed: {error}"),
        };
        result.stderr.extend_from_slice(
            format!(
                "archon-mutation-move-state-{}: {index}:{state}\n",
                plan.nonce
            )
            .as_bytes(),
        );
    }
}

/// Observe a moved input after the check's process tree exited and before
/// restoration. This narrow absence-to-presence window is the evidence used
/// for a check-attributed recreation marker.
pub(super) fn record_check_recreated(
    result: &mut CheckResult,
    plan: &MutationPlan,
    moved: &[mutation_fd::MoveOutcome],
    root: &mutation_fd::SiteRoot,
) {
    for (index, (input, outcome)) in plan.inputs.iter().zip(moved).enumerate() {
        if *outcome == mutation_fd::MoveOutcome::Moved {
            match mutation_fd::site_entry_exists(root, &input.move_path) {
                Ok(true) => result.stderr.extend_from_slice(
                    format!("archon-mutation-check-recreated-{}: {index}\n", plan.nonce).as_bytes(),
                ),
                Ok(false) => {}
                Err(error) => {
                    let note = format!(
                        "could not observe whether check recreated {}: {error}",
                        input.move_path.display()
                    );
                    result.operational_error = Some(match result.operational_error.take() {
                        Some(previous) => format!("{previous}; {note}"),
                        None => note,
                    });
                }
            }
        }
    }
}

pub(super) fn snapshot_parent_states(
    plan: &MutationPlan,
    moved: &[mutation_fd::MoveOutcome],
    root: Option<&mutation_fd::SiteRoot>,
) -> Vec<Option<Result<String, String>>> {
    plan.inputs
        .iter()
        .zip(moved)
        .map(|(input, outcome)| {
            (*outcome == mutation_fd::MoveOutcome::Moved)
                .then(|| {
                    root.map(|root| {
                        mutation_fd::snapshot_parent(root, &input.move_path)
                            .map_err(|error| error.to_string())
                    })
                })
                .flatten()
        })
        .collect()
}

pub(super) fn restore_plan(
    result: &mut CheckResult,
    plan: &MutationPlan,
    moved: &[mutation_fd::MoveOutcome],
    parent_snapshots: &[Option<Result<String, String>>],
    root: &mutation_fd::SiteRoot,
) {
    let mut safe = true;
    for (index, (input, outcome)) in plan.inputs.iter().zip(moved).enumerate().rev() {
        if *outcome != mutation_fd::MoveOutcome::Moved {
            continue;
        }
        let current_parent = mutation_fd::snapshot_parent(root, &input.move_path);
        let before_parent = parent_snapshots
            .get(index)
            .and_then(Option::as_ref)
            .expect("moved inputs have a parent snapshot entry");
        let before_parent = match before_parent {
            Ok(before_parent) => before_parent,
            Err(reason) => {
                let earlier_moved_ancestor =
                    plan.inputs[..index]
                        .iter()
                        .zip(&moved[..index])
                        .any(|(ancestor, outcome)| {
                            *outcome == mutation_fd::MoveOutcome::Moved
                                && super::super::is_strict_descendant(
                                    &input.move_path,
                                    &ancestor.move_path,
                                )
                        });
                let state = if earlier_moved_ancestor {
                    match mutation_fd::restore(root, &input.aside_path, &input.move_path) {
                        mutation_fd::RestoreOutcome::Restored => "restored".to_string(),
                        mutation_fd::RestoreOutcome::OriginalRecreated => {
                            safe = false;
                            record_unsafe_recreation(
                                result,
                                plan,
                                index,
                                format!("parent snapshot failed: {reason}"),
                            )
                        }
                        mutation_fd::RestoreOutcome::AsideMissing => "aside-missing".into(),
                        mutation_fd::RestoreOutcome::UnsafeToRestore(restore_reason) => {
                            safe = false;
                            format!("unsafe-to-restore: {restore_reason}")
                        }
                    }
                } else {
                    safe = false;
                    match mutation_fd::restore(root, &input.aside_path, &input.move_path) {
                        mutation_fd::RestoreOutcome::Restored => {
                            format!("unsafe-to-restore: parent snapshot failed: {reason}")
                        }
                        mutation_fd::RestoreOutcome::AsideMissing => "aside-missing".into(),
                        mutation_fd::RestoreOutcome::OriginalRecreated => record_unsafe_recreation(
                            result,
                            plan,
                            index,
                            format!("parent snapshot failed: {reason}"),
                        ),
                        mutation_fd::RestoreOutcome::UnsafeToRestore(restore_reason) => {
                            format!("unsafe-to-restore: {restore_reason}")
                        }
                    }
                };
                result.stdout.extend_from_slice(
                    format!("archon-mutation-state-{}: {index}:{state}\n", plan.nonce).as_bytes(),
                );
                continue;
            }
        };
        let parent_changed = match current_parent {
            Ok(current) => current != *before_parent,
            Err(error) => {
                let state = match mutation_fd::restore(root, &input.aside_path, &input.move_path) {
                    mutation_fd::RestoreOutcome::AsideMissing => "aside-missing".to_string(),
                    mutation_fd::RestoreOutcome::OriginalRecreated => {
                        safe = false;
                        record_unsafe_recreation(
                            result,
                            plan,
                            index,
                            format!("parent snapshot failed while checking: {error}"),
                        )
                    }
                    mutation_fd::RestoreOutcome::UnsafeToRestore(reason) => {
                        safe = false;
                        format!("unsafe-to-restore: {reason}")
                    }
                    mutation_fd::RestoreOutcome::Restored => {
                        safe = false;
                        format!("unsafe-to-restore: parent snapshot failed while checking: {error}")
                    }
                };
                result.stdout.extend_from_slice(
                    format!("archon-mutation-state-{}: {index}:{state}\n", plan.nonce).as_bytes(),
                );
                continue;
            }
        };
        let restored = match mutation_fd::restore(root, &input.aside_path, &input.move_path) {
            mutation_fd::RestoreOutcome::Restored => "restored".to_string(),
            mutation_fd::RestoreOutcome::OriginalRecreated => {
                safe = false;
                result.stderr.extend_from_slice(
                    format!("archon-mutation-generated-{}: {index}\n", plan.nonce).as_bytes(),
                );
                result.stderr.extend_from_slice(
                    format!("archon-mutation-recreated-{}: {index}\n", plan.nonce).as_bytes(),
                );
                "original-recreated".into()
            }
            mutation_fd::RestoreOutcome::AsideMissing => "aside-missing".into(),
            mutation_fd::RestoreOutcome::UnsafeToRestore(reason) => {
                safe = false;
                format!("unsafe-to-restore: {reason}")
            }
        };
        let state = if parent_changed {
            safe = false;
            result.stderr.extend_from_slice(
                format!("archon-mutation-recreated-{}: {index}\n", plan.nonce).as_bytes(),
            );
            "original-recreated".to_string()
        } else {
            restored
        };
        result.stdout.extend_from_slice(
            format!("archon-mutation-state-{}: {index}:{state}\n", plan.nonce).as_bytes(),
        );
    }
    if safe {
        result
            .stdout
            .extend_from_slice(format!("archon-mutation-restored-{}\n", plan.nonce).as_bytes());
    }
}

fn record_unsafe_recreation(
    result: &mut CheckResult,
    plan: &MutationPlan,
    index: usize,
    reason: String,
) -> String {
    result.stderr.extend_from_slice(
        format!("archon-mutation-generated-{}: {index}\n", plan.nonce).as_bytes(),
    );
    result.stderr.extend_from_slice(
        format!("archon-mutation-recreated-{}: {index}\n", plan.nonce).as_bytes(),
    );
    format!("unsafe-to-restore: {reason}")
}

pub(super) fn mark_plan_unsafe(
    result: &mut CheckResult,
    plan: &MutationPlan,
    moved: &[mutation_fd::MoveOutcome],
) {
    for (index, outcome) in moved.iter().enumerate() {
        if *outcome == mutation_fd::MoveOutcome::Moved {
            result.stdout.extend_from_slice(
                format!("archon-mutation-state-{}: {index}:unsafe-to-restore: teardown did not confirm the process tree gone\n", plan.nonce).as_bytes(),
            );
        }
    }
}
