use super::*;
use std::io;
use std::sync::{Arc, Mutex};
use tracing_subscriber::fmt::MakeWriter;

#[derive(Clone)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl io::Write for Capture {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Capture {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

fn capture_note(output: &[u8]) -> (Option<String>, String) {
    let host = BTreeMap::from([
        ("PATH".into(), "/bin".into()),
        ("FIXTURE_DATA".into(), "secret-value".into()),
    ]);
    let policy = CheckPolicy {
        toolchain_path: Some("/bin".into()),
        ..Default::default()
    };
    let environment = CommandEnvironment::from_host(&host, Some(&policy)).unwrap();
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(tracing::Level::WARN)
        .with_writer(Capture(Arc::clone(&bytes)))
        .finish();
    let note = tracing::subscriber::with_default(subscriber, || environment.note(&[output]));
    let logs = String::from_utf8(bytes.lock().unwrap().clone()).unwrap();
    (note, logs)
}

#[test]
fn matching_names_are_reported_through_tracing() {
    let (note, logs) = capture_note(b"missing FIXTURE_DATA");
    assert!(
        note.as_deref()
            .is_some_and(|value| value.contains("FIXTURE_DATA"))
    );
    assert!(logs.contains("FIXTURE_DATA"), "{logs}");
    assert!(!logs.contains("secret-value"), "{logs}");
}

#[test]
fn boundary_matches_are_reported_through_tracing() {
    let (note, logs) = capture_note(b"(FIXTURE_DATA)");
    assert!(note.is_some());
    assert!(logs.contains("FIXTURE_DATA"), "{logs}");
}

#[test]
fn identifier_substrings_produce_no_note_or_tracing_event() {
    let (note, logs) = capture_note(b"prefix_FIXTURE_DATA_suffix");
    assert_eq!(note, None);
    assert!(logs.is_empty(), "{logs}");
}
