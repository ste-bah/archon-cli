use std::{path::Path, sync::Arc};

use tokio::{process::Command, sync::Mutex};
use uuid::Uuid;

use super::{
    AppState,
    ingest::{WebIngestJob, WebIngestRunRequest},
};

const JOB_LIMIT: usize = 25;
const OUTPUT_LIMIT: usize = 6000;

pub(crate) type WebIngestJobStore = Arc<Mutex<Vec<WebIngestJob>>>;

pub(crate) fn new_job_store() -> WebIngestJobStore {
    Arc::new(Mutex::new(Vec::new()))
}

pub(crate) async fn start_job(
    state: &AppState,
    target: String,
    label: String,
    args: Vec<String>,
) -> WebIngestJob {
    let job_id = Uuid::new_v4().to_string();
    let job = WebIngestJob {
        job_id: job_id.clone(),
        label,
        target,
        command: format!("archon {}", shell_words(&args)),
        status: "running".into(),
        started_at_ms: now_ms(),
        finished_at_ms: None,
        exit_code: None,
        stdout_tail: String::new(),
        stderr_tail: String::new(),
    };
    {
        let mut jobs = state.ingest_jobs.lock().await;
        jobs.insert(0, job.clone());
        jobs.truncate(JOB_LIMIT);
    }
    state.live.record("web.ingest.started", &job.command);
    let jobs = state.ingest_jobs.clone();
    let live = state.live.clone();
    let cwd = state.paths.cwd.clone();
    tokio::spawn(async move {
        let result = run_archon_command(&cwd, &args).await;
        let (status, code, stdout, stderr) = match result {
            Ok(output) => (
                if output.status.success() {
                    "completed"
                } else {
                    "failed"
                }
                .to_string(),
                output.status.code(),
                tail(&String::from_utf8_lossy(&output.stdout)),
                tail(&String::from_utf8_lossy(&output.stderr)),
            ),
            Err(err) => ("failed".into(), None, String::new(), err.to_string()),
        };
        let mut guard = jobs.lock().await;
        if let Some(stored) = guard.iter_mut().find(|item| item.job_id == job_id) {
            stored.status = status.clone();
            stored.exit_code = code;
            stored.stdout_tail = stdout;
            stored.stderr_tail = stderr;
            stored.finished_at_ms = Some(now_ms());
            live.record(
                "web.ingest.finished",
                &format!("{}: {status}", stored.command),
            );
        }
    });
    job
}

pub(crate) fn command_args(request: &WebIngestRunRequest) -> Result<Vec<String>, String> {
    let source = request.source.trim();
    if source.is_empty() && request.target != "kb_process" {
        return Err("source is required".into());
    }
    let mut args = match request.target.as_str() {
        "docs" | "document" => vec!["docs".into(), "ingest".into(), source.into()],
        "kb" => vec!["kb".into(), "ingest".into(), source.into()],
        "kb_process" => vec![
            "kb".into(),
            "process".into(),
            "--claims".into(),
            "--entities".into(),
            "--relations".into(),
            "--contradictions".into(),
        ],
        "video" => vec![
            "video".into(),
            "ingest".into(),
            source.into(),
            "--yes".into(),
        ],
        other => return Err(format!("unsupported ingest target: {other}")),
    };
    if request.target == "video" {
        push_opt(&mut args, "--frames", request.frames.as_deref());
        push_opt(&mut args, "--asr", request.asr.as_deref());
        push_opt(&mut args, "--transcript", request.transcript.as_deref());
        if request.vlm {
            args.push("--vlm".into());
        }
        if request.metadata_only {
            args.push("--metadata-only".into());
        }
    }
    Ok(args)
}

async fn run_archon_command(cwd: &Path, args: &[String]) -> std::io::Result<std::process::Output> {
    Command::new(std::env::current_exe()?)
        .args(args)
        .current_dir(cwd)
        .output()
        .await
}

fn push_opt(args: &mut Vec<String>, flag: &str, value: Option<&str>) {
    if let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) {
        if value != "disabled" && value != "none" {
            args.extend([flag.into(), value.into()]);
        }
    }
}

fn shell_words(args: &[String]) -> String {
    args.iter()
        .map(|arg| {
            if arg.contains(' ') {
                format!("{arg:?}")
            } else {
                arg.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn tail(value: &str) -> String {
    let chars: Vec<_> = value.chars().collect();
    chars[chars.len().saturating_sub(OUTPUT_LIMIT)..]
        .iter()
        .collect()
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}
