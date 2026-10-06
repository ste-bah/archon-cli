//! Quarantine evidence that will not parse (Issue 317).
//!
//! The evidence file is what a load knows of a quarantined record: its
//! place, and its failing state when a copy held it. Damaged evidence must
//! never be skipped: the record would then count as never quarantined, and
//! a run whose every record is quarantined would bring back the legacy
//! ledger (its old revisit count) with nothing reported. Its record is a
//! loss of unknown state instead, reported, paused on and acknowledged like
//! any other; the damaged bytes are kept.

use std::path::Path;

use super::{
    DAMAGED_SUFFIX, EVIDENCE_SUFFIX, QUARANTINE_EVENT, QuarantinedRecordV1, attempt_of, file_time,
    relative, round_of,
};

/// Evidence that would not parse, kept when an acknowledgement replaced it.
/// Not an evidence name: no load reads it as one.
const UNREADABLE_EVIDENCE_SUFFIX: &str = ".evidence.unreadable";

/// Issue 317: evidence that will not parse is never skipped. When the
/// record's bytes moved beside it (its `.damaged` file), the record is
/// LOST and what the evidence said of it is gone: it is rebuilt from the
/// file names with no state, so it counts as quarantined (the legacy ledger
/// never returns) and, unless the saved ledger's copy holds its state, the
/// load reports it as a loss of unknown state for the caller to pause on.
/// When the bytes never moved (a crash between the evidence and the move),
/// the record is still in its round, or was quarantined again under
/// another name, and is counted there. A name that says nothing of the
/// record is an error, never a skip.
pub(super) fn unreadable_evidence(
    run_dir: &Path,
    round: &Path,
    path: &Path,
    error: &serde_json::Error,
) -> crate::WorkflowResult<Option<QuarantinedRecordV1>> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    let stem = name.strip_suffix(EVIDENCE_SUFFIX).unwrap_or(name);
    let moved = path.with_file_name(format!("{stem}{DAMAGED_SUFFIX}"));
    // A file system that will not say whether the bytes moved is an I/O
    // error the caller pauses on, never "they never moved".
    match moved.try_exists() {
        Ok(true) => {}
        Ok(false) => {
            let original = stem.rsplit_once('.').map_or(stem, |(record, _)| record);
            let original = round.join(original);
            // Only an intact original proves that the move never happened.
            // Missing bytes could also be a second loss after quarantine.
            let original_bytes = match std::fs::read(&original) {
                Ok(bytes) => Some(bytes),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(crate::WorkflowError::io(&original, error)),
            };
            if original_bytes
                .and_then(|bytes| {
                    serde_json::from_slice::<super::AcceptanceRoundRecordV1>(&bytes).ok()
                })
                .is_some_and(|record| {
                    Some(record.round) == round_of(round)
                        && Some(record.attempt) == attempt_of(stem)
                })
            {
                tracing::warn!(path = %path.display(), %error, "damaged quarantine evidence has an intact original; the record is counted there");
                return Ok(None);
            }
            return Err(crate::WorkflowError::StateCorrupt(format!(
                "quarantine evidence {} will not parse ({error}); its damaged bytes and intact original are missing, so acceptance history cannot be reconstructed",
                path.display()
            )));
        }
        Err(source) => return Err(crate::WorkflowError::io(&moved, source)),
    }
    let (Some(round_number), Some(attempt)) = (round_of(round), attempt_of(stem)) else {
        return Err(crate::WorkflowError::StateCorrupt(format!(
            "quarantine evidence {} will not parse ({error}) and its name names no round attempt",
            path.display()
        )));
    };
    let original = stem.rsplit_once('.').map_or(stem, |(record, _)| record);
    let quarantined_at = std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .map_or_else(
            |_| chrono::Utc::now(),
            chrono::DateTime::<chrono::Utc>::from,
        );
    tracing::warn!(path = %path.display(), %error, "quarantine evidence will not parse; its record is lost, state unknown");
    Ok(Some(QuarantinedRecordV1 {
        event: QUARANTINE_EVENT.to_string(),
        round: round_number,
        attempt,
        original: relative(run_dir, &round.join(original)),
        quarantined: relative(run_dir, &moved),
        reason: format!(
            "its quarantine evidence {} will not parse ({error}), so its failing state is unknown",
            relative(run_dir, path)
        ),
        state: None,
        progress_frontier: super::super::super::reservation::frontier(
            run_dir,
            round_number,
            attempt,
        )?,
        written_nanos: u64::try_from(file_time(&moved)).unwrap_or(u64::MAX),
        quarantined_at: quarantined_at.to_rfc3339(),
        acknowledged_at: None,
    }))
}

/// Issue 317: evidence that will not parse is replaced by the
/// acknowledgement, never lost: its bytes are kept first, beside it, under a
/// name no load reads as evidence.
pub(super) fn keep_unreadable_evidence(
    evidence: &Path,
    dir: &Path,
    name: &str,
) -> crate::WorkflowResult<()> {
    let bytes = match std::fs::read(evidence) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(crate::WorkflowError::io(evidence, error)),
    };
    if serde_json::from_slice::<QuarantinedRecordV1>(&bytes).is_ok() {
        return Ok(());
    }
    crate::store::write_atomic(
        &dir.join(format!(".{name}.unreadable.tmp")),
        &dir.join(format!("{name}{UNREADABLE_EVIDENCE_SUFFIX}")),
        &bytes,
    )
}
