//! Run-owned transport diagnostics, explicitly inherited across named tasks.
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    fs::OpenOptions,
    future::Future,
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Mutex},
};

tokio::task_local! { static SCOPE: EvidenceScope; }
static APPEND_LOCK: Mutex<()> = Mutex::new(());
static DISK_LOW_CAPS: LazyLock<Mutex<HashMap<PathBuf, bool>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

const SEGMENT_BYTES: u64 = 64 * 1024 * 1024;
const DEFAULT_MIN_FREE_BYTES: u64 = 20 * 1024 * 1024 * 1024;

/// One host call's append-only evidence destination. Never serializes secrets.
#[derive(Clone)]
pub struct EvidenceScope {
    state: Arc<Mutex<State>>,
    call_id: String,
}
struct State {
    path: PathBuf,
    error: Option<String>,
    segment_bytes: u64,
    min_free_bytes: u64,
    free_space: Arc<dyn Fn(&Path) -> io::Result<u64> + Send + Sync>,
}

impl EvidenceScope {
    /// Open evidence before dispatch; an unwritable destination blocks dispatch.
    pub fn new(path: PathBuf, call_id: &str) -> io::Result<Self> {
        Self::with_limits(
            path,
            call_id,
            SEGMENT_BYTES,
            min_free_bytes(),
            Arc::new(|path| fs2::available_space(path)),
        )
    }

    fn with_limits(
        path: PathBuf,
        call_id: &str,
        segment_bytes: u64,
        min_free_bytes: u64,
        free_space: Arc<dyn Fn(&Path) -> io::Result<u64> + Send + Sync>,
    ) -> io::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        open_append(&path)?;
        Ok(Self {
            state: Arc::new(Mutex::new(State {
                path,
                error: None,
                segment_bytes,
                min_free_bytes,
                free_space,
            })),
            call_id: call_id.into(),
        })
    }

    /// Attach this destination for the lifetime of a future.
    pub async fn run<F: Future>(&self, future: F) -> F::Output {
        SCOPE.scope(self.clone(), future).await
    }

    /// Append already-redacted structured evidence; retain write errors for the host.
    pub fn record(&self, mut record: Value) {
        record["call_id"] = json!(self.call_id);
        record["recorded_at"] = json!(chrono::Utc::now().to_rfc3339());
        let line = format!("{record}\n");
        let _append = APPEND_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if disk_low_cap_recorded(&state.path) {
            match resume_if_space(&state, &self.call_id) {
                Ok(true) => {}
                // Still below the floor: drop the record, write nothing more.
                Ok(false) => return,
                Err(error) => {
                    state.error = Some(error.to_string());
                    return;
                }
            }
        }
        let result = append_record(&mut state, &line, &self.call_id);
        if let Err(error) = result {
            state.error = Some(error.to_string());
        }
    }

    /// Fail the call if durable evidence could not be written.
    pub fn check(&self) -> io::Result<()> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        match OpenOptions::new().append(true).open(&state.path) {
            Ok(file) => {
                if let Err(error) = file.sync_data() {
                    state.error.get_or_insert(error.to_string());
                }
            }
            Err(error) => {
                state.error.get_or_insert(error.to_string());
            }
        }
        match &state.error {
            Some(e) => Err(io::Error::other(e.clone())),
            None => Ok(()),
        }
    }
}

