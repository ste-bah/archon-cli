//! Operator labeling helper for reasoning-quality shadow mode.

use std::io::{IsTerminal, Write};
use std::path::Path;

use anyhow::Result;

pub(crate) fn render_sample_label(
    root: &Path,
    session_id: &str,
    turn: Option<u64>,
) -> Result<String> {
    let store = archon_reasoning_quality::store::ReasoningQualityStore::open(root)?;
    let mut events = store.events_for_session(session_id)?;
    if let Some(turn) = turn {
        events.retain(|event| event.turn_number == turn);
    }
    events.retain(|event| event.severity_effective >= 0.4);
    events.truncate(10);
    if events.is_empty() {
        return Ok(format!(
            "Reasoning Sample Label\n======================\nSession: {session_id}\nNo labelable events found."
        ));
    }

    let interactive = std::io::stdin().is_terminal();
    let path = root
        .join("shadow")
        .join("operator-labels")
        .join(format!("{session_id}.jsonl"));
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    let mut labeled = 0usize;
    for event in &events {
        let label = if interactive {
            print!(
                "Label {} {:?} {:?}: correct? [y/n/s] ",
                event.claim_id, event.event_kind, event.subject
            );
            let _ = std::io::stdout().flush();
            let mut answer = String::new();
            std::io::stdin().read_line(&mut answer)?;
            match answer.trim().to_ascii_lowercase().as_str() {
                "y" | "yes" => Some(true),
                "n" | "no" => Some(false),
                _ => None,
            }
        } else {
            None
        };
        serde_json::to_writer(
            &mut file,
            &serde_json::json!({
                "session_id": session_id,
                "turn_number": event.turn_number,
                "event_id": event.event_id,
                "claim_id": event.claim_id,
                "event_kind": event.event_kind,
                "subject": event.subject,
                "label_correct": label,
                "created_at": chrono::Utc::now().to_rfc3339(),
            }),
        )?;
        file.write_all(b"\n")?;
        labeled += 1;
    }
    Ok(format!(
        "Reasoning Sample Label\n======================\nSession: {session_id}\nMode: {}\nRows written: {labeled}\nLabel file: {}",
        if interactive {
            "interactive"
        } else {
            "worksheet"
        },
        path.display()
    ))
}
