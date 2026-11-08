use super::mounts::MountSeals;
use crate::read_boundary::status_record::StatusRecord;
use std::io::{self, Read};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::sync::{Arc, Mutex};

/// A prepared channel retained independently by the command and its report.
#[derive(Clone, Debug)]
pub struct SealStatus(Arc<Channel>);

#[derive(Debug)]
struct Channel {
    mounts: Arc<MountSeals>,
    pipe: Result<(Mutex<Reader>, OwnedFd), String>,
}

#[derive(Debug)]
struct Reader {
    file: std::fs::File,
    bytes: [u8; 48],
    filled: usize,
    latest: Option<String>,
}

impl SealStatus {
    pub(super) fn new(mounts: Arc<MountSeals>) -> Self {
        let pipe = (|| {
            let mut fds = [-1; 2];
            // SAFETY: valid output buffer; both fresh descriptors become owned.
            if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK) } != 0 {
                return Err(format!(
                    "status channel unavailable: {}",
                    io::Error::last_os_error()
                ));
            }
            Ok(unsafe {
                (
                    Mutex::new(Reader {
                        file: std::fs::File::from_raw_fd(fds[0]),
                        bytes: [0; 48],
                        filled: 0,
                        latest: None,
                    }),
                    OwnedFd::from_raw_fd(fds[1]),
                )
            })
        })();
        Self(Arc::new(Channel { mounts, pipe }))
    }

    /// Called only in the child; no allocation, formatting, or synchronization.
    pub(super) fn apply(&self) {
        let Ok((_, writer)) = &self.0.pipe else {
            return; // Without a reporting channel, skip optional operations.
        };
        let bytes = self.0.mounts.packet().encode();
        loop {
            // SAFETY: a live descriptor and a fixed stack buffer, smaller than PIPE_BUF.
            let written =
                unsafe { libc::write(writer.as_raw_fd(), bytes.as_ptr().cast(), bytes.len()) };
            if written < 0 && io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            break;
        }
    }

    /// Read after spawn. A pre-spawn query truthfully reports an unknown outcome.
    pub fn report(&self) -> String {
        let (reader, _) = match &self.0.pipe {
            Ok(pipe) => pipe,
            Err(reason) => return format!("not-contained: all optional seals absent ({reason})"),
        };
        let mut reader = reader.lock().unwrap_or_else(|error| error.into_inner());
        loop {
            let Reader {
                file,
                bytes,
                filled,
                ..
            } = &mut *reader;
            match file.read(&mut bytes[*filled..]) {
                Ok(0) => break,
                Ok(n) => *filled += n,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) => return format!("seal outcome unknown: {error}"),
            }
            if *filled == 48 {
                let record = StatusRecord::decode(*bytes);
                reader.latest = Some(self.0.mounts.outcome_record(record));
                reader.filled = 0;
            }
        }
        reader
            .latest
            .clone()
            .unwrap_or_else(|| "seal outcome pending; no applied seals confirmed".into())
    }
}