fn min_free_bytes() -> u64 {
    std::env::var("ARCHON_TRANSPORT_EVIDENCE_MIN_FREE_BYTES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(DEFAULT_MIN_FREE_BYTES)
}

fn open_append(path: &Path) -> io::Result<std::fs::File> {
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

fn append_record(state: &mut State, line: &str, call_id: &str) -> io::Result<()> {
    let mut file = open_append(&state.path)?;
    let bytes = file.metadata()?.len();
    if bytes.saturating_add(line.len() as u64) > state.segment_bytes && bytes > 0 {
        drop(file);
        let (rotated_from, pruned, free) = rotate(state)?;
        if rotated_from.is_none() {
            file = open_append(&state.path)?;
            if !pruned.is_empty() {
                let event = format!(
                    "{}\n",
                    json!({"kind":"evidence_pruned","segments":pruned,
                        "free_bytes":free})
                );
                file.write_all(event.as_bytes())?;
            }
            let capped = format!(
                "{}\n",
                json!({"kind":"evidence_capped","call_id":call_id,
                    "recorded_at":chrono::Utc::now().to_rfc3339(),
                    "reason":"disk_low","free_bytes":free,
                    "floor_bytes":state.min_free_bytes})
            );
            file.write_all(capped.as_bytes())?;
            DISK_LOW_CAPS
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(state.path.clone(), true);
            return Ok(());
        }
        file = open_append(&state.path)?;
        if !pruned.is_empty() {
            let event = format!(
                "{}\n",
                json!({"kind":"evidence_pruned","segments":pruned,
                    "free_bytes":free})
            );
            file.write_all(event.as_bytes())?;
        }
        let event = format!(
            "{}\n",
            json!({"kind":"evidence_rotated","from":rotated_from})
        );
        file.write_all(event.as_bytes())?;
    }
    file.write_all(line.as_bytes())?;
    Ok(())
}

/// While a path is capped for disk_low, re-check free space. When it is back at
/// or above the floor, clear the cap and write one `evidence_resumed` line.
/// Return true when appends may go on. Caller holds APPEND_LOCK.
fn resume_if_space(state: &State, call_id: &str) -> io::Result<bool> {
    let free = (state.free_space)(&state.path)?;
    if free < state.min_free_bytes {
        return Ok(false);
    }
    let resumed = format!(
        "{}\n",
        json!({"kind":"evidence_resumed","call_id":call_id,
            "recorded_at":chrono::Utc::now().to_rfc3339(),
            "free_bytes":free,"floor_bytes":state.min_free_bytes})
    );
    open_append(&state.path)?.write_all(resumed.as_bytes())?;
    DISK_LOW_CAPS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(state.path.clone(), false);
    Ok(true)
}

/// True when the newest cap/resume marker at the file tail is a disk_low cap.
fn disk_low_cap_recorded(path: &Path) -> bool {
    let mut cache = DISK_LOW_CAPS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(capped) = cache.get(path) {
        return *capped;
    }
    let mut found = false;
    if let Ok(mut file) = OpenOptions::new().read(true).open(path)
        && let Ok(length) = file.metadata().map(|metadata| metadata.len())
    {
        let start = length.saturating_sub(4096);
        if file.seek(SeekFrom::Start(start)).is_ok() {
            let mut tail = String::new();
            if file.read_to_string(&mut tail).is_ok() {
                found = tail
                    .lines()
                    .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                    .filter_map(|record| match record["kind"].as_str() {
                        Some("evidence_capped") if record["reason"] == "disk_low" => Some(true),
                        Some("evidence_resumed") => Some(false),
                        _ => None,
                    })
                    .last()
                    .unwrap_or(false);
            }
        }
    }
    cache.insert(path.to_path_buf(), found);
    found
}

/// Rename the active file only while APPEND_LOCK is held. Return false when disk
/// space stays below the configured floor after pruning every old segment.
fn rotate(state: &State) -> io::Result<(Option<String>, Vec<String>, u64)> {
    let mut free = (state.free_space)(&state.path)?;
    let mut deleted = Vec::new();
    if free < state.min_free_bytes {
        // Prune rotated segments only; the listing ends with the active file.
        let segments = list_transport_segments(&state.path)?;
        for segment in segments.iter().filter(|path| **path != state.path) {
            std::fs::remove_file(segment)?;
            deleted.push(segment.file_name().unwrap().to_string_lossy().into_owned());
            free = (state.free_space)(&state.path)?;
            if free >= state.min_free_bytes {
                break;
            }
        }
    }
    if free < state.min_free_bytes {
        return Ok((None, deleted, free));
    }
    let next = next_segment_path(&state.path)?;
    let from = next.file_name().unwrap().to_string_lossy().into_owned();
    std::fs::rename(&state.path, &next)?;
    open_append(&state.path)?;
    Ok((Some(from), deleted, free))
}

fn next_segment_path(active: &Path) -> io::Result<PathBuf> {
    let parent = active.parent().unwrap_or_else(|| Path::new("."));
    let name = active
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("transport.jsonl");
    let (stem, ext) = name.rsplit_once('.').unwrap_or((name, "jsonl"));
    let mut highest = list_transport_segments(active)?
        .iter()
        .filter_map(|path| segment_number(path, stem, ext))
        .max()
        .unwrap_or(0);
    // Pruning can remove every rotated filename. Keep monotonic numbering by
    // recovering the last number from the rotation event in the active file.
    if let Ok(contents) = std::fs::read_to_string(active) {
        for line in contents.lines() {
            if let Ok(record) = serde_json::from_str::<Value>(line)
                && record["kind"] == "evidence_rotated"
                && let Some(from) = record["from"].as_str()
                && let Some(number) = segment_number(Path::new(from), stem, ext)
            {
                highest = highest.max(number);
            }
        }
    }
    let mut number = highest
        .checked_add(1)
        .ok_or_else(|| io::Error::other("segment number exhausted"))?;
    loop {
        let candidate = parent.join(format!("{stem}.{number:06}.{ext}"));
        if !candidate.exists() {
            return Ok(candidate);
        }
        number = number
            .checked_add(1)
            .ok_or_else(|| io::Error::other("segment number exhausted"))?;
    }
}

fn segment_number(path: &Path, stem: &str, ext: &str) -> Option<u64> {
    let name = path.file_name()?.to_str()?;
    name.strip_prefix(&format!("{stem}."))?
        .strip_suffix(&format!(".{ext}"))?
        .parse()
        .ok()
}

/// List rotated evidence in numeric order, followed by the active path if present.
pub fn list_transport_segments(active: &Path) -> io::Result<Vec<PathBuf>> {
    let parent = active.parent().unwrap_or_else(|| Path::new("."));
    let name = active
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("transport.jsonl");
    let (stem, ext) = name.rsplit_once('.').unwrap_or((name, "jsonl"));
    let prefix = format!("{stem}.");
    let mut numbered = Vec::<(u64, PathBuf)>::new();
    if parent.exists() {
        for entry in std::fs::read_dir(parent)? {
            let path = entry?.path();
            let Some(file) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            let Some(rest) = file.strip_prefix(&prefix) else {
                continue;
            };
            let Some(number) = rest.strip_suffix(&format!(".{ext}")) else {
                continue;
            };
            if let Ok(number) = number.parse::<u64>() {
                numbered.push((number, path));
            }
        }
    }
    numbered.sort_by_key(|(number, _)| *number);
    let mut paths: Vec<_> = numbered.into_iter().map(|(_, path)| path).collect();
    if active.exists() {
        paths.push(active.to_path_buf());
    }
    Ok(paths)
}

/// Read all transport evidence in segment order as a single JSONL string.
pub fn read_transport_evidence(active: &Path) -> io::Result<String> {
    let mut contents = String::new();
    for path in list_transport_segments(active)? {
        use std::io::Read;
        OpenOptions::new()
            .read(true)
            .open(path)?
            .read_to_string(&mut contents)?;
    }
    Ok(contents)
}

/// Capture the current call's context before spawning a child task.
pub fn current() -> Option<EvidenceScope> {
    SCOPE.try_with(Clone::clone).ok()
}
/// Preserve call attribution across the named-task boundary.
pub fn inherit<F: Future>(future: F) -> impl Future<Output = F::Output> {
    let scope = current();
    async move {
        match scope {
            Some(scope) => scope.run(future).await,
            None => future.await,
        }
    }
}

#[cfg(test)]
#[path = "transport_tests.rs"]
mod tests;
