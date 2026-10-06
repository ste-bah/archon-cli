//! Recording a round for its owner (Issue 316): the attempt number is
//! settled, the record decided and its owner checked, and the record
//! landed, all under the recording-order lock.

use std::path::{Path, PathBuf};

use super::{
    AcceptanceRoundRecordV1, WorkflowError, WorkflowResult, attempt_taken, land_locked,
    next_attempt, progress, round_dir,
};

/// Issue 316: records `record` for the round's owner, as the next FREE
/// attempt of its round, under the recording-order lock held until the
/// record has landed. When another writer took its attempt meanwhile (the
/// number was chosen when the round started), the record takes the next
/// free one instead, so the owner is never refused for a number. Then `act`
/// runs on the record with the number it lands as: it settles what the
/// record says from the history it now sees (the other writer's record
/// included) and lands it through `landing`, inside whatever lock its
/// ownership check needs, so no change of owner falls between the check
/// and the landing. An `Err` from `act`, or an `act` that does not land,
/// lands nothing.
pub fn record_round<T, E: From<WorkflowError>>(
    run_dir: &Path,
    record: &mut AcceptanceRoundRecordV1,
    act: impl FnOnce(&mut AcceptanceRoundRecordV1, &mut RoundLanding<'_>) -> Result<T, E>,
) -> Result<(PathBuf, T), E> {
    let dir = round_dir(run_dir, record.round);
    std::fs::create_dir_all(&dir).map_err(|source| WorkflowError::io(&dir, source))?;
    progress::under_order_lock(run_dir, || {
        if attempt_taken(&dir, record.attempt)? {
            let wanted = record.attempt;
            record.attempt = next_attempt(run_dir, record.round);
            if attempt_taken(&dir, record.attempt)? {
                return Err(WorkflowError::StateCorrupt(format!(
                    "acceptance round {} has no free attempt past {}",
                    record.round, record.attempt
                ))
                .into());
            }
            tracing::warn!(
                round = record.round,
                wanted,
                attempt = record.attempt,
                "another writer took this acceptance attempt; the record takes the next free one"
            );
        }
        let mut landing = RoundLanding {
            run_dir,
            dir: &dir,
            attempt: record.attempt,
            landed: None,
        };
        let acted = act(record, &mut landing)?;
        let path = landing.landed.ok_or_else(|| {
            WorkflowError::StateCorrupt(format!(
                "acceptance round {} attempt {} was decided but never landed",
                record.round, record.attempt
            ))
        })?;
        Ok((path, acted))
    })
}

/// The one landing [`record_round`] hands its `act`, under the order lock:
/// the attempt it checked free, landed at most once.
pub struct RoundLanding<'a> {
    run_dir: &'a Path,
    dir: &'a Path,
    attempt: u32,
    landed: Option<PathBuf>,
}

impl RoundLanding<'_> {
    /// Lands `record` as the attempt the order lock checked free.
    pub fn land(&mut self, record: &AcceptanceRoundRecordV1) -> WorkflowResult<PathBuf> {
        if self.landed.is_some() || record.attempt != self.attempt {
            return Err(WorkflowError::StateCorrupt(format!(
                "acceptance round {} may land attempt {} once, not attempt {}",
                record.round, self.attempt, record.attempt
            )));
        }
        let path = land_locked(self.run_dir, self.dir, record)?;
        self.landed = Some(path.clone());
        Ok(path)
    }
}
