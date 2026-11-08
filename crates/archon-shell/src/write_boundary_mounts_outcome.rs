use super::*;

impl MountSeals {
    pub(in crate::write_boundary::landlock) fn outcome_record(
        &self,
        record: StatusRecord,
    ) -> String {
        let StatusRecord(packet, failed) = record;
        if packet[0] < 0
            || packet[0] as usize > STEPS.len()
            || packet[2] < 0
            || packet[2] as usize > self.seals.len()
        {
            return "seal outcome unknown: invalid status record".into();
        }
        let failure = if packet[0] == 0 {
            None
        } else {
            Some(Failure {
                step: STEPS[(packet[0] - 1) as usize],
                errno: packet[1],
                seal: packet[2] as usize,
            })
        };
        if let Some(f) = failure {
            if !diagnostics::namespace_step(f.step)
                && !diagnostics::process_step(f.step)
                && f.seal >= self.seals.len()
            {
                return "seal outcome unknown: invalid entry index".into();
            }
        }
        let mut applied: Vec<_> = self
            .seals
            .iter()
            .enumerate()
            .filter(|(index, _)| failed[index / 64] & (1 << (index % 64)) == 0)
            .map(|(_, seal)| seal)
            .map(|seal| {
                format!(
                    "{} {}",
                    if seal.readonly {
                        "read-only"
                    } else {
                        "rename guard"
                    },
                    std::path::Path::new(std::ffi::OsStr::from_bytes(seal.path.as_bytes()))
                        .display()
                )
            })
            .collect();
        if let Some(f) = failure {
            if matches!(f.step, "remount-readonly" | "recursive-readonly") {
                if let Some(seal) = self.seals.get(f.seal) {
                    applied.push(format!(
                        "{} {}{}",
                        if f.step == "recursive-readonly" {
                            "read-only"
                        } else {
                            "bind"
                        },
                        std::path::Path::new(std::ffi::OsStr::from_bytes(seal.path.as_bytes()))
                            .display(),
                        if f.step == "recursive-readonly" && f.errno == libc::ENOSYS {
                            "; submounts writable"
                        } else {
                            " (incomplete)"
                        }
                    ));
                }
            }
        }
        format!(
            "applied seals: [{}]; {}{}{}",
            applied.join("; "),
            failure.map_or_else(
                || "all seal steps completed".into(),
                |f| {
                    format!(
                        "failed step {} (errno {}); {}",
                        f.step,
                        f.errno,
                        f.message(self)
                    )
                }
            ),
            {
                let unapplied: Vec<_> = self
                    .seals
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| failed[index / 64] & (1 << (index % 64)) != 0)
                    .map(|(_, seal)| {
                        format!(
                            "{} not sealed",
                            std::path::Path::new(std::ffi::OsStr::from_bytes(seal.path.as_bytes()))
                                .display()
                        )
                    })
                    .collect();
                if unapplied.is_empty() {
                    String::new()
                } else {
                    format!("; {}", unapplied.join("; "))
                }
            },
            if packet[3] == 0 && !applied.is_empty() {
                "; applied seals bypassable through the inherited working directory"
            } else {
                ""
            }
        )
    }
}
