//! Plan-document and Plan Mode audit-file I/O helpers.
//!
//! Editable plans live under `.archon/plans/<plan-id>.md`. Intercepted
//! mutation attempts remain immutable per-session audit entries under
//! `.archon/plan-audit/<session-id>.md`; they never pollute a document the
//! user edits via `/plan open`.
//!
//! Audit append serialization is in-process only. Separate Archon processes
//! can append concurrently and have no stronger ordering guarantee here.

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use archon_session::plan::PlanDocument;
use uuid::Uuid;

static AUDIT_PATH_LOCKS: OnceLock<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>> = OnceLock::new();

fn invalid_id(kind: &str, id: &str) -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        format!("unsafe {kind} ID {id:?}: use ASCII letters, digits, '-' or '_'")
    )
}

fn validate_artifact_id(kind: &str, id: &str) -> std::io::Result<()> {
    if id.is_empty()
        || id == "."
        || id == ".."
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(invalid_id(kind, id));
    }
    Ok(())
}

fn artifact_path(working_dir: &Path, directory: &str, kind: &str, id: &str) -> std::io::Result<PathBuf> {
    validate_artifact_id(kind, id)?;
    Ok(working_dir
        .join(".archon")
        .join(directory)
        .join(format!("{id}.md")))
}

fn audit_path_lock(path: &Path) -> std::io::Result<Arc<Mutex<()>>> {
    let key = path.to_path_buf();
    let locks = AUDIT_PATH_LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut locks = locks.lock().map_err(|_| std::io::Error::other("audit lock registry poisoned"))?;
    Ok(locks
        .entry(key)
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone())
}

/// Path to the editable Markdown document for one plan.
///
/// The ID is an opaque filename component, not a path. Only ASCII letters,
/// digits, `-`, and `_` are accepted, so the returned path remains confined to
/// `.archon/plans` even before that directory exists.
pub fn plan_document_path(working_dir: &Path, plan_id: &str) -> std::io::Result<PathBuf> {
    artifact_path(working_dir, "plans", "plan", plan_id)
}

/// Path to immutable Plan Mode interception audit entries for one session.
///
/// The session ID has the same opaque-component validation as plan IDs, keeping
/// the result confined to `.archon/plan-audit`.
pub fn plan_audit_path(working_dir: &Path, session_id: &str) -> std::io::Result<PathBuf> {
    artifact_path(working_dir, "plan-audit", "session", session_id)
}

fn plan_document_markdown(plan: &PlanDocument) -> String {
    let mut document = format!("# Plan: {}\n\n## Steps\n", plan.title);
    for step in &plan.steps {
        document.push_str(&format!("\n{}. {}\n", step.number, step.description));
    }
    if !plan.risks.is_empty() {
        document.push_str("\n## Risks\n");
        for risk in &plan.risks {
            document.push_str(&format!("\n- {risk}\n"));
        }
    }
    if !plan.questions.is_empty() {
        document.push_str("\n## Questions\n");
        for question in &plan.questions {
            document.push_str(&format!("\n- {question}\n"));
        }
    }
    document
}

