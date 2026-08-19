// Used only by the unix descendant-cleanup path below; on Windows the import
// is dead and trips `-D warnings`.
#[cfg(unix)]
use std::collections::HashMap;

use tokio::process::Command;

use super::BASH_PROGRAM;

const UNIX_DESCENDANT_CLEANUP: &str = r#"
__archon_children() {
    ps -axo pid=,ppid= 2>/dev/null | awk -v parent="$1" '$2 == parent { print $1 }'
}
__archon_kill_tree() {
    for child in $(__archon_children "$1"); do
        kill -STOP "$child" 2>/dev/null || true
        __archon_kill_tree "$child"
    done
    kill -KILL "$1" 2>/dev/null || true
}
__archon_cleanup() {
    __archon_status=$?
    trap - EXIT
    for child in $(__archon_children "$$"); do
        kill -STOP "$child" 2>/dev/null || true
        __archon_kill_tree "$child"
    done
    exit "$__archon_status"
}
trap __archon_cleanup EXIT
(
    "$1" -c "$2"
    __archon_inner_status=$?
    exit "$__archon_inner_status"
)
exit $?
"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BashContainment {
    ProcessGroup,
}

pub(super) fn containment_for_platform(_platform: &str) -> BashContainment {
    BashContainment::ProcessGroup
}

pub(super) fn contained_bash_command(command_text: &str) -> Command {
    let mut command = Command::new(BASH_PROGRAM.as_path());
    match containment_for_platform(std::env::consts::OS) {
        BashContainment::ProcessGroup if cfg!(unix) => {
            // Process groups provide portable best-effort descendant cleanup,
            // not a security sandbox. Linux adds subreaper tracking below for
            // session-detached descendants; other Unix platforms cannot
            // guarantee cleanup after a deliberate setsid(2) escape.
            configure_own_process_group(&mut command);
            configure_linux_subreaper(&mut command);
            command
                .arg("-c")
                .arg(UNIX_DESCENDANT_CLEANUP)
                .arg("archon-bash-guard")
                .arg(BASH_PROGRAM.as_os_str())
                .arg(command_text);
        }
        BashContainment::ProcessGroup => {
            command.arg("-c").arg(command_text);
        }
    }
    command
}

/// Put the child in a process group of its own, with itself as leader.
///
/// This module is named for process-group containment and, until #192, never
/// created one. The child inherited the caller's group, so `child.id()` — which
/// `terminate_completed_process_group` passes to `kill(-pgid)` — was a plain
/// pid that usually named no group at all.
///
/// Two consequences, and the second is the serious one:
///
/// 1. The kill was a no-op. `kill(-pid, SIGKILL)` returned `ESRCH`, which the
///    cleanup path treats as "the group is already gone", so it reported
///    success. Descendant cleanup rested entirely on the shell's `EXIT` trap.
/// 2. A pid is only "usually" not a pgid. On a busy machine it can collide with
///    a real process group belonging to something else, and then the kill lands
///    on processes archon does not own. macOS reported that as `EPERM` and CI
///    went red — the kernel refusing the signal is the bug reporting itself,
///    not a platform quirk to be worked around.
///
/// With `process_group(0)` the child's pid *is* its pgid, so the kill can only
/// ever reach descendants of this command, and `ESRCH` genuinely means the
/// group has drained.
///
/// The child is not interactive — its stdin is `/dev/null` and cancellation
/// arrives over the `CancellationToken`, not as a terminal `SIGINT` — so
/// leaving the caller's group costs nothing.
#[cfg(unix)]
fn configure_own_process_group(command: &mut Command) {
    command.process_group(0);
}

#[cfg(not(unix))]
fn configure_own_process_group(_command: &mut Command) {}

#[cfg(target_os = "linux")]
fn configure_linux_subreaper(command: &mut Command) {
    unsafe {
        command.pre_exec(|| {
            if libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[cfg(not(target_os = "linux"))]
fn configure_linux_subreaper(_command: &mut Command) {}

pub(super) fn terminate_completed_process_tree(process_group: Option<u32>) -> Option<String> {
    #[cfg(target_os = "linux")]
    if let Some(pid) = process_group
        && let Some(error) = terminate_linux_descendants(pid)
    {
        return Some(error);
    }
    terminate_completed_process_group(process_group)
}

#[cfg(target_os = "linux")]
fn terminate_linux_descendants(root_pid: u32) -> Option<String> {
    let mut descendants = linux_descendants(root_pid);
    for pid in &descendants {
        if let Some(error) = signal_process(*pid, libc::SIGSTOP) {
            return Some(error);
        }
    }
    descendants.reverse();
    for pid in descendants {
        if let Some(error) = signal_process(pid, libc::SIGKILL) {
            return Some(error);
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn signal_process(pid: u32, signal: libc::c_int) -> Option<String> {
    let result = unsafe { libc::kill(pid as libc::pid_t, signal) };
    if result == 0 {
        return None;
    }
    let error = std::io::Error::last_os_error();
    (error.raw_os_error() != Some(libc::ESRCH)).then(|| error.to_string())
}

#[cfg(target_os = "linux")]
fn linux_descendants(root_pid: u32) -> Vec<u32> {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    let mut parents = HashMap::<u32, Vec<u32>>::new();
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse().ok())
        else {
            continue;
        };
        let Ok(status) = std::fs::read_to_string(entry.path().join("status")) else {
            continue;
        };
        let Some(parent) = process_parent(&status) else {
            continue;
        };
        parents.entry(parent).or_default().push(pid);
    }
    collect_descendants(root_pid, parents)
}

#[cfg(target_os = "linux")]
fn process_parent(status: &str) -> Option<u32> {
    status
        .lines()
        .find_map(|line| line.strip_prefix("PPid:")?.trim().parse().ok())
}

#[cfg(target_os = "linux")]
fn collect_descendants(root_pid: u32, mut parents: HashMap<u32, Vec<u32>>) -> Vec<u32> {
    let mut descendants = Vec::new();
    let mut pending = vec![root_pid];
    while let Some(parent) = pending.pop() {
        if let Some(children) = parents.remove(&parent) {
            pending.extend(children.iter().copied());
            descendants.extend(children);
        }
    }
    descendants
}

fn terminate_completed_process_group(process_group: Option<u32>) -> Option<String> {
    #[cfg(unix)]
    {
        let pid = process_group?;
        let result = unsafe { libc::kill(-(pid as libc::pid_t), libc::SIGKILL) };
        if result == 0 {
            tracing::debug!(
                process_group = pid,
                "bash: terminated completed process group"
            );
            None
        } else {
            let error = std::io::Error::last_os_error();
            (error.raw_os_error() != Some(libc::ESRCH)).then(|| error.to_string())
        }
    }
    #[cfg(not(unix))]
    {
        let _ = process_group;
        None
    }
}
