use std::io::{self, Write};
use std::sync::{Arc, Mutex};

const LOG_LIMIT: usize = 64 * 1024;

#[derive(Clone, Default)]
pub(super) struct Logs(Arc<Mutex<(Vec<u8>, bool)>>);

impl Write for Logs {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut state = self.0.lock().unwrap();
        let remaining = LOG_LIMIT - state.0.len();
        state
            .0
            .extend_from_slice(&bytes[..bytes.len().min(remaining)]);
        state.1 |= bytes.len() > remaining;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Logs {
    pub(super) fn install() -> Self {
        let logs = Self::default();
        let writer = logs.clone();
        tracing::subscriber::set_global_default(
            tracing_subscriber::fmt()
                .without_time()
                .with_ansi(false)
                .with_max_level(tracing::Level::INFO)
                .with_writer(move || writer.clone())
                .finish(),
        )
        .unwrap();
        logs
    }

    pub(super) fn read(&self) -> String {
        let state = self.0.lock().unwrap();
        assert!(!state.1, "voice test logs exceeded bound");
        String::from_utf8(state.0.clone()).unwrap()
    }
}

/// Voice bindings are OnceLocks. Each scenario needs a fresh process, with
/// exact test selection rather than resetting or replacing production globals.
pub(super) fn isolated(name: &str) -> bool {
    let full_name = format!("main_voice_tests::{name}");
    if std::env::var("ARCHON_VOICE_WIRING_TEST").as_deref() == Ok(&full_name) {
        return false;
    }
    let status = archon_shell::spawn::command(std::env::current_exe().unwrap())
        .args(["--exact", &full_name, "--test-threads=1"])
        .env("ARCHON_VOICE_WIRING_TEST", &full_name)
        .status()
        .expect("run isolated voice wiring test");
    assert!(
        status.success(),
        "isolated voice wiring test failed: {status}"
    );
    true
}
