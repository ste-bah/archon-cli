use std::fs;
use std::path::Path;

use super::server::Requests;

/// Retain setup failures too, before a TestProject/provider exists.
pub struct PendingProject(pub Option<tempfile::TempDir>);

impl Drop for PendingProject {
    fn drop(&mut self) {
        if std::thread::panicking()
            && let Some(temp) = self.0.take()
        {
            eprintln!("failed cell project retained at {}", temp.keep().display());
        }
    }
}

pub fn print_failure(root: &Path, requests: &Requests) {
    eprintln!("failed cell project retained at {}", root.display());
    if let Ok(entries) = fs::read_dir(root.join(".archon/workflows")) {
        for entry in entries.flatten() {
            let run = entry.path();
            let events_path = run.join("events.jsonl");
            let Ok(events) = fs::read_to_string(&events_path) else {
                continue;
            };
            let rows = events
                .lines()
                .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
                .collect::<Vec<_>>();
            eprintln!(
                "last events for {}:",
                run.file_name().unwrap_or_default().to_string_lossy()
            );
            for event in rows.iter().rev().take(30).rev() {
                let kind = event.get("kind").unwrap_or(&serde_json::Value::Null);
                let detail = event.get("detail").unwrap_or(&serde_json::Value::Null);
                eprintln!("  kind={} detail={}", kind, detail);
                if event.to_string().contains("progress_history")
                    || event.to_string().contains("last_findings")
                {
                    eprintln!("  pause evidence={event}");
                }
            }
        }
    }
    let snapshot = requests.snapshot();
    let log = snapshot
        .iter()
        .map(|r| {
            serde_json::json!({
                "method": r.method, "path": r.path, "auth_present": r.auth_present,
                "known_path": r.known_path, "role": r.role, "key": r.key,
                "prompt": r.prompt, "answer": r.answer,
            })
            .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n");
    if let Err(error) = fs::write(root.join("provider-requests.jsonl"), log) {
        eprintln!("cannot retain provider requests: {error}");
    }
    for request in snapshot {
        eprintln!(
            "provider method={} path={} auth_present={} known_path={} role={} prompt={} answer={}",
            request.method,
            request.path,
            request.auth_present,
            request.known_path,
            request.role,
            request.prompt.chars().take(200).collect::<String>(),
            request.answer.chars().take(200).collect::<String>()
        );
    }
}
