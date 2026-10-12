use std::fs;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Output};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::server::FakeProvider;

// Scripted local replies are immediate. A child pointed at any other provider
// must fail within this bound instead of waiting through the production
// provider transport timeout.
pub const NO_PROGRESS_LIMIT: Duration = Duration::from_secs(30);

pub fn progress(test: &str, step: &str) {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    eprintln!("[{stamp}] {test}: {step}");
}

pub fn run(root: &Path, args: &[&str]) -> Output {
    run_within(root, args, NO_PROGRESS_LIMIT)
}

/// `run` with its own no-progress bound, for a cell whose silent host
/// commands (a large freeze) outlast the default under parallel load.
pub fn run_within(root: &Path, args: &[&str], limit: Duration) -> Output {
    let stdout_path = root.join("cell.stdout");
    let stderr_path = root.join("cell.stderr");
    let mut command = Command::new(env!("CARGO_BIN_EXE_archon"));
    isolate_child(&mut command, root);
    let mut child = command
        .current_dir(root)
        .args(args)
        .stdout(fs::File::create(&stdout_path).unwrap())
        .stderr(fs::File::create(&stderr_path).unwrap())
        .spawn()
        .unwrap();
    let status = wait_for_child_within(&mut child, &stdout_path, &stderr_path, "CLI child", limit);
    Output {
        status,
        stdout: fs::read(stdout_path).unwrap_or_default(),
        stderr: fs::read(stderr_path).unwrap_or_default(),
    }
}

pub fn isolate_child(command: &mut Command, root: &Path) {
    let home = root.join(".test-home");
    let config_home = home.join(".config");
    let data_home = home.join(".local/share");
    let cache_home = home.join(".cache");
    for dir in [&home, &config_home, &data_home, &cache_home] {
        fs::create_dir_all(dir).unwrap();
    }
    let config: toml::Value =
        toml::from_str(&fs::read_to_string(root.join("config.toml")).unwrap())
            .expect("temporary project config parses");
    let endpoint = config["api"]["base_url"]
        .as_str()
        .expect("temporary project config has provider endpoint");
    command
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", config_home)
        .env("XDG_DATA_HOME", data_home)
        .env("XDG_CACHE_HOME", cache_home)
        .env("ARCHON_CONFIG_DIR", root)
        .env("ANTHROPIC_API_KEY", "offline-test-key")
        .env("ANTHROPIC_BASE_URL", endpoint)
        .env("OPENAI_BASE_URL", endpoint.trim_end_matches("/v1/messages"));
    // A Windows process without these cannot start Winsock (no socket, so no
    // provider request ever leaves the child) or resolve programs. They name
    // the operating system, not the user, so isolation keeps them.
    #[cfg(windows)]
    for name in [
        "SystemRoot",
        "SystemDrive",
        "WINDIR",
        "COMSPEC",
        "PATHEXT",
        "TEMP",
        "TMP",
    ] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
}

pub fn tail(path: &Path) -> String {
    let bytes = fs::read(path).unwrap_or_default();
    String::from_utf8_lossy(&bytes[bytes.len().saturating_sub(4096)..]).into_owned()
}

pub fn await_provider_boundary(
    provider: &FakeProvider,
    root: &Path,
    child: &mut Child,
    label: &str,
) {
    let stdout = root.join("decompose.stdout");
    let stderr = root.join("decompose.stderr");
    let mut last_progress = Instant::now();
    let mut sizes = (0, 0);
    loop {
        if provider.wait_until_blocked(Duration::from_secs(1)) {
            return;
        }
        let current = (
            fs::metadata(&stdout)
                .map(|metadata| metadata.len())
                .unwrap_or_default(),
            fs::metadata(&stderr)
                .map(|metadata| metadata.len())
                .unwrap_or_default(),
        );
        if current != sizes {
            sizes = current;
            last_progress = Instant::now();
        }
        if last_progress.elapsed() >= NO_PROGRESS_LIMIT {
            let _ = child.kill();
            let _ = wait_for_child(child, &stdout, &stderr, label);
            panic!(
                "{label} made no output progress for {:?} before the provider boundary; child killed; stdout tail: {}; stderr tail: {}",
                NO_PROGRESS_LIMIT,
                tail(&stdout),
                tail(&stderr)
            );
        }
    }
}

pub fn wait_for_child(child: &mut Child, stdout: &Path, stderr: &Path, label: &str) -> ExitStatus {
    wait_for_child_within(child, stdout, stderr, label, NO_PROGRESS_LIMIT)
}

pub fn wait_for_child_within(
    child: &mut Child,
    stdout: &Path,
    stderr: &Path,
    label: &str,
    limit: Duration,
) -> ExitStatus {
    let mut last_progress = Instant::now();
    let mut sizes = (0, 0);
    loop {
        if let Some(status) = child.try_wait().unwrap_or_else(|error| {
            panic!(
                "{label} wait failed: {error}; stdout tail: {}; stderr tail: {}",
                tail(stdout),
                tail(stderr)
            )
        }) {
            return status;
        }
        let current = (
            fs::metadata(stdout)
                .map(|metadata| metadata.len())
                .unwrap_or_default(),
            fs::metadata(stderr)
                .map(|metadata| metadata.len())
                .unwrap_or_default(),
        );
        if current != sizes {
            sizes = current;
            last_progress = Instant::now();
        }
        if last_progress.elapsed() >= limit {
            let _ = child.kill();
            let _ = child.wait();
            panic!(
                "{label} made no output progress for {:?}; child killed; stdout tail: {}; stderr tail: {}",
                limit,
                tail(stdout),
                tail(stderr)
            );
        }
        thread::sleep(Duration::from_millis(100));
    }
}
