//! The acceptance history heals itself (Issue 262, round 8).
//!
//! A round record that will not parse is damage (records land whole, so it
//! is never a write in progress). It must never fail the run, and it must
//! never be read again on every resume. It is QUARANTINED: its bytes move
//! to `<round>/quarantine/` beside an evidence file naming the record, why,
//! and its failing state when known; nothing is deleted.
//!
//! Its state comes from the progress ledger's copy (`observed`), saved
//! after every round. With that copy the rebuild is exact, so a lost record
//! makes neither a stall look like progress nor progress look like a
//! stall. Without one the state is unknown: guessing either way could do
//! both (dropping a new state merges two revisit streaks; dropping a
//! revisit shortens one), so the load reports it ([`HealedLedger::unknown`])
//! and the caller PAUSES with the evidence. A resume goes on from the
//! remaining records: the gap is then a known, reported loss, bounded to
//! the one record.
//!
//! A record or log the file system will not hand over is no damage: that
//! I/O error is returned for the caller to pause on, and nothing moves.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::order::{Recorded, in_recorded_order};
use super::{ACCEPTANCE_RECORDS_DIR, AcceptanceRoundRecordV1, ObservedState, ProgressLedger};

/// Where a round's damaged records go, under its round directory.
pub const QUARANTINE_DIR: &str = "quarantine";

/// The evidence event's name, in its file and in the run's events.
pub const QUARANTINE_EVENT: &str = "acceptance_record_quarantined";

const EVIDENCE_SUFFIX: &str = ".evidence.json";
const DAMAGED_SUFFIX: &str = ".damaged";

/// What was quarantined, and why: the evidence file's content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuarantinedRecordV1 {
    pub event: String,
    pub round: u32,
    pub attempt: u32,
    /// Where the record was, relative to the run directory.
    pub original: String,
    /// Where its bytes are now, relative to the run directory.
    pub quarantined: String,
    pub reason: String,
    /// Its failing state, from the progress ledger's copy; `None` when no
    /// copy held it.
    pub state: Option<Vec<String>>,
    /// The record's file time, epoch nanoseconds: its place when the order
    /// log has no entry for it.
    pub written_nanos: u64,
    pub quarantined_at: String,
}

/// The ledger a load rebuilt, and the records it quarantined doing so.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealedLedger {
    pub ledger: ProgressLedger,
    /// Quarantined by this load (reported once, at discovery).
    pub quarantined: Vec<QuarantinedRecordV1>,
}

impl HealedLedger {
    /// The records quarantined with no copy of their state: the rebuild
    /// cannot be exact, and the caller pauses.
    pub fn unknown(&self) -> Vec<&QuarantinedRecordV1> {
        (self.quarantined.iter())
            .filter(|record| record.state.is_none())
            .collect()
    }
}

/// A record that would not parse.
struct Damaged {
    path: PathBuf,
    round: u32,
    attempt: u32,
    reason: String,
    written: u128,
}

impl ProgressLedger {
    /// The ledger the round records leave, healing what is damaged: every
    /// readable record, plus each quarantined one whose state is known, in
    /// recording order. No record at all leaves the saved ledger (a run
    /// from before the records). An I/O error is returned, never healed.
    pub fn load_healing(run_dir: &Path) -> crate::WorkflowResult<HealedLedger> {
        let saved = Self::saved(run_dir);
        let (mut history, damaged) = scan(run_dir)?;
        let mut quarantined = Vec::new();
        for record in damaged {
            let state = saved
                .as_ref()
                .and_then(|s| s.state_of(record.round, record.attempt));
            quarantined.extend(quarantine(run_dir, record, state.cloned())?);
        }
        history.extend(quarantined_states(run_dir, saved.as_ref())?);
        if history.is_empty() && quarantined.is_empty() {
            return Ok(HealedLedger {
                ledger: saved.unwrap_or_default(),
                quarantined,
            });
        }
        Ok(HealedLedger {
            ledger: Self::from_states(in_recorded_order(run_dir, history)?),
            quarantined,
        })
    }
}

