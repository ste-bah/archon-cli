//! The order the acceptance round records were written in (Issue 262).
//!
//! A resume restarts the loop at round 1, so (round, attempt) is not the
//! order the records were written in. The append-only recording-order log
//! is: one entry per record, appended (and synced) before the record is
//! written. The order the entries landed in the log is authoritative: an
//! append (`O_APPEND`) lands after every entry before it, whatever number
//! it carries, so a log a racing writer left with a number repeated still
//! replays right. Entries are `seq=<n> round attempt end`; the earlier
//! formats (`round attempt nanos`, `round attempt`) are read too.
//!
//! Writers append under an advisory lock on a sibling lock file (several
//! processes can record at once: a round of an obsolete generation still
//! finishing beside the resumed owner's), held from the check that their
//! attempt is free until their record has landed (round 9), read the log
//! only while they hold it, give the entry a number past every number already in the log, and
//! write it with ONE write of the whole line. A last line with no newline
//! was cut short by a crash: it is never an entry, in any format, and the
//! next append marks it torn before writing its own line.
//!
//! Wall-clock time never orders a logged record: a clock stepped back, or a
//! coarse file time (2 s on FAT), cannot reorder them. Only a record with no
//! entry (a failed append) is placed by its own file time, against the file
//! times of the logged records.

use std::collections::BTreeMap;
use std::path::Path;

use super::super::ACCEPTANCE_RECORDS_DIR;

/// The log's file, under the acceptance records directory.
const RECORDING_ORDER_FILE: &str = "recording-order.log";

/// The advisory lock every append holds, beside the log. A file of its own:
/// Windows locks are mandatory, and a lock on the log itself would refuse
/// the readers.
const RECORDING_ORDER_LOCK: &str = "recording-order.lock";

/// The sequence number's field, first on a line this writer appends.
const SEQ_FIELD: &str = "seq=";

/// The field that ends a line this writer appends: a line cut short by a
/// crash (`seq=7 3 1` of `seq=7 3 12 end`) lacks it and names no record.
const END_FIELD: &str = "end";

/// What the next append adds to a last line cut short, before its own
/// line: the fragment becomes a whole line no format reads as an entry.
const TORN_MARK: &str = " torn\n";

/// A record to place: its file time (epoch nanoseconds), its (round,
/// attempt), and what the caller replays.
pub(super) type Recorded<T> = (u128, (u32, u32), T);

/// The (round, attempt) a whole line names; `None` for a line no writer of
/// the log wrote whole.
fn parse(line: &str) -> Option<(u32, u32)> {
    let fields: Vec<&str> = line.split_whitespace().collect();
    let number = |field: &str| field.parse::<u32>().ok();
    match fields[..] {
        [seq, round, attempt, END_FIELD] => {
            seq.strip_prefix(SEQ_FIELD)?.parse::<u64>().ok()?;
            Some((number(round)?, number(attempt)?))
        }
        [round, attempt, nanos] => {
            nanos.parse::<u128>().ok()?;
            Some((number(round)?, number(attempt)?))
        }
        [round, attempt] => Some((number(round)?, number(attempt)?)),
        _ => None,
    }
}

/// Every entry of `text` with its 1-based line, in landing order. Only a
/// line ended by a newline can be one.
fn entries(text: &str) -> impl Iterator<Item = ((u32, u32), usize)> + '_ {
    (text.split_inclusive('\n').enumerate()).filter_map(|(index, line)| {
        let key = parse(line.strip_suffix('\n')?)?;
        Some((key, index + 1))
    })
}

/// The sequence number the next entry takes: past every number any line
/// carries (a torn one too) and past the line count, so it is above every
/// place already in the log.
fn next_seq(text: &str) -> u64 {
    (text.lines())
        .filter_map(|line| {
            let first = line.split_whitespace().next()?;
            first.strip_prefix(SEQ_FIELD)?.parse::<u64>().ok()
        })
        .fold(text.lines().count() as u64, u64::max)
        .saturating_add(1)
}

fn log_path(run_dir: &Path) -> std::path::PathBuf {
    run_dir
        .join(ACCEPTANCE_RECORDS_DIR)
        .join(RECORDING_ORDER_FILE)
}

