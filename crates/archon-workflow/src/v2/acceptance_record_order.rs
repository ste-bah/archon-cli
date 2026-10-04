//! The order the acceptance round records were written in (Issue 262).
//!
//! A resume restarts the loop at round 1, so (round, attempt) is not the
//! order the records were written in. The append-only recording-order log
//! is: one entry per record, appended (and synced) before the record is
//! written. The log's own order is authoritative. Each entry carries a
//! sequence number one past every place already in the log; an entry of an
//! earlier format (`round attempt nanos`, or `round attempt`) has none and
//! takes its 1-based line number, which is its place in an append-only log.
//! Wall-clock time never orders a logged record: a clock stepped back, or a
//! coarse file time (2 s on FAT), cannot reorder them. Only a record with no
//! entry (a failed append) is placed by its own file time, against the file
//! times of the logged records.

use std::collections::BTreeMap;
use std::path::Path;

use super::super::{ACCEPTANCE_RECORDS_DIR, AcceptanceRoundRecordV1};

/// The log's file, under the acceptance records directory.
const RECORDING_ORDER_FILE: &str = "recording-order.log";

/// The sequence number's field, first on a line this writer appends.
const SEQ_FIELD: &str = "seq=";

/// The field that ends a line this writer appends: a line cut short by a
/// crash (`seq=7 3 1` of `seq=7 3 12 end`) lacks it and names no record.
const END_FIELD: &str = "end";

/// An entry's place: its sequence number, then its line (a tie only when
/// two writers raced for one number).
type Place = (u64, usize);

/// The (round, attempt) a line names, and its sequence number when it has
/// one; `None` for a line no writer of the log wrote whole.
fn parse(line: &str) -> Option<(Option<u64>, u32, u32)> {
    let fields: Vec<&str> = line.split_whitespace().collect();
    let number = |field: &str| field.parse::<u32>().ok();
    match fields[..] {
        [seq, round, attempt, END_FIELD] => {
            let seq = seq.strip_prefix(SEQ_FIELD)?.parse::<u64>().ok()?;
            Some((Some(seq), number(round)?, number(attempt)?))
        }
        [round, attempt, nanos] => {
            nanos.parse::<u128>().ok()?;
            Some((None, number(round)?, number(attempt)?))
        }
        [round, attempt] => Some((None, number(round)?, number(attempt)?)),
        _ => None,
    }
}

/// Every entry of `text` with its place, in line order.
fn entries(text: &str) -> impl Iterator<Item = ((u32, u32), Place)> + '_ {
    (text.lines().enumerate()).filter_map(|(index, line)| {
        let (seq, round, attempt) = parse(line)?;
        let line_number = index + 1;
        let seq = seq.unwrap_or(line_number as u64);
        Some(((round, attempt), (seq, line_number)))
    })
}

/// The sequence number the next entry takes: one past every place in the
/// log, implicit line numbers included, so it follows every older entry.
fn next_seq(text: &str) -> u64 {
    let lines = text.lines().count() as u64;
    (entries(text).map(|(_, (seq, _))| seq))
        .fold(lines, u64::max)
        .saturating_add(1)
}

fn log_path(run_dir: &Path) -> std::path::PathBuf {
    run_dir
        .join(ACCEPTANCE_RECORDS_DIR)
        .join(RECORDING_ORDER_FILE)
}

/// Appends the entry of (`round`, `attempt`) to the log and syncs it and
/// the directories above it (the log, or the records directory, may be
/// new). Best effort: a record the log misses is still read, placed by its
/// own file time.
pub(in crate::v2::acceptance_stage) fn note_recorded(run_dir: &Path, round: u32, attempt: u32) {
    let path = log_path(run_dir);
    let appended = append(&path, round, attempt).map_err(|e| crate::WorkflowError::io(&path, e));
    let synced = appended.and_then(|()| match path.parent() {
        Some(dir) => super::super::sync_record_dirs(run_dir, dir),
        None => Ok(()),
    });
    if let Err(error) = synced {
        tracing::warn!(%error, path = %path.display(), "acceptance recording order not appended");
    }
}

fn append(path: &Path, round: u32, attempt: u32) -> std::io::Result<()> {
    use std::io::{Read, Write};
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut file = (std::fs::OpenOptions::new())
        .create(true)
        .read(true)
        .append(true)
        .open(path)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let text = String::from_utf8_lossy(&bytes);
    let seq = next_seq(&text);
    // A line cut short by a crash is ended first, never joined.
    let lead = if text.is_empty() || text.ends_with('\n') {
        ""
    } else {
        "\n"
    };
    writeln!(file, "{lead}{SEQ_FIELD}{seq} {round} {attempt} {END_FIELD}")?;
    file.sync_all()
}

/// Each logged (round, attempt) with its place. The LATEST entry wins, so
/// a write retried after a failed one takes its new place; an entry whose
/// record never landed matches no record and is ignored. No log yet is no
/// entry; a log that exists and cannot be read is an error, never a silent
/// fall back to file times.
fn places(run_dir: &Path) -> crate::WorkflowResult<BTreeMap<(u32, u32), Place>> {
    let path = log_path(run_dir);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(crate::WorkflowError::io(path, error)),
    };
    let mut places = BTreeMap::new();
    for (key, place) in entries(&String::from_utf8_lossy(&bytes)) {
        let latest = places.entry(key).or_insert(place);
        *latest = place.max(*latest);
    }
    Ok(places)
}

/// `records` (each with its file time, epoch nanoseconds) in the order they
/// were written: the logged ones in log order; each unlogged one (by file
/// time, then round and attempt) just before the first logged record whose
/// file time is later than its own, else at the end.
pub(super) fn in_recorded_order(
    run_dir: &Path,
    records: Vec<(u128, AcceptanceRoundRecordV1)>,
) -> crate::WorkflowResult<Vec<AcceptanceRoundRecordV1>> {
    let places = places(run_dir)?;
    let (mut logged, mut unlogged): (Vec<_>, Vec<_>) = (records.into_iter())
        .map(|(written, record)| {
            let place = places.get(&(record.round, record.attempt)).copied();
            (place, written, record)
        })
        .partition(|(place, _, _)| place.is_some());
    logged.sort_by_key(|(place, _, _)| *place);
    unlogged.sort_by_key(|(_, written, record)| (*written, record.round, record.attempt));
    let mut unlogged = unlogged.into_iter().peekable();
    let mut ordered = Vec::with_capacity(logged.len() + unlogged.len());
    for (_, written, record) in logged {
        while let Some((_, _, earlier)) = unlogged.next_if(|(_, at, _)| *at < written) {
            ordered.push(earlier);
        }
        ordered.push(record);
    }
    ordered.extend(unlogged.map(|(_, _, record)| record));
    Ok(ordered)
}