/// When the file at `path` was last written, in epoch nanoseconds.
fn file_time(path: &Path) -> u128 {
    std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|at| at.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |at| at.as_nanos())
}

/// The paths in `dir` whose file names `keep` accepts; none when `dir` does
/// not exist (no round recorded yet).
fn entries_named(dir: &Path, keep: impl Fn(&str) -> bool) -> crate::WorkflowResult<Vec<PathBuf>> {
    let listing = match std::fs::read_dir(dir) {
        Ok(listing) => listing,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(crate::WorkflowError::io(dir, error)),
    };
    let mut paths = Vec::new();
    for entry in listing {
        let entry = entry.map_err(|e| crate::WorkflowError::io(dir, e))?;
        if entry.file_name().to_str().is_some_and(&keep) {
            paths.push(entry.path());
        }
    }
    Ok(paths)
}

fn round_dirs(run_dir: &Path) -> crate::WorkflowResult<Vec<PathBuf>> {
    let records = run_dir.join(ACCEPTANCE_RECORDS_DIR);
    let rounds = entries_named(&records, |name| name.starts_with("round-"))?;
    Ok(rounds.into_iter().filter(|round| round.is_dir()).collect())
}

/// The attempt number a record or quarantine file name carries.
fn attempt_of(name: &str) -> Option<u32> {
    name.strip_prefix("attempt-")?
        .split('.')
        .next()?
        .parse()
        .ok()
}

fn round_of(dir: &Path) -> Option<u32> {
    dir.file_name()?
        .to_str()?
        .strip_prefix("round-")?
        .parse()
        .ok()
}

/// Every readable record's state, and every record that would not parse.
fn scan(run_dir: &Path) -> crate::WorkflowResult<(Vec<Recorded<ObservedState>>, Vec<Damaged>)> {
    let (mut readable, mut damaged) = (Vec::new(), Vec::new());
    let is_record = |name: &str| name.starts_with("attempt-") && name.ends_with(".json");
    for round in round_dirs(run_dir)? {
        for path in entries_named(&round, is_record)? {
            let bytes = match std::fs::read(&path) {
                Ok(bytes) => bytes,
                // Moved meanwhile (another writer quarantined it).
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(crate::WorkflowError::io(&path, error)),
            };
            let written = file_time(&path);
            match serde_json::from_slice::<AcceptanceRoundRecordV1>(&bytes) {
                Ok(record) => readable.push((
                    written,
                    (record.round, record.attempt),
                    ObservedState {
                        round: record.round,
                        attempt: record.attempt,
                        state: super::state_key(&record),
                    },
                )),
                Err(error) => {
                    let name = path.file_name().and_then(|name| name.to_str());
                    let (Some(round), Some(attempt)) =
                        (round_of(&round), name.and_then(attempt_of))
                    else {
                        continue;
                    };
                    damaged.push(Damaged {
                        reason: format!("the record will not parse: {error}"),
                        path,
                        round,
                        attempt,
                        written,
                    });
                }
            }
        }
    }
    Ok((readable, damaged))
}

fn relative(run_dir: &Path, path: &Path) -> String {
    super::super::relative_record_path(run_dir, path)
}