/// Runs `act` holding the recording-order lock: the writer of a record
/// decides whether its attempt is free, appends its entry and lands the
/// record under it, so two writers of one attempt can never both succeed.
/// A lock that cannot be taken is an error, never a write without it.
pub(in crate::v2::acceptance_stage) fn under_order_lock<T, E: From<crate::WorkflowError>>(
    run_dir: &Path,
    act: impl FnOnce() -> Result<T, E>,
) -> Result<T, E> {
    let path = log_path(run_dir);
    let dir = path.parent().unwrap_or(Path::new("."));
    let lock_path = dir.join(RECORDING_ORDER_LOCK);
    let io = |error| E::from(crate::WorkflowError::io(&lock_path, error));
    std::fs::create_dir_all(dir).map_err(|e| E::from(crate::WorkflowError::io(dir, e)))?;
    let lock_file = (std::fs::OpenOptions::new())
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .map_err(io)?;
    let mut lock = fd_lock::RwLock::new(lock_file);
    let _held = lock.write().map_err(io)?;
    act()
}

/// Appends the entry of (`round`, `attempt`) to the log and syncs it and
/// the directories above it (the log, or the records directory, may be
/// new). The caller holds the order lock ([`under_order_lock`]). Best
/// effort: a record the log misses is still read, placed by its own file
/// time.
pub(in crate::v2::acceptance_stage) fn note_recorded_locked(
    run_dir: &Path,
    round: u32,
    attempt: u32,
) {
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

/// [`note_recorded_locked`], taking the order lock itself.
#[cfg(test)]
pub(in crate::v2::acceptance_stage) fn note_recorded(run_dir: &Path, round: u32, attempt: u32) {
    let noted = under_order_lock(run_dir, || {
        note_recorded_locked(run_dir, round, attempt);
        crate::WorkflowResult::Ok(())
    });
    noted.expect("the order lock is taken");
}

/// One whole line appended; the caller holds the order lock.
fn append(path: &Path, round: u32, attempt: u32) -> std::io::Result<()> {
    use std::io::{Read, Write};
    let mut file = (std::fs::OpenOptions::new())
        .create(true)
        .read(true)
        .append(true)
        .open(path)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let text = String::from_utf8_lossy(&bytes);
    let torn = if text.is_empty() || text.ends_with('\n') {
        ""
    } else {
        TORN_MARK
    };
    let line = format!(
        "{torn}{SEQ_FIELD}{} {round} {attempt} {END_FIELD}\n",
        next_seq(&text)
    );
    // One write of the whole line: a short one leaves a fragment with no
    // newline, which is never an entry and is marked torn by the next.
    let written = file.write(line.as_bytes())?;
    if written != line.len() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::WriteZero,
            format!("wrote {written} of {} bytes of an order entry", line.len()),
        ));
    }
    file.sync_all()
}

/// Each logged (round, attempt) with the line of its LATEST entry, so a
/// write retried after a failed one takes its new place; an entry whose
/// record never landed matches no record and is ignored. No log yet is no
/// entry; a log that exists and cannot be read is an error, never a silent
/// fall back to file times.
fn places(run_dir: &Path) -> crate::WorkflowResult<BTreeMap<(u32, u32), usize>> {
    let path = log_path(run_dir);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(crate::WorkflowError::io(path, error)),
    };
    let mut places = BTreeMap::new();
    for (key, line) in entries(&String::from_utf8_lossy(&bytes)) {
        let latest = places.entry(key).or_insert(line);
        *latest = line.max(*latest);
    }
    Ok(places)
}

/// `records` in the order they were written: the logged ones in log order;
/// each unlogged one (by file time, then round and attempt) just before the
/// first logged record whose file time is later than its own, else at the
/// end.
pub(super) fn in_recorded_order<T>(
    run_dir: &Path,
    records: Vec<Recorded<T>>,
) -> crate::WorkflowResult<Vec<T>> {
    let places = places(run_dir)?;
    let (mut logged, mut unlogged): (Vec<_>, Vec<_>) = (records.into_iter())
        .map(|(written, key, record)| (places.get(&key).copied(), written, key, record))
        .partition(|(place, ..)| place.is_some());
    logged.sort_by_key(|(place, ..)| *place);
    unlogged.sort_by_key(|(_, written, key, _)| (*written, *key));
    let mut unlogged = unlogged.into_iter().peekable();
    let mut ordered = Vec::with_capacity(logged.len() + unlogged.len());
    for (_, written, _, record) in logged {
        while let Some((.., earlier)) = unlogged.next_if(|(_, at, ..)| *at < written) {
            ordered.push(earlier);
        }
        ordered.push(record);
    }
    ordered.extend(unlogged.map(|(.., record)| record));
    Ok(ordered)
}

#[cfg(test)]
#[path = "acceptance_record_order_tests.rs"]
mod tests;
