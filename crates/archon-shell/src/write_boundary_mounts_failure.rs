use super::MountSeals;

#[derive(Clone, Copy, Debug)]
pub(in crate::write_boundary::landlock) struct Failure {
    pub(super) step: &'static str,
    pub(super) errno: i32,
    pub(super) seal: usize,
}

impl Failure {
    pub(super) fn message(self, mounts: &MountSeals) -> String {
        if self.step == "re-enter working directory" && self.errno == libc::ESTALE {
            return "re-enter working directory: directory changed".into();
        }
        if namespace_step(self.step) {
            return format!(
                "private mount namespace unavailable ({}, errno {})",
                self.step, self.errno
            );
        }
        if process_step(self.step) {
            return format!(
                "private seal operation {} failed (errno {})",
                self.step, self.errno
            );
        }
        let seal = &mounts.seals[self.seal];
        if self.step == "recursive-readonly" && self.errno == libc::ENOSYS && seal.submounts {
            return format!(
                "Linux <5.12 lacks mount_setattr and {} contains mounts that cannot be made read-only",
                seal.display.display()
            );
        }
        format!(
            "private seal operation {} failed (errno {}); {}{}",
            self.step,
            self.errno,
            if seal.readonly {
                "cannot protect "
            } else {
                "rename guard for "
            },
            seal.display.display()
        )
    }
}

pub(super) fn namespace_step(step: &str) -> bool {
    matches!(
        step,
        "unshare" | "setgroups" | "uid_map" | "gid_map" | "mount-private"
    )
}

pub(super) fn process_step(step: &str) -> bool {
    matches!(
        step,
        "inspect working directory"
            | "re-enter working directory"
            | "drop mount capability"
            | "prevent privilege gain"
            | "clear process capabilities"
    )
}
