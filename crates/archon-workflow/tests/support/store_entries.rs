//! Fixtures for the store-entry tests (Issue-292): planted special
//! entries, the warnings a reader reports, and a bound on a reader that
//! would block on a FIFO.
#![allow(dead_code)]

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Metadata, Subscriber};

/// How long a store read may take before the test calls it blocked.
pub const BLOCKED: Duration = Duration::from_secs(10);

/// Every warning reported while it was installed, one line each.
#[derive(Clone, Default)]
pub struct Warnings(Arc<Mutex<Vec<String>>>);

impl Warnings {
    pub fn lines(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }

    /// Whether a warning names `path`.
    pub fn name(&self, path: &Path) -> bool {
        let shown = path.display().to_string();
        self.lines().iter().any(|line| line.contains(&shown))
    }

    /// How many warnings name `path`.
    pub fn count(&self, path: &Path) -> usize {
        let shown = path.display().to_string();
        self.lines()
            .iter()
            .filter(|line| line.contains(&shown))
            .count()
    }
}

struct Capture(Warnings);
struct Line(String);

impl Visit for Line {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        let _ = write!(self.0, "{}={value:?} ", field.name());
    }
}

impl Subscriber for Capture {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        *metadata.level() <= tracing::Level::WARN
    }
    fn new_span(&self, _: &Attributes<'_>) -> Id {
        Id::from_u64(1)
    }
    fn record(&self, _: &Id, _: &Record<'_>) {}
    fn record_follows_from(&self, _: &Id, _: &Id) {}
    fn event(&self, event: &Event<'_>) {
        let mut line = Line(String::new());
        event.record(&mut line);
        self.0.0.lock().unwrap().push(line.0);
    }
    fn enter(&self, _: &Id) {}
    fn exit(&self, _: &Id) {}
}

/// Run `read` on its own thread with warnings captured. A read still
/// running after [`BLOCKED`] fails the test; each of `fifos` is first
/// opened for writing so the blocked reader can return. The test never
/// waits on the reader past a second [`BLOCKED`]: a reader that is still
/// blocked is left behind, and the suite goes on.
pub fn bounded<T: Send + 'static>(
    fifos: &[PathBuf],
    read: impl FnOnce() -> T + Send + 'static,
) -> (T, Warnings) {
    let warnings = Warnings::default();
    let captured = warnings.clone();
    let (done, wait) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let value = tracing::subscriber::with_default(Capture(captured), read);
        let _ = done.send(value);
    });
    match wait.recv_timeout(BLOCKED) {
        Ok(value) => (value, warnings),
        Err(_) => {
            release(fifos);
            let returned = wait.recv_timeout(BLOCKED).is_ok();
            panic!(
                "a store reader blocked on a planted special entry \
                 (returned once released: {returned}): {fifos:?}"
            );
        }
    }
}

/// Open each FIFO for writing without blocking, then close it: a reader
/// waiting in its open returns, and its read sees end of file.
fn release(fifos: &[PathBuf]) {
    use std::os::unix::fs::OpenOptionsExt;
    for _ in 0..50 {
        for fifo in fifos {
            let _ = std::fs::OpenOptions::new()
                .write(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(fifo);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

pub fn mkfifo(path: &Path) {
    let raw = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(raw.as_ptr(), 0o600) }, 0, "{path:?}");
}

/// `json` padded with whitespace past the store's file bound: still a
/// valid document, too large to be a record.
pub fn write_oversized(path: &Path, json: &[u8]) {
    use std::io::Write;
    let mut file = std::fs::File::create(path).unwrap();
    file.write_all(json).unwrap();
    let pad = vec![b' '; 1 << 20];
    let bound = archon_workflow::v2::store_file::MAX_STORE_FILE_BYTES;
    for _ in 0..=(bound >> 20) {
        file.write_all(&pad).unwrap();
    }
}
