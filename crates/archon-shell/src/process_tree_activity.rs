//! CPU activity of pinned members, including descendants outside the original group.
use super::Pinned;
use std::collections::BTreeMap;
use std::io;
use std::time::Instant;

#[derive(Default)]
pub struct Activity {
    previous: Option<BTreeMap<Pinned, u64>>,
}
impl Activity {
    /// Unreadable samples never manufacture activity. Identity is verified around
    /// the query so PID reuse cannot renew a different process's window.
    pub fn observe(&mut self, pins: &[Pinned], deadline: Instant) -> io::Result<bool> {
        let mut current = BTreeMap::new();
        for pin in pins {
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "activity scan stalled",
                ));
            }
            if let Some(cpu) = cpu_time(*pin)? {
                current.insert(*pin, cpu);
            }
        }
        let changed = self
            .previous
            .as_ref()
            .is_some_and(|previous| *previous != current);
        self.previous = Some(current);
        Ok(changed)
    }
}

fn cpu_time(pin: Pinned) -> io::Result<Option<u64>> {
    if super::identity_of(pin.pid)? != Some(pin.start) {
        return Ok(None);
    }
    #[cfg(target_os = "linux")]
    let cpu = {
        let stat = match std::fs::read_to_string(format!("/proc/{}/stat", pin.pid)) {
            Ok(stat) => stat,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let fields: Vec<_> = stat
            .rsplit_once(')')
            .ok_or_else(|| io::Error::other("invalid process stat"))?
            .1
            .split_whitespace()
            .collect();
        // Fields 14/15 (user/system ticks), after pid and comm.
        let parse = |index: usize| -> io::Result<u64> {
            fields
                .get(index)
                .and_then(|v| v.parse().ok())
                .ok_or_else(|| io::Error::other("invalid CPU time"))
        };
        parse(11)?.saturating_add(parse(12)?)
    };
    #[cfg(target_os = "macos")]
    let cpu = {
        // SAFETY: zeroed taskinfo is valid output space, with exactly its size.
        let mut info: libc::proc_taskinfo = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of_val(&info);
        let written = unsafe {
            libc::proc_pidinfo(
                pin.pid as i32,
                libc::PROC_PIDTASKINFO,
                0,
                (&mut info as *mut libc::proc_taskinfo).cast(),
                size as i32,
            )
        };
        if written != size as i32 {
            if super::identity_of(pin.pid)? != Some(pin.start) {
                return Ok(None);
            }
            return Err(io::Error::last_os_error());
        }
        info.pti_total_user.saturating_add(info.pti_total_system)
    };
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    let cpu = {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "CPU activity sampling unsupported",
        ));
    };
    Ok((super::identity_of(pin.pid)? == Some(pin.start)).then_some(cpu))
}
