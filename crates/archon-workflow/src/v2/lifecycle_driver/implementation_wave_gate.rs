// The wave-silence invariant.
//
// #163 failure 3: `implementation-wave-1` ran, its worktree held no branch
// directory, not one file was written, and the run carried on to remediation as
// though implementation had happened. Nothing on the acceptance path asked the
// question. `has_concrete_evidence` is satisfied by a single `commands_run`
// entry and never looks at `files_changed`; the per-dependency-wave gate takes
// the wave result as `_wave` and does not read it.
//
// This asks it, once, at the wave boundary. It is deliberately NOT a widening
// of `has_concrete_evidence`: that predicate is shared by no-op proofs and by
// verification, where "did anything get written" is the wrong question and
// answering it would change what those stages accept.

use serde_json::Value;

use super::support;

/// Keys a branch uses to claim it changed something on disk.
const WRITE_CLAIM_KEYS: [&str; 4] = [
    "files_changed",
    "changed_files",
    "artifacts",
    "artifact_paths",
];

/// Did every branch of this wave finish without leaving a trace?
///
/// Four things count as a trace, in descending order of authority:
///
/// - `patch_landed` — the write coordinator's own verdict, measured by
///   `worktree_patch_landed` against the branch's DECLARED BASELINE and stamped
///   on every worktree branch result whatever its status. It is the one signal
///   here a branch cannot assert its way into, and until now only the JS
///   prelude read it;
/// - a non-empty `files_changed` / `changed_files`, in either of the two
///   spellings the fan-out reports it in (see [`branch_records`]);
/// - a non-empty `artifacts` / `artifact_paths`;
/// - `idempotent_noop` — an explicit declaration that the branch looked at the
///   work and found nothing that needed writing.
///
/// The last is why this is a silence gate rather than a write gate. A wave of
/// genuinely declared no-ops has done its job and passes here; the write
/// coordinator has already refused the dishonest version of that claim
/// (`patch is empty and item did not declare idempotent_noop`). A wave that
/// says nothing at all has not done its job, and that is the case this stops.
///
/// A wave with no branch record at all is silence too. `outcomes_of` hands back
/// the bare envelope as a single outcome rather than an empty list precisely so
/// it cannot pass a gate unexamined, and that envelope carries no trace either —
/// which is the right answer, because `run_implementation_wave` is only reached
/// with a non-empty ready-item list, so a fan-out that came back with nothing
/// did not do the work.
pub(super) fn wave_left_no_trace(wave: &Value) -> bool {
    !branch_records(wave).iter().any(branch_left_a_trace)
}

/// Everything the wave says about what its branches did.
///
/// A write fan-out reports the same wave twice. `items` holds each branch's full
/// result, which is the only place `files_changed`, `artifacts` and the
/// coordinator's `patch_landed` survive intact; `outcomes` holds a trimmed
/// per-branch view that keeps those same facts as typed `evidence` entries and
/// drops the arrays. `outcomes_of` prefers `outcomes`, so reading only what it
/// returns would call a wave that wrote five files silent. Both are read, and
/// the overlap costs nothing: one trace anywhere answers the question.
fn branch_records(wave: &Value) -> Vec<Value> {
    let mut records = support::outcomes_of(wave);
    for envelope in [Some(wave), wave.get("result"), wave.get("data")]
        .into_iter()
        .flatten()
    {
        records.extend(support::array(envelope.get("items")));
    }
    records
}

fn branch_left_a_trace(outcome: &Value) -> bool {
    trace_roots(outcome).into_iter().any(|root| {
        declared(root, "patch_landed")
            || declared(root, "idempotent_noop")
            || WRITE_CLAIM_KEYS
                .iter()
                .any(|key| !support::array(root.get(*key)).is_empty())
            || support::array(root.get("evidence"))
                .iter()
                .any(is_changed_file_evidence)
    })
}

/// The outcome view's spelling of a `files_changed` entry: the same record,
/// flattened into the typed evidence list instead of kept as its own array.
/// Matched on the exact kind rather than on "there is evidence" — a branch that
/// only inspected has evidence too, and that is the case this gate is for.
fn is_changed_file_evidence(evidence: &Value) -> bool {
    evidence.get("kind").and_then(Value::as_str) == Some("file_changed")
}

/// A branch result reaches the lifecycle either bare or wrapped in `result`,
/// and the write coordinator stamps its markers on `data` rather than at the
/// top level — so all four places are read instead of assuming one envelope
/// and reading a real trace as silence.
fn trace_roots(outcome: &Value) -> Vec<&Value> {
    let nested = outcome.get("result");
    [
        Some(outcome),
        nested,
        outcome.get("data"),
        nested.and_then(|result| result.get("data")),
    ]
    .into_iter()
    .flatten()
    .collect()
}

fn declared(root: &Value, key: &str) -> bool {
    root.get(key).and_then(Value::as_bool) == Some(true)
}

#[cfg(test)]
#[path = "implementation_wave_gate_tests.rs"]
mod tests;
