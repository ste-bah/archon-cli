//! Issue 422: a resume after the executor was SIGKILLed mid-command proves
//! the recorded scope empty instead of asking an operator to verify it.
use super::{HostCommandGroupRecord, record_running};

/// What `record` still runs: probed over its scope when its executor
/// crashed mid-command (`crashed`), else exactly [`record_running`].
pub(super) fn probe(record: &mut HostCommandGroupRecord, crashed: bool) -> Option<bool> {
    if crashed {
        crashed_scope_running(record)
    } else {
        record_running(record)
    }
}

/// Issue 422: whether anything the record of a crashed executor names still
/// runs, probed rather than assumed. The probe is [`record_running`] over
/// the recorded scope: every pinned identity (pid and start time), the
/// leader (a stranger holding its pid with another start time proves the
/// group and session ended), the process group and the session. `Some(false)`
/// only when that probe proves the scope empty; then, and when a member is
/// found, the record's survivors are known (`survivors_unknown` false), so a
/// refusal names the live member by pid and start time. `None` (a probe that
/// could not be read, or a platform without one) keeps them unknown.
fn crashed_scope_running(record: &mut HostCommandGroupRecord) -> Option<bool> {
    if !cfg!(unix) {
        return None;
    }
    let probed = record_running(&HostCommandGroupRecord {
        survivors_unknown: false,
        ..record.clone()
    })?;
    record.survivors_unknown = false;
    Some(probed)
}