/// Moves `record` into its round's quarantine, evidence first (a crash
/// between the two leaves the record in place, to be quarantined again;
/// evidence whose bytes never moved is ignored). `None` when another writer
/// moved it first.
fn quarantine(
    run_dir: &Path,
    record: Damaged,
    state: Option<Vec<String>>,
) -> crate::WorkflowResult<Option<QuarantinedRecordV1>> {
    let round_dir = record.path.parent().unwrap_or(run_dir).to_path_buf();
    let dir = round_dir.join(QUARANTINE_DIR);
    std::fs::create_dir_all(&dir).map_err(|e| crate::WorkflowError::io(&dir, e))?;
    let name = record
        .path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("record");
    let stem = format!("{name}.{}", uuid::Uuid::new_v4());
    let moved = dir.join(format!("{stem}{DAMAGED_SUFFIX}"));
    let evidence = QuarantinedRecordV1 {
        event: QUARANTINE_EVENT.to_string(),
        round: record.round,
        attempt: record.attempt,
        original: relative(run_dir, &record.path),
        quarantined: relative(run_dir, &moved),
        reason: record.reason,
        state,
        written_nanos: u64::try_from(record.written).unwrap_or(u64::MAX),
        quarantined_at: chrono::Utc::now().to_rfc3339(),
    };
    let evidence_path = dir.join(format!("{stem}{EVIDENCE_SUFFIX}"));
    let staging = dir.join(format!(".{stem}.tmp"));
    crate::store::write_atomic(
        &staging,
        &evidence_path,
        &serde_json::to_vec_pretty(&evidence)?,
    )?;
    match crate::store::rename_durable(&record.path, &moved) {
        Err(crate::WorkflowError::Io { source, .. })
            if source.kind() == std::io::ErrorKind::NotFound =>
        {
            return Ok(None);
        }
        other => other?,
    }
    super::super::sync_record_dirs(run_dir, &dir)?;
    super::super::sync_record_dirs(run_dir, &round_dir)?;
    tracing::warn!(
        record = %evidence.original,
        moved_to = %evidence.quarantined,
        state_known = evidence.state.is_some(),
        "damaged acceptance record quarantined"
    );
    Ok(Some(evidence))
}

/// Every quarantined record whose state is known (its evidence, else the
/// saved ledger's copy) and whose bytes did move, once per record.
fn quarantined_states(
    run_dir: &Path,
    saved: Option<&ProgressLedger>,
) -> crate::WorkflowResult<Vec<Recorded<ObservedState>>> {
    let mut states = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for round in round_dirs(run_dir)? {
        let dir = round.join(QUARANTINE_DIR);
        let mut evidence_files = entries_named(&dir, |name| name.ends_with(EVIDENCE_SUFFIX))?;
        evidence_files.sort();
        for path in evidence_files {
            let bytes = std::fs::read(&path).map_err(|e| crate::WorkflowError::io(&path, e))?;
            let Ok(evidence) = serde_json::from_slice::<QuarantinedRecordV1>(&bytes) else {
                tracing::warn!(path = %path.display(), "quarantine evidence will not parse");
                continue;
            };
            let key = (evidence.round, evidence.attempt);
            let state = (evidence.state.clone())
                .or_else(|| saved.and_then(|s| s.state_of(key.0, key.1).cloned()));
            let moved = run_dir.join(&evidence.quarantined).is_file();
            if let (Some(state), true) = (state, moved)
                && seen.insert(key)
            {
                let place = u128::from(evidence.written_nanos);
                let (round, attempt) = key;
                states.push((
                    place,
                    key,
                    ObservedState {
                        round,
                        attempt,
                        state,
                    },
                ));
            }
        }
    }
    Ok(states)
}

/// The highest attempt quarantined in the round directory `dir`: a number
/// the next record of that round never reuses.
pub(in crate::v2::acceptance_stage) fn highest_quarantined_attempt(dir: &Path) -> Option<u32> {
    (std::fs::read_dir(dir.join(QUARANTINE_DIR)).ok()?)
        .filter_map(Result::ok)
        .filter_map(|entry| attempt_of(entry.file_name().to_str()?))
        .max()
}

/// Records one `acceptance_record_quarantined` event per record in the
/// run's events, under the run lock. Best effort: the evidence file beside
/// the bytes is the durable record.
pub fn record_quarantine_events(
    store: &crate::WorkflowStore,
    run_id: &str,
    quarantined: &[QuarantinedRecordV1],
) {
    for record in quarantined {
        let detail = serde_json::to_value(record).unwrap_or_default();
        let emitted = store.with_run_lock(run_id, |locked| {
            let seq = locked.next_event_seq(run_id)?;
            crate::WorkflowEventLog::new(locked.clone()).emit(
                run_id,
                seq,
                crate::WorkflowEventKind::AcceptanceRecordQuarantined,
                detail.clone(),
            )
        });
        if let Err(error) = emitted {
            tracing::warn!(%error, record = %record.original, "quarantine event not recorded");
        }
    }
}
