//! Host-owned Git configuration, captured before agent commands may run.
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

struct Host {
    git: PathBuf,
    path: Option<std::ffi::OsString>,
    contents: String,
    snapshot: PathBuf,
    protected: Vec<PathBuf>,
    pinned: Vec<(String, String)>,
}
static HOST: OnceLock<Result<Host, String>> = OnceLock::new();
static SNAPSHOT_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub(super) fn initialize() -> Result<(), String> {
    let _ = programs();
    let host = host()?;
    write_snapshot(host)
}
fn host() -> Result<&'static Host, String> {
    HOST.get_or_init(capture).as_ref().map_err(Clone::clone)
}
fn capture() -> Result<Host, String> {
    let path = startup_path().clone();
    let cwd = std::env::current_dir()
        .map_err(|e| format!("host tool working directory unavailable: {e}"))?;
    let git = programs()
        .get(OsStr::new("git"))
        .cloned()
        .ok_or("host Git is missing from the absolute startup PATH entries")?;
    let output = super::raw_command(&git)
        .arg("--version")
        .output()
        .map_err(|e| e.to_string())?;
    let version = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() || !supported_version(&version) {
        return Err(format!(
            "Host Git requires version 2.32 or newer (GIT_CONFIG_GLOBAL/SYSTEM support); reported: {}",
            version.trim()
        ));
    }
    let output = super::raw_command(&git)
        .args([
            "config",
            "--includes",
            "--show-origin",
            "--show-scope",
            "--null",
            "--list",
        ])
        .output()
        .map_err(|e| format!("operator configuration discovery failed: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "operator configuration discovery failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let records: Vec<_> = output.stdout.split(|b| *b == 0).collect();
    let mut protected = Vec::new();
    let mut globals = Vec::new();
    for record in records.chunks_exact(3) {
        let scope = record[0];
        if scope != b"global" && scope != b"system" {
            continue;
        }
        let origin = std::str::from_utf8(record[1]).map_err(|e| e.to_string())?;
        if let Some(file) = origin.strip_prefix("file:") {
            let file = cwd.join(file);
            protected.push(file.clone());
            let entry = std::str::from_utf8(record[2]).map_err(|e| e.to_string())?;
            if let Some((key, value)) = entry.split_once('\n') {
                if key == "include.path" || key.starts_with("includeif.") && key.ends_with(".path")
                {
                    let path = value.strip_prefix("~/").map_or_else(
                        || file.parent().unwrap_or(&cwd).join(value),
                        |value| crate::credential_paths::home().join(value),
                    );
                    protected.push(path);
                }
            }
        }
        if scope == b"global" {
            globals.push(record[2]);
        }
    }
    let mut contents = String::new();
    let mut pinned = vec![
        ("core.fsmonitor".into(), "false".into()),
        ("core.hooksPath".into(), neutral().into()),
        ("core.sshCommand".into(), "ssh".into()),
        ("core.editor".into(), "false".into()),
        ("sequence.editor".into(), "false".into()),
        ("core.pager".into(), "cat".into()),
        ("core.askPass".into(), "".into()),
        ("credential.helper".into(), "".into()),
    ];
    for entry in globals {
        let entry = std::str::from_utf8(entry)
            .map_err(|_| "non-UTF8 operator Git configuration cannot be snapshotted")?;
        let (key, value) = entry.split_once('\n').unwrap_or((entry, "true"));
        if key == "include.path" || key.starts_with("includeif.") && key.ends_with(".path") {
            continue;
        }
        let (section, name) = key
            .rsplit_once('.')
            .ok_or("invalid global Git config key")?;
        let (section, subsection) = section
            .split_once('.')
            .map_or((section, None), |(s, v)| (s, Some(v)));
        contents.push_str(&format!(
            "[{}{}]\n{} = \"{}\"\n",
            section,
            subsection
                .map(|s| format!(" \"{}\"", escape(s)))
                .unwrap_or_default(),
            name,
            escape(value)
        ));
        // Keep operator-installed filters above repository configuration.
        if key.starts_with("filter.") {
            pinned.push((key.into(), value.into()));
        }
    }
    // Host-owned run state defaults to the operator configuration directory.
    let directory = std::env::var_os("ARCHON_HOST_RUN_ROOT")
        .map(PathBuf::from)
        .filter(|root| root.is_absolute())
        .unwrap_or_else(|| crate::credential_paths::host_config_dir().join("runs"))
        .join(format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_nanos()
        ));
    let snapshot = directory.join("global.config");
    protected.push(directory);
    let host = Host {
        git,
        path,
        contents,
        snapshot,
        protected,
        pinned,
    };
    #[cfg(any(unix, windows))]
    {
        extern "C" fn cleanup_at_exit() {
            cleanup();
        }
        // SAFETY: the callback owns no borrowed data; static host state outlives it.
        if unsafe { libc::atexit(cleanup_at_exit) } != 0 {
            return Err("host run cleanup callback registration failed".into());
        }
    }
    if let Err(reason) = write_snapshot(&host) {
        if let Some(directory) = host.snapshot.parent() {
            let _ = std::fs::remove_dir_all(directory);
        }
        return Err(reason);
    }
    Ok(host)
}

fn write_snapshot(host: &Host) -> Result<(), String> {
    use std::io::Write;
    let _lock = SNAPSHOT_LOCK.lock().map_err(|e| e.to_string())?;
    let directory = host.snapshot.parent().ok_or("snapshot parent missing")?;
    std::fs::create_dir_all(directory).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    std::fs::set_permissions(
        directory,
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .map_err(|e| e.to_string())?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    match options.open(&host.snapshot) {
        Ok(mut file) => file
            .write_all(host.contents.as_bytes())
            .map_err(|e| e.to_string()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let metadata = std::fs::symlink_metadata(&host.snapshot).map_err(|e| e.to_string())?;
            if !metadata.is_file()
                || std::fs::read(&host.snapshot).map_err(|e| e.to_string())?
                    != host.contents.as_bytes()
            {
                return Err("host configuration snapshot was replaced or changed".into());
            }
            Ok(())
        }
        Err(error) => Err(format!("host configuration snapshot: {error}")),
    }
}

static STARTUP_PATH: OnceLock<Option<std::ffi::OsString>> = OnceLock::new();
fn startup_path() -> &'static Option<std::ffi::OsString> {
    STARTUP_PATH.get_or_init(|| {
        std::env::var_os("PATH").and_then(|path| {
            std::env::join_paths(std::env::split_paths(&path).filter(|p| p.is_absolute())).ok()
        })
    })
}

static PROGRAMS: OnceLock<std::collections::BTreeMap<std::ffi::OsString, PathBuf>> =
    OnceLock::new();
fn programs() -> &'static std::collections::BTreeMap<std::ffi::OsString, PathBuf> {
    PROGRAMS.get_or_init(|| {
        let mut programs = std::collections::BTreeMap::new();
        if let Some(path) = startup_path() {
            for directory in std::env::split_paths(path) {
                let Ok(entries) = std::fs::read_dir(&directory) else {
                    continue;
                };
                for entry in entries.flatten() {
                    let name = program_key(&entry.file_name());
                    if programs.contains_key(&name) {
                        continue;
                    }
                    if let Ok(path) =
                        which::which_in(&name, Some(directory.as_os_str()), &directory)
                    {
                        programs.entry(name).or_insert_with(|| path.clone());
                        #[cfg(windows)]
                        if let Some(stem) = path.file_stem() {
                            let key = program_key(stem);
                            if !programs.contains_key(&key) {
                                if let Ok(resolved) =
                                    which::which_in(&key, startup_path().as_ref(), &directory)
                                {
                                    programs.insert(key, resolved);
                                }
                            }
                        }
                    }
                }
            }
        }
        programs
    })
}