/// Render a structured plan as human-editable Markdown through a same-directory
/// temporary file. The replacement rename is atomic on a single filesystem;
/// concurrent writers in different processes remain last-writer-wins.
pub fn write_plan_document(path: &Path, plan: &PlanDocument) -> std::io::Result<()> {
    let parent = path.parent().ok_or_else(|| std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        "plan document path has no parent directory",
    ))?;
    std::fs::create_dir_all(parent)?;

    let filename = path.file_name().and_then(|name| name.to_str()).ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "plan document has no UTF-8 file name")
    })?;
    let temporary = parent.join(format!(".{filename}.{}.tmp", Uuid::new_v4()));
    let result = (|| -> std::io::Result<()> {
        let mut file = OpenOptions::new().write(true).create_new(true).open(&temporary)?;
        file.write_all(plan_document_markdown(plan).as_bytes())?;
        file.flush()?;
        file.sync_all()?;
        std::fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

/// Read editable document text. Returns `None` when no document exists yet.
pub fn read_plan_document(path: &Path) -> std::io::Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(document) => Ok(Some(document)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn format_plan_audit_entry(
    timestamp: chrono::DateTime<chrono::Utc>,
    tool_name: &str,
    input: &serde_json::Value,
) -> String {
    let input_pretty = serde_json::to_string_pretty(input).unwrap_or_else(|_| input.to_string());
    format!(
        "\n## {} — {tool_name} (intercepted in Plan Mode)\n\n```json\n{input_pretty}\n```\n\n",
        timestamp.to_rfc3339()
    )
}

/// Append a structured entry about an intercepted tool call to a session audit
/// log. A complete formatted block is written with one `write_all` while an
/// in-process lock keyed by audit path prevents entries from interleaving.
pub fn append_plan_entry(path: &Path, tool_name: &str, input: &serde_json::Value) -> std::io::Result<()> {
    let lock = audit_path_lock(path)?;
    let _guard = lock.lock().map_err(|_| std::io::Error::other("audit path lock poisoned"))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let block = format_plan_audit_entry(chrono::Utc::now(), tool_name, input);
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    file.write_all(block.as_bytes())?;
    file.flush()
}

/// Open the plan file in `$EDITOR` (or platform default). Returns the
/// resolved plan path on success so the caller can surface it to the user.
/// Blocks until the editor process exits.
pub fn open_plan_in_editor(path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if !path.exists() {
        std::fs::write(path, "# Archon plan\n\n")?;
    }
    let editor = std::env::var("EDITOR")
        .or_else(|_| std::env::var("VISUAL"))
        .unwrap_or_else(|_| if cfg!(windows) { "notepad".to_string() } else { "vi".to_string() });
    let status = std::process::Command::new(&editor).arg(path).status()?;
    if !status.success() {
        return Err(std::io::Error::other(format!("editor '{editor}' exited with status {status}")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_plan() -> PlanDocument {
        let mut plan = PlanDocument::new("plan-42", "Refactor plan files");
        plan.steps.push(archon_session::plan::PlanStep {
            number: 1,
            description: "Separate documents from audit logs".to_string(),
            affected_files: vec!["crates/archon-core/src/plan_file.rs".to_string()],
            status: archon_session::plan::PlanStepStatus::Pending,
            blocked_by: Vec::new(),
            required_evidence: Vec::new(),
            task_id: None,
        });
        plan.risks.push("Do not mix audit history into the document".to_string());
        plan
    }

    #[test]
    fn read_plan_document_returns_none_when_missing() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(read_plan_document(&tmp.path().join("plan.md")).unwrap().is_none());
    }

    #[test]
    fn read_plan_document_returns_content_when_present() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("plan.md");
        std::fs::write(&path, "hello").unwrap();
        assert_eq!(read_plan_document(&path).unwrap(), Some("hello".to_string()));
    }

    #[test]
    fn artifact_paths_reject_unsafe_ids_and_remain_confined() {
        let tmp = tempfile::tempdir().unwrap();
        let unsafe_ids = ["", ".", "..", "/tmp/x", "a/b", "a\\b", "C:\\x", "with space", "plan.md", "å"];
        for id in unsafe_ids {
            assert!(plan_document_path(tmp.path(), id).is_err(), "plan ID {id:?} must fail");
            assert!(plan_audit_path(tmp.path(), id).is_err(), "session ID {id:?} must fail");
        }
        let document = plan_document_path(tmp.path(), "plan_42-a").unwrap();
        let audit = plan_audit_path(tmp.path(), "session_42-a").unwrap();
        assert!(document.strip_prefix(tmp.path().join(".archon").join("plans")).is_ok());
        assert!(audit.strip_prefix(tmp.path().join(".archon").join("plan-audit")).is_ok());
    }

    #[test]
    fn editable_document_and_session_audit_paths_are_distinct() {
        let tmp = tempfile::tempdir().unwrap();
        let document = plan_document_path(tmp.path(), "plan-42").unwrap();
        let audit = plan_audit_path(tmp.path(), "session-a").unwrap();
        assert_eq!(document, tmp.path().join(".archon/plans/plan-42.md"));
        assert_eq!(audit, tmp.path().join(".archon/plan-audit/session-a.md"));
        assert_ne!(document, audit);
    }

    #[test]
    fn plan_document_round_trips_as_editable_markdown() {
        let tmp = tempfile::tempdir().unwrap();
        let path = plan_document_path(tmp.path(), "plan-42").unwrap();
        write_plan_document(&path, &sample_plan()).unwrap();
        let document = read_plan_document(&path).unwrap().expect("document exists");
        assert!(document.contains("# Plan: Refactor plan files"));
        assert!(document.contains("## Risks"));
    }

    #[test]
    fn audit_entry_matches_the_existing_byte_format() {
        let timestamp = chrono::DateTime::parse_from_rfc3339("2026-08-15T12:34:56Z").unwrap().with_timezone(&chrono::Utc);
        assert_eq!(
            format_plan_audit_entry(timestamp, "Write", &serde_json::json!({"path":"/tmp/x"})),
            "\n## 2026-08-15T12:34:56+00:00 — Write (intercepted in Plan Mode)\n\n```json\n{\n  \"path\": \"/tmp/x\"\n}\n```\n\n"
        );
    }

    #[test]
    fn append_plan_entry_creates_and_appends_complete_blocks() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("sub/.archon/plan.md");
        append_plan_entry(&path, "Write", &serde_json::json!({"a":1})).unwrap();
        append_plan_entry(&path, "Bash", &serde_json::json!({"cmd":"ls"})).unwrap();
        let content = std::fs::read_to_string(path).unwrap();
        assert_eq!(content.matches("(intercepted in Plan Mode)").count(), 2);
        assert!(content.contains("Write"));
        assert!(content.contains("Bash"));
    }

    #[test]
    fn concurrent_audit_writers_do_not_interleave_blocks() {
        let tmp = tempfile::tempdir().unwrap();
        let path = Arc::new(tmp.path().join("audit.md"));
        let mut writers = Vec::new();
        for number in 0..16 {
            let path = Arc::clone(&path);
            writers.push(std::thread::spawn(move || {
                append_plan_entry(&path, &format!("Tool{number}"), &serde_json::json!({"writer":number})).unwrap();
            }));
        }
        for writer in writers {
            writer.join().unwrap();
        }
        let content = std::fs::read_to_string(&*path).unwrap();
        assert_eq!(content.matches("(intercepted in Plan Mode)").count(), 16);
        assert_eq!(content.matches("```json").count(), 16);
        assert_eq!(content.matches("\n```\n").count(), 16);
        for number in 0..16 {
            assert!(content.contains(&format!("Tool{number}")));
            assert!(content.contains(&format!("\"writer\": {number}")));
        }
    }
}
