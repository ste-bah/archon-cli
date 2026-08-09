//! Replay of cognitive writes that reached the ledger but not the database.
//!
//! There is exactly one place in this crate where a record can survive as a
//! file line while its relation row is missing: [`crate::ReflectionWriter`]
//! writes the reflection to Cozo, *records the failure as degraded rather than
//! aborting*, and then appends the ledger regardless. That gap is the
//! dead-letter queue. Everywhere else (decisions, metric events) the relation
//! is written first and a failure propagates before anything is appended, so
//! those files cannot run ahead of their relations.
//!
//! `replay` therefore reports a real count: `0` means the ledger and the
//! relation agree, which is a measurement, not a placeholder.

use std::collections::BTreeSet;
use std::io::{BufRead, BufReader};
use std::path::Path;

use cozo::{DbInstance, ScriptMutability};

use crate::cozo_guard::run_script_guarded;
use crate::reflection_store::put_reflection;
use crate::{CognitiveError, ReflectionRecord};

const REFLECTION_LEDGER: &str = "cognitive-reflections.jsonl";

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DeadLetterReplay {
    pub replayed: u64,
    /// Ledger lines that could not be parsed. Counted, not silently skipped:
    /// an unparseable line is a record we have permanently lost, and the tick
    /// audit is the only place that would ever say so.
    pub unparseable: u64,
    pub errors: Vec<String>,
}

/// Re-put every ledgered reflection whose relation row is missing.
pub fn replay(db: &DbInstance, ledger_dir: &Path) -> Result<DeadLetterReplay, CognitiveError> {
    let mut report = DeadLetterReplay::default();
    let path = ledger_dir.join(REFLECTION_LEDGER);
    if !path.exists() {
        // No ledger is a genuine empty queue, not an unmeasured one.
        return Ok(report);
    }
    let stored = stored_reflection_ids(db)?;
    let file = std::fs::File::open(&path)?;
    // Deduplicated because the ledger is append-only: a reflection re-written
    // after a transient failure appears twice and must count once.
    let mut replayed = BTreeSet::new();
    for line in BufReader::new(file).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let Ok(reflection) = serde_json::from_str::<ReflectionRecord>(&line) else {
            report.unparseable += 1;
            continue;
        };
        if stored.contains(&reflection.reflection_id)
            || !replayed.insert(reflection.reflection_id.clone())
        {
            continue;
        }
        match put_reflection(db, &reflection) {
            Ok(()) => report.replayed += 1,
            Err(error) => report
                .errors
                .push(format!("dead_letter_replay_failed:{error}")),
        }
    }
    Ok(report)
}

fn stored_reflection_ids(db: &DbInstance) -> Result<BTreeSet<String>, CognitiveError> {
    let rows = run_script_guarded(
        db,
        "?[reflection_id] := *cognitive_reflections{reflection_id}",
        Default::default(),
        ScriptMutability::Immutable,
        "list stored reflection ids",
    )?;
    Ok(rows
        .rows
        .iter()
        .filter_map(|row| row.first().and_then(cozo::DataValue::get_str))
        .map(str::to_string)
        .collect())
}