pub(super) fn cleanup() {
    let _lock = SNAPSHOT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(Ok(host)) = HOST.get() {
        if let Some(directory) = host.snapshot.parent() {
            let _ = std::fs::remove_dir_all(directory);
        }
    }
}

fn escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\t', "\\t")
}
fn neutral() -> &'static str {
    if cfg!(windows) { "NUL" } else { "/dev/null" }
}
fn supported_version(version: &str) -> bool {
    let mut parts = version.trim().trim_start_matches("git version ").split('.');
    matches!((parts.next().and_then(|s| s.parse::<u32>().ok()), parts.next().and_then(|s| s.parse::<u32>().ok())), (Some(major), Some(minor)) if major > 2 || major == 2 && minor >= 32)
}
pub(super) fn protected_paths() -> Vec<PathBuf> {
    match host() {
        Ok(host) => host.protected.clone(),
        Err(error) => {
            tracing::error!(%error, "host tool initialization failed");
            Vec::new()
        }
    }
}
fn program_key(name: &OsStr) -> std::ffi::OsString {
    #[cfg(windows)]
    {
        name.to_string_lossy().to_lowercase().into()
    }
    #[cfg(not(windows))]
    {
        name.to_owned()
    }
}
pub(super) fn program(program: &OsStr) -> std::ffi::OsString {
    if Path::new(program).components().count() != 1 || Path::new(program).is_absolute() {
        return program.to_owned();
    }
    programs()
        .get(&program_key(program))
        .map(|path| path.as_os_str().to_owned())
        .unwrap_or_else(|| {
            // A file cannot become a directory in which a later child plants
            // a missing executable. An unavailable executable path fails closed.
            std::env::current_exe()
                .map(|path| path.join(program).into_os_string())
                .unwrap_or_default()
        })
}

pub(super) fn isolate(command: &mut Command) {
    let name = Path::new(command.get_program())
        .file_stem()
        .and_then(|n| n.to_str());
    if !name.is_some_and(|name| name.eq_ignore_ascii_case("git")) {
        return;
    }
    if let Ok(host) = host() {
        if command.get_program() != host.git.as_os_str() {
            return;
        }
        if let Err(reason) = write_snapshot(host) {
            tracing::error!(%reason, "host configuration snapshot unavailable");
        }
        match &host.path {
            Some(path) => {
                command.env("PATH", path);
            }
            None => {
                command.env_remove("PATH");
            }
        }
        command
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_COMMON_DIR")
            .env("GIT_CONFIG_GLOBAL", &host.snapshot)
            .env("GIT_CONFIG_SYSTEM", neutral())
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_COUNT", host.pinned.len().to_string())
            .env("GIT_PAGER", "cat")
            .env_remove("GIT_EXTERNAL_DIFF")
            .env_remove("GIT_SSH_COMMAND")
            .env_remove("GIT_SSH")
            .env("GIT_ASKPASS", "false")
            .env("SSH_ASKPASS", "false")
            .env_remove("GIT_CONFIG_PARAMETERS")
            .env_remove("GIT_CONFIG");
        for (index, (key, value)) in host.pinned.iter().enumerate() {
            command
                .env(format!("GIT_CONFIG_KEY_{index}"), key)
                .env(format!("GIT_CONFIG_VALUE_{index}"), value);
        }
    }
}

#[cfg(test)]
#[path = "spawn_host_config_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "spawn_host_contract_tests.rs"]
mod contract_tests;
