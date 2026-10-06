//! Reserve an attempt before any evidence is written. Reservations survive
//! cancellation and crashes: an abandoned number is never reused.
use super::*;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const MARKER: &str = "reservation.json";
#[derive(Serialize, Deserialize)]
struct Marker {
    token: String,
    frontier: u64,
}

pub struct RoundReservation {
    pub round: u32,
    pub attempt: u32,
    pub frontier: u64,
    token: String,
}

pub fn reserve_round(run_dir: &Path, round: u32) -> WorkflowResult<RoundReservation> {
    progress::under_order_lock(run_dir, || {
        crate::stage_write::with_write(|| {
            let parent = round_dir(run_dir, round);
            let mut highest = 0u32;
            for directory in [&parent, &parent.join("quarantine")] {
                let entries = match std::fs::read_dir(directory) {
                    Ok(entries) => entries,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(error) => return Err(WorkflowError::io(directory, error)),
                };
                for entry in entries {
                    let entry = entry.map_err(|e| WorkflowError::io(directory, e))?;
                    let name = entry.file_name();
                    let number = name
                        .to_str()
                        .and_then(|name| name.strip_prefix("attempt-"))
                        .and_then(|name| name.split('.').next())
                        .and_then(|number| number.parse::<u32>().ok());
                    if let Some(number) = number {
                        highest = highest.max(number);
                    }
                }
            }
            let attempt = highest.checked_add(1).ok_or_else(|| {
                WorkflowError::StateCorrupt("acceptance attempt numbers exhausted".into())
            })?;
            let frontier = progress::frontier_locked(run_dir)?;
            let token = uuid::Uuid::new_v4().to_string();
            let dir = parent.join(format!("attempt-{attempt:02}"));
            std::fs::create_dir_all(&parent).map_err(|e| WorkflowError::io(&parent, e))?;
            std::fs::create_dir(&dir).map_err(|e| WorkflowError::io(&dir, e))?;
            crate::store::write_atomic(
                &dir.join(".reservation.tmp"),
                &dir.join(MARKER),
                &serde_json::to_vec_pretty(&Marker {
                    token: token.clone(),
                    frontier,
                })?,
            )?;
            sync_record_dirs(run_dir, &dir)?;
            Ok(RoundReservation {
                round,
                attempt,
                frontier,
                token,
            })
        })
    })
}

fn marker(run_dir: &Path, round: u32, attempt: u32) -> WorkflowResult<Option<Marker>> {
    let path = round_dir(run_dir, round)
        .join(format!("attempt-{attempt:02}"))
        .join(MARKER);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(WorkflowError::io(&path, error)),
    };
    serde_json::from_slice(&bytes).map(Some).map_err(|error| {
        WorkflowError::StateCorrupt(format!(
            "acceptance reservation {} will not parse: {error}",
            path.display()
        ))
    })
}

pub(super) fn frontier(run_dir: &Path, round: u32, attempt: u32) -> WorkflowResult<Option<u64>> {
    Ok(marker(run_dir, round, attempt)?.map(|marker| marker.frontier))
}

impl RoundReservation {
    /// Land at the reserved number, checking the reservation under the same
    /// order lock as the ownership check and landing. Never renumber evidence.
    pub fn record<T, E: From<WorkflowError>>(
        &self,
        run_dir: &Path,
        record: &mut AcceptanceRoundRecordV1,
        act: impl FnOnce(&mut AcceptanceRoundRecordV1, &mut RoundLanding<'_>) -> Result<T, E>,
    ) -> Result<(PathBuf, T), E> {
        progress::under_order_lock(run_dir, || {
            let marker = marker(run_dir, self.round, self.attempt)?;
            if record.round != self.round
                || record.attempt != self.attempt
                || !marker.is_some_and(|marker| {
                    marker.token == self.token && marker.frontier == self.frontier
                })
            {
                return Err(WorkflowError::StateCorrupt(
                    "acceptance record does not own its evidence reservation".into(),
                )
                .into());
            }
            let dir = round_dir(run_dir, self.round);
            let path = dir.join(attempt_file_name(self.attempt));
            if path.try_exists().map_err(|e| WorkflowError::io(&path, e))?
                || progress::quarantined_attempt(&dir, self.attempt)?
            {
                return Err(WorkflowError::StateCorrupt(
                    "reserved acceptance record already landed or was quarantined".into(),
                )
                .into());
            }
            record.progress_frontier = Some(self.frontier);
            super::round_landing::land_at_locked(run_dir, &dir, record, act)
        })
    }
}

#[cfg(test)]
#[path = "acceptance_round_reservation_tests.rs"]
mod tests;
