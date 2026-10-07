//! Issue 301 (round-4 review minor 6): a recovery re-freeze accepts only the
//! skeleton its anchor binds, the newest launch of the task set's
//! authenticated launch history. A live skeleton that is another launch's is
//! a rollback, and it refuses by name with the remedy:
//! - superseded: a launch the prior pin's lineage (oldest first) moved on
//!   from;
//! - unrelated: a captured run's launch outside that history.
//!
//! Anything else is an unauthenticated shape and refuses as before.
use super::*;

/// The refusal for a live skeleton (`live`, read from `live_path`) that the
/// recovery anchor does not authenticate.
pub(super) fn refusal(
    record: &Recovery,
    anchor: &PortableAcceptanceIdentityV1,
    live: &[u8],
    live_path: &Path,
    pin: &Path,
) -> Result<anyhow::Error> {
    let history = ChainHistory::for_pin(pin);
    let live_name = live_path.display();
    let remedy = format!(
        "to recover, restore {live_name} to the skeleton of anchor launch {} (archived preimage {}) and retry the re-freeze, or re-freeze the task set and start a new run",
        anchor.freeze_event_id,
        anchor.skeleton_digest.as_deref().unwrap_or("none"),
    );
    let lineage = record.prior.iter().flat_map(|prior| prior.lineage.iter());
    for link in lineage {
        if link.from != *anchor && is_launch_skeleton(record, &link.from, live, &history)? {
            return Ok(anyhow!(
                "chain check skeleton_changed failed: the live skeleton {live_name} is the skeleton of launch {}, which launch {} superseded in this task set's authenticated launch history; a recovery re-freeze never rolls back to a superseded launch; {remedy}",
                link.from.freeze_event_id,
                link.to.freeze_event_id,
            ));
        }
    }
    for (run, launch) in &record.runs {
        if launch != anchor && is_launch_skeleton(record, launch, live, &history)? {
            return Ok(anyhow!(
                "chain check skeleton_changed failed: the live skeleton {live_name} is the skeleton of launch {} of run {run}, which is not in this task set's authenticated launch history (anchor launch {}); a recovery re-freeze never rolls back to an unrelated launch; {remedy}",
                launch.freeze_event_id,
                anchor.freeze_event_id,
            ));
        }
    }
    Ok(anyhow!(
        "chain check skeleton_changed failed: live skeleton shape or contract binding is not authenticated by a captured launch; {remedy}"
    ))
}

/// Whether `live` is `launch`'s skeleton: byte-identical to its bound
/// preimage, or that preimage's shape with an authenticated contract binding.
fn is_launch_skeleton(
    record: &Recovery,
    launch: &PortableAcceptanceIdentityV1,
    live: &[u8],
    history: &ChainHistory,
) -> Result<bool> {
    let Some(bound) = &launch.skeleton_digest else {
        return Ok(false);
    };
    if *bound == content_digest(live) {
        return Ok(true);
    }
    match history.get(bound)? {
        Some(preimage) => evidence::authenticates_skeleton(record, launch, live, &preimage),
        None => Ok(false),
    }
}
