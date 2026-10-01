//! Whether a run's launch recorded lineage, and the refusal a lineage-bound
//! run gives a pin move no recorded republish explains.
//!
//! A launch snapshot, and a pin written by freeze or republish, carries
//! [`LINEAGE_RECORDING_V1`] once the binary that wrote it records a lineage
//! link for every sanctioned per-check republish. A run whose launch snapshot
//! carries no marker predates lineage recording: only for such a run may a
//! pin move with no recorded lineage be proven from the launch contract and
//! the current one alone. For a run launched with the marker, every
//! sanctioned change after launch is recorded, so an unrecorded one is
//! tampering.

use super::{AcceptancePin, ChainCheck, ChainRefusal, PortableAcceptanceIdentityV1};

/// The engine namespace under a project's `.archon/` holding the frozen task
/// sets' pins, their chain history and their check-source sidecars.
pub const PIN_STORE_NAMESPACE: &str = "task-set-pins";

/// `<project>/.archon/<PIN_STORE_NAMESPACE>`: where every pin lives.
pub fn pin_store_dir(project_root: &std::path::Path) -> std::path::PathBuf {
    project_root.join(".archon").join(PIN_STORE_NAMESPACE)
}

/// The lineage-recording marker a launch snapshot and a pin carry.
pub const LINEAGE_RECORDING_V1: u32 = 1;

/// The sanctioned way to change a frozen check after launch.
pub const REAUTHOR_COMMAND: &str =
    "archon workflow freeze-acceptance --tasks <DIR> --prd <PATH> --reauthor <CHECK_ID>";

/// What a run's launch recorded about lineage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchLineage {
    /// The launch snapshot carries no lineage marker: the run predates
    /// lineage recording.
    Predates,
    /// The launch snapshot carries a lineage marker: every sanctioned change
    /// after launch is recorded on the pin's lineage.
    Recorded,
}

impl LaunchLineage {
    /// The launch lineage a snapshot's marker records. Any marker, including
    /// one of a later version, binds the run to recorded lineage.
    pub fn from_marker(marker: Option<u32>) -> Self {
        match marker {
            Some(_) => Self::Recorded,
            None => Self::Predates,
        }
    }
}

/// The `unrecorded_change` refusal a lineage-bound run gives a pin that moved
/// from `launch` with no recorded republish starting at the launch pin, or
/// `None` when the run predates lineage recording, the pin is the launch pin,
/// or a recorded link starts at it.
pub fn unrecorded_under_recording(
    launch: &PortableAcceptanceIdentityV1,
    launch_lineage: LaunchLineage,
    pin: &AcceptancePin,
) -> Option<ChainRefusal> {
    if launch_lineage == LaunchLineage::Predates
        || pin.identity() == *launch
        || pin.lineage.iter().any(|link| link.from == *launch)
    {
        return None;
    }
    Some(ChainRefusal {
        check: ChainCheck::UnrecordedChange,
        detail: format!(
            "pin {} differs from launch pin {} and no recorded republish starts at the launch pin; the run launched recording lineage, so a contract change it does not record is tampering, and importing the launch chain cannot prove it. A frozen check is changed only by `{REAUTHOR_COMMAND}`",
            pin.freeze_event_id, launch.freeze_event_id
        ),
    })
}
