//! Batch M: a refusal that rests only on landings its tree did not hold is
//! no verdict.
//!
//! A remediation verdict is shown its unit's project-data landings and must
//! judge them ([`super::project_data_landings`]). Before Batch M it was shown
//! every landing any execution of the unit's fixes ever made, including ones
//! an earlier execution made that were no longer in the project when it ran.
//! A verdict that refused because of those judged data that was not there:
//! it answered a question about another tree, and nothing it said is a
//! refusal of the tree it was dispatched on.
//!
//! Such a verdict is MISLED, and is read as no verdict at all: the
//! refused-landing revert treats what it judged as pending (never reverted
//! on its word), and a resume never replays it (it is asked again). It is
//! misled only when all of this holds:
//!
//! - it did not accept, and it names project data it judged not legitimate;
//! - every severe finding or gap it raised names that data (a refusal
//!   raised on anything else is a refusal of its own);
//! - every such answer covers only landings it was shown that the project
//!   did not hold when it was dispatched, and at least one;
//! - that is known from the host's own records: what the verdict was shown
//!   ([`record_shown`], kept at dispatch, with each file's state then), or,
//!   for a verdict dispatched before that was kept, the seed the host took
//!   of the project for its unit's fix after the landing and before the
//!   verdict, which did not hold what the landing left (a captured input
//!   change the project does not hold now either).
//!
//! A verdict that judged a landing the project did hold not legitimate, or
//! whose refusal names no project data, is never misled.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::project_data_landings::{
    ANSWER_KEY, Landing, LandingsStamp, covers, in_place, project_root, same_state,
    unit_fix_stages, unit_landings,
};
use crate::v2::WorkflowV2Status;
use crate::v2::result_store::{WorkflowV2CallRecord, WorkflowV2ResultStore};
use crate::v2::script::resume_verdict::{is_remediation_verdict, remediation_unit_key};
use crate::write_coordinator::patch_apply::{ProjectInputLanding, run_project_input_landings};
use crate::write_coordinator::project_inputs::{SeedRecord, read_json, seed_path, write_json};

/// Under the v2 store root: one file per dispatched verdict item.
const SHOWN_DIR: &str = "verdict-landings";

/// What one verdict item was shown, and when.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Shown {
    at: i64,
    #[serde(default)]
    stamp: Option<LandingsStamp>,
}

fn component(id: &str) -> String {
    let name: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if name.is_empty() || name.chars().all(|c| c == '.') {
        "_".into()
    } else {
        name
    }
}

fn shown_dir(store: &WorkflowV2ResultStore, call_id: &str) -> PathBuf {
    store.root().join(SHOWN_DIR).join(component(call_id))
}

fn now() -> i64 {
    chrono::Utc::now().timestamp_nanos_opt().unwrap_or(i64::MAX)
}

/// Keep what the verdict item `item_id` of `call_id` is shown (`None`: no
/// landing). Best effort: a verdict whose record is missing is read by the
/// seed rule, which never reads it as misled on less.
pub(crate) fn record_shown(
    store: &WorkflowV2ResultStore,
    call_id: &str,
    item_id: &str,
    stamp: Option<&LandingsStamp>,
) {
    let path = shown_dir(store, call_id).join(format!("{}.json", component(item_id)));
    let shown = Shown {
        at: now(),
        stamp: stamp.cloned(),
    };
    let _ = write_json(&path, &shown);
}

fn ns(text: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(text)
        .ok()
        .and_then(|at| at.timestamp_nanos_opt())
}

fn modified_ns(path: &Path) -> Option<i64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    let since = modified.duration_since(std::time::UNIX_EPOCH).ok()?;
    i64::try_from(since.as_nanos()).ok()
}

