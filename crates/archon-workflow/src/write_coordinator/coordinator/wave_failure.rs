use std::collections::BTreeMap;
use std::path::Path;

use crate::work_unit_coverage;
use crate::write_coordinator::WriteCoordinatorConfig;
use crate::write_coordinator::patch_manifest::ManifestStatus;
use crate::write_coordinator::worktree_isolation::{WorkspaceStatus, cleanup_workspace};
use crate::write_coordinator::write_plan::NormalizedPath;

use super::{CoordinatedOutcome, ItemState, PlanRecord, WaveOutcome};

pub(super) fn finalize_failed_wave(
    canonical: &Path,
    cfg: &WriteCoordinatorConfig,
    wave: &crate::write_coordinator::Wave,
    mutator: &str,
    items: &[ItemState<'_>],
    outcome: &mut CoordinatedOutcome,
) {
    for it in items {
        outcome.item_status.insert(
            it.plan.item_id.clone(),
            ManifestStatus::Failed {
                reason: format!("wave aborted: {mutator}"),
            },
        );
        outcome.plans.push(PlanRecord {
            item_id: it.plan.item_id.clone(),
            wave_id: wave.wave_id,
            work_unit_ids: work_unit_coverage::item_required_units(&it.input.item.payload),
            target_files: it
                .plan
                .target_files
                .iter()
                .map(NormalizedPath::as_str)
                .collect(),
            changed_files: vec![],
            post_hashes: BTreeMap::new(),
            patch_bytes_len: 0,
        });
        let _ = cleanup_workspace(
            canonical,
            &it.plan.isolated_root,
            WorkspaceStatus::Failed,
            cfg,
        );
    }
    outcome.waves.push(WaveOutcome {
        wave_id: wave.wave_id,
        items: wave.items.clone(),
        apply_record: None,
        verify: None,
        failure: Some(mutator.to_string()),
    });
}

pub(super) fn cleanup_all(
    canonical: &Path,
    cfg: &WriteCoordinatorConfig,
    items: &mut [ItemState<'_>],
    status: WorkspaceStatus,
) {
    for it in items.iter() {
        let _ = cleanup_workspace(canonical, &it.plan.isolated_root, status, cfg);
    }
}
