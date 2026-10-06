//! Refusals address live identities rather than a scope an escapee left.
use super::{HostCommandGroupRecord, stall_note};

pub(super) fn known_refusal(record: &HostCommandGroupRecord, run: &str, others: usize) -> String {
    if record.pid == 0 {
        return format!(
            "fixed decomposition {run} cannot resume: host command '{}' launch is still supervised by executor {}; wait for that executor to settle its launch or teardown and resume again (barrier {})",
            record.command_id,
            record.host_pid,
            record
                .file
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_default()
        );
    }
    #[cfg(unix)]
    let mut live: Vec<_> = record
        .survivors
        .iter()
        .filter(|(pid, start)| {
            archon_shell::process_tree::identity_of(*pid).ok().flatten() == Some(*start)
        })
        .copied()
        .collect();
    // Legacy evidence may name only the session. A nested runner can move
    // to another group after the original group has ended.
    #[cfg(unix)]
    if live.is_empty() {
        let scope = archon_shell::process_tree::Scope {
            groups: vec![record.pgid],
            sessions: record.session.into_iter().collect(),
        };
        let identities = scope.members().and_then(|members| {
            let mut pins = Vec::new();
            for pid in members {
                if let Some(start) = archon_shell::process_tree::identity_of(pid)? {
                    pins.push((pid, start));
                }
            }
            Ok(pins)
        });
        match identities {
            Ok(pins) => live.extend(pins),
            Err(error) => {
                return format!(
                    "fixed decomposition {run} cannot resume: host command '{}' membership cannot be read ({error}); verify its processes, then remove {} and resume again",
                    record.command_id,
                    record
                        .file
                        .as_ref()
                        .map(|path| super::refusal_files(path))
                        .unwrap_or_default()
                );
            }
        }
    }
    #[cfg(windows)]
    let live: Vec<_> = record
        .survivors
        .iter()
        .filter(|(pid, start)| {
            archon_shell::job_object::identity_of(*pid).ok().flatten() == Some(*start)
        })
        .copied()
        .collect();
    #[cfg(not(any(unix, windows)))]
    let live: Vec<(u32, u64)> = Vec::new();
    let remedy = if !live.is_empty() {
        let targets = live
            .iter()
            .map(|(pid, start)| {
                #[cfg(unix)]
                let action = format!("kill -TERM {pid}");
                #[cfg(windows)]
                let action = format!("Stop-Process -Id {pid}");
                #[cfg(not(any(unix, windows)))]
                let action = format!("stop pid {pid}");
                format!("pid {pid} (creation time {start}; verify this identity before {action})")
            })
            .collect::<Vec<_>>()
            .join(", ");
        format!("stop the recorded live survivors: {targets}, then resume again")
    } else {
        #[cfg(unix)]
        let action =
            "the membership changed during this check; resume again to recheck the recorded scope"
                .to_string();
        #[cfg(windows)]
        let action = format!(
            "stop the remaining members of Windows Job Object {:?} and resume again",
            record.job
        );
        #[cfg(not(any(unix, windows)))]
        let action = "verify the recorded processes have exited and resume again".to_string();
        action
    };
    format!(
        "fixed decomposition {run} cannot resume: host command '{}' (process group {}, pid {}) is still running{}{}; {remedy}",
        record.command_id,
        record.pgid,
        record.pid,
        stall_note(record),
        if others == 0 {
            String::new()
        } else {
            format!(", with {others} other group(s)")
        }
    )
}