/// Every path the verdict judged not legitimate, wherever its result
/// carries answers (its own data, its items', its outcomes').
fn refused_answers(data: &Value) -> BTreeSet<String> {
    let mut lists: Vec<&Value> = vec![data];
    for key in ["items", "outcomes"] {
        for entry in data
            .get(key)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            lists.extend(entry.get("data"));
            lists.extend(entry.pointer("/result/data"));
        }
    }
    let mut refused = BTreeSet::new();
    for list in lists {
        for answer in (list.get(ANSWER_KEY).and_then(Value::as_array))
            .into_iter()
            .flatten()
        {
            if answer.get("legitimate").and_then(Value::as_bool) == Some(false)
                && let Some(path) = answer.get("path").and_then(Value::as_str)
            {
                refused.insert(path.to_string());
            }
        }
    }
    refused
}

/// What the verdict was shown, as kept at its dispatch: each path and
/// whether the project did not hold it then. `None`: nothing kept before
/// the verdict finished. `Some(None)`: kept, but not readable as a list.
fn kept(
    store: &WorkflowV2ResultStore,
    call_id: &str,
    finish: i64,
) -> Option<Option<Vec<(String, bool)>>> {
    let dir = shown_dir(store, call_id);
    let mut shown: Vec<Shown> = (std::fs::read_dir(&dir).ok()?.flatten())
        .filter_map(|entry| read_json::<Shown>(&entry.path()))
        .filter(|shown| shown.at < finish)
        .collect();
    if shown.is_empty() {
        return None;
    }
    let mut out = Vec::new();
    for stamp in shown.drain(..).filter_map(|shown| shown.stamp) {
        if stamp.unreadable.is_some() {
            return Some(None);
        }
        out.extend((stamp.landings.iter()).map(|landing| {
            (
                landing.path.clone(),
                !same_state(&landing.now, &landing.after),
            )
        }));
        out.extend(stamp.more.keys().map(|dir| (format!("{dir}/"), false)));
    }
    Some(Some(out))
}

/// Whether the host's seed for `landing`'s fix, taken after the landing and
/// before `finish`, shows the project not holding what it left, nothing
/// logged between that seed and `finish` put it back, and the project does
/// not hold it now. Only a captured input change (`applied`) is read so: the
/// seed holds every such file under its inputs, never a tracked one.
fn seed_shows_out(
    run_root: &Path,
    project: &Path,
    log: &[ProjectInputLanding],
    landing: &Landing,
    finish: i64,
) -> bool {
    if landing.outcome != "applied" || landing.at <= 0 || in_place(project, landing) {
        return false;
    }
    let path = seed_path(run_root, &landing.stage_id, &landing.item_id);
    let (Some(seed), Some(seeded)) = (read_json::<SeedRecord>(&path), modified_ns(&path)) else {
        return false;
    };
    let under =
        |input: &String| landing.path == *input || landing.path.starts_with(&format!("{input}/"));
    if !(landing.at < seeded && seeded < finish)
        || seed.truncated
        || !seed.inputs.iter().any(under)
        || seed.unseeded.contains_key(&landing.path)
        || seed
            .skipped
            .iter()
            .any(|(skipped, _)| *skipped == landing.path)
    {
        return false;
    }
    let state = seed
        .files
        .get(&landing.path)
        .map_or("absent", String::as_str);
    !same_state(state, &landing.after)
        && !log.iter().any(|line| {
            line.path == landing.path
                && line.landed()
                && line.at >= seeded
                && line.at < finish
                && same_state(&line.after, &landing.after)
        })
}

/// What a verdict dispatched before its view was kept was shown: its unit's
/// landings logged before it finished, each out when its seed proves it.
fn reconstructed(
    store: &WorkflowV2ResultStore,
    record: &WorkflowV2CallRecord,
    records: &[WorkflowV2CallRecord],
    finish: i64,
) -> Option<Vec<(String, bool)>> {
    let run_root = store.run_root();
    let unit = remediation_unit_key(&record.call)?;
    let stages = unit_fix_stages(records, &unit);
    let project = project_root(run_root)?;
    let landings = unit_landings(run_root, Some(&project), &stages, Some(finish)).ok()?;
    let log = run_project_input_landings(run_root).ok()?;
    Some(
        landings
            .iter()
            .map(|landing| {
                let out = seed_shows_out(run_root, &project, &log, landing, finish);
                (landing.path.clone(), out)
            })
            .collect(),
    )
}

