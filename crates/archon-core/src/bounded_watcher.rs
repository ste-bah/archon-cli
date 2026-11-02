//! Keep native watcher startup and shutdown off the caller's thread.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

use notify::{Config, EventHandler, RecommendedWatcher, RecursiveMode, Watcher};

// FSEvents startup can stall in notify's FsEventWatcher::run (rl_rx.recv)
// while a host daemon is overloaded. Thirty seconds allows substantial host
// scheduling/daemon delay without leaving a session parked indefinitely.
const WATCH_OPERATION_TIMEOUT: Duration = Duration::from_secs(30);

type Reply = Sender<notify::Result<()>>;
enum Command {
    Watch(PathBuf, RecursiveMode, Reply),
    Unwatch(PathBuf, Reply),
}

pub(crate) struct BoundedWatcher {
    commands: Option<Sender<Command>>,
}

impl BoundedWatcher {
    pub(crate) fn new(handler: impl EventHandler, config: Config) -> notify::Result<Self> {
        let (commands, incoming) = mpsc::channel();
        let (ready, started) = mpsc::channel();
        std::thread::Builder::new()
            .name("archon-file-watcher".into())
            .spawn(move || {
                let mut watcher = match RecommendedWatcher::new(handler, config) {
                    Ok(watcher) => watcher,
                    Err(error) => {
                        let _ = ready.send(Err(error));
                        return;
                    }
                };
                if ready.send(Ok(())).is_err() {
                    return;
                }
                while let Ok(command) = incoming.recv() {
                    let (reply, result) = match command {
                        Command::Watch(path, mode, reply) => (reply, watcher.watch(&path, mode)),
                        Command::Unwatch(path, reply) => (reply, watcher.unwatch(&path)),
                    };
                    if reply.send(result).is_err() {
                        break;
                    }
                }
                // Native drop can also wait on FSEvents. It must stay on this
                // worker: the caller never joins a stalled native operation.
            })
            .map_err(notify::Error::io)?;
        receive(&started, "create watcher", WATCH_OPERATION_TIMEOUT)?;
        Ok(Self {
            commands: Some(commands),
        })
    }

    /// False after a timed-out operation: every later request fails at once.
    pub(crate) fn is_available(&self) -> bool {
        self.commands.is_some()
    }

    pub(crate) fn watch(&mut self, path: &Path, mode: RecursiveMode) -> notify::Result<()> {
        self.request(
            |reply| Command::Watch(path.to_path_buf(), mode, reply),
            "watch path",
        )
        .map_err(|error| error.add_path(path.to_path_buf()))
    }

    pub(crate) fn unwatch(&mut self, path: &Path) -> notify::Result<()> {
        self.request(
            |reply| Command::Unwatch(path.to_path_buf(), reply),
            "unwatch path",
        )
        .map_err(|error| error.add_path(path.to_path_buf()))
    }

    fn request(
        &mut self,
        command: impl FnOnce(Reply) -> Command,
        operation: &str,
    ) -> notify::Result<()> {
        self.request_with_timeout(command, operation, WATCH_OPERATION_TIMEOUT)
    }

    fn request_with_timeout(
        &mut self,
        command: impl FnOnce(Reply) -> Command,
        operation: &str,
        timeout: Duration,
    ) -> notify::Result<()> {
        let (reply, response) = mpsc::channel();
        let commands = self.commands.as_ref().ok_or_else(|| {
            notify::Error::generic("native watcher unavailable after an earlier failure")
        })?;
        commands
            .send(command(reply))
            .map_err(|_| notify::Error::generic("native watcher worker disconnected"))?;
        let result = receive(&response, operation, timeout);
        if matches!(&result, Err(error) if matches!(&error.kind,
            notify::ErrorKind::Io(error) if error.kind() == std::io::ErrorKind::TimedOut))
        {
            // Stop accepting work after a timeout. Dropping the sender lets the
            // worker clean up if the daemon recovers; never wait for that here.
            self.commands.take();
        }
        result
    }
}

fn receive(
    response: &Receiver<notify::Result<()>>,
    operation: &str,
    timeout: Duration,
) -> notify::Result<()> {
    match response.recv_timeout(timeout) {
        Ok(result) => result,
        Err(RecvTimeoutError::Timeout) => Err(notify::Error::io(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            format!("{operation}: native watcher did not respond within {timeout:?}"),
        ))),
        Err(RecvTimeoutError::Disconnected) => Err(notify::Error::generic(&format!(
            "{operation}: native watcher worker disconnected"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_startup_acknowledgement_returns_timeout() {
        let (_sender, response) = mpsc::channel();
        let error = receive(&response, "watch path", Duration::ZERO).unwrap_err();
        assert!(matches!(error.kind, notify::ErrorKind::Io(ref e)
            if e.kind() == std::io::ErrorKind::TimedOut));
        assert!(error.to_string().contains("watch path"));
    }

    #[test]
    fn backend_failure_is_not_reported_as_success() {
        let (sender, response) = mpsc::channel();
        sender
            .send(Err(notify::Error::generic("backend failed")))
            .unwrap();
        let error = receive(&response, "watch path", Duration::ZERO).unwrap_err();
        assert!(error.to_string().contains("backend failed"));
    }

    #[test]
    fn timed_out_worker_is_disabled_without_joining() {
        let (commands, incoming) = mpsc::channel();
        let mut watcher = BoundedWatcher {
            commands: Some(commands),
        };
        // An unread command models a native worker stalled before acknowledging.
        // Keep the receiver alive so the failure must be a deadline, not disconnect.
        let error = watcher
            .request_with_timeout(
                |reply| {
                    Command::Watch(PathBuf::from("example"), RecursiveMode::NonRecursive, reply)
                },
                "watch path",
                Duration::ZERO,
            )
            .unwrap_err();
        assert!(matches!(error.kind, notify::ErrorKind::Io(ref e)
            if e.kind() == std::io::ErrorKind::TimedOut));
        drop(incoming);
        assert!(watcher.commands.is_none());
        assert!(
            watcher
                .watch(Path::new("example"), RecursiveMode::NonRecursive)
                .is_err()
        );
    }
}