/// Severities a refusal can rest on.
const SEVERE: &[&str] = &["high", "critical", "blocking"];

/// The text of every severe signal in `value`: each object whose
/// `severity` is severe, as its string fields joined.
fn severe_signals(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            let severe = (map.get("severity").and_then(Value::as_str)).is_some_and(|severity| {
                SEVERE
                    .iter()
                    .any(|s| severity.trim().eq_ignore_ascii_case(s))
            });
            if severe {
                out.push(
                    map.values()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join("\n"),
                );
            }
            map.values().for_each(|child| severe_signals(child, out));
        }
        Value::Array(items) => items.iter().for_each(|item| severe_signals(item, out)),
        _ => {}
    }
}

/// The refused paths, and the directory they share below the top level.
fn needles(refused: &BTreeSet<String>) -> Vec<String> {
    let mut needles: Vec<String> = (refused.iter())
        .map(|path| {
            path.trim()
                .trim_start_matches("./")
                .trim_end_matches('/')
                .to_string()
        })
        .filter(|path| !path.is_empty() && path != "*")
        .collect();
    let parts: Vec<Vec<&str>> = needles
        .iter()
        .map(|path| path.split('/').collect())
        .collect();
    if let [first, rest @ ..] = parts.as_slice()
        && !rest.is_empty()
    {
        let common = (rest.iter())
            .map(|other| first.iter().zip(other).take_while(|(a, b)| a == b).count())
            .min()
            .unwrap_or(0);
        if common >= 2 {
            let shared = first[..common].join("/");
            needles.push(shared);
        }
    }
    needles
}

/// Whether every severe signal the verdict raised is about the data it
/// refused: a severe finding or gap naming none of it is a refusal of its
/// own, and the verdict is then a verdict whatever else it was shown.
fn only_about(record: &WorkflowV2CallRecord, refused: &BTreeSet<String>) -> bool {
    let needles = needles(refused);
    let mut signals = Vec::new();
    match serde_json::to_value(&record.result) {
        Ok(result) => severe_signals(&result, &mut signals),
        Err(_) => return false,
    }
    (signals.iter()).all(|text| needles.iter().any(|needle| text.contains(needle.as_str())))
}

/// Whether `record` is a misled verdict (see the module docs).
pub fn verdict_misled(
    store: &WorkflowV2ResultStore,
    record: &WorkflowV2CallRecord,
    records: &[WorkflowV2CallRecord],
) -> bool {
    if !is_remediation_verdict(&record.call)
        || matches!(
            record.status,
            WorkflowV2Status::Accepted | WorkflowV2Status::Noop
        )
    {
        return false;
    }
    let refused = refused_answers(&record.result.data);
    if refused.is_empty() || !only_about(record, &refused) {
        return false;
    }
    // As the earlier sessions recorded it; this session's own record else.
    let finish = store
        .recorded_finish(record)
        .unwrap_or_else(|| record.finished_at.clone());
    let Some(finish) = ns(&finish) else {
        return false;
    };
    let shown = match kept(store, &record.call.id, finish) {
        Some(Some(shown)) => shown,
        Some(None) => return false,
        None => match reconstructed(store, record, records, finish) {
            Some(shown) => shown,
            None => return false,
        },
    };
    refused.iter().all(|answer| {
        let hits: Vec<bool> = (shown.iter())
            .filter(|(path, _)| covers(answer, path))
            .map(|(_, out)| *out)
            .collect();
        !hits.is_empty() && hits.iter().all(|out| *out)
    })
}

#[cfg(test)]
#[path = "misled_verdict_tests.rs"]
mod tests;
