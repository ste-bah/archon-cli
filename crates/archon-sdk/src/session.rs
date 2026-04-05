//! Session management for archon-sdk (TASK-CLI-305).
//!
//! Sessions are persisted as JSON files under `sessions_dir/<id>.json`.
//! Default location: `~/.local/share/archon/sessions/`.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::error::SdkError;
use crate::{SdkAuth, SdkStream};

// ── Public types ──────────────────────────────────────────────────────────────

/// Options for session creation / resumption.
#[derive(Debug, Clone, Default)]
pub struct SessionOptions {
    /// Where to store session JSON files.
    /// Defaults to `~/.local/share/archon/sessions`.
    pub sessions_dir: Option<PathBuf>,
    /// Model override for this session.
    pub model: Option<String>,
    /// Max tokens override for this session.
    pub max_tokens: Option<u32>,
    /// System prompt for this session.
    pub system_prompt: Option<String>,
    /// Auth override for this session.
    pub auth: Option<SdkAuth>,
}

/// Metadata for a stored session (returned by [`list_sessions`] and [`get_session_info`]).
#[derive(Debug, Clone)]
pub struct SdkSessionInfo {
    /// Unique session ID (UUID v4 short form).
    pub id: String,
    /// User-assigned title, if set via [`rename_session`].
    pub title: Option<String>,
    /// User-assigned tags, added via [`tag_session`].
    pub tags: Vec<String>,
    /// Creation time (milliseconds since UNIX epoch).
    pub created_at: u64,
    /// Number of stored conversation turns.
    pub message_count: usize,
}

/// A persistent multi-turn conversation session.
///
/// Create via [`unstable_v2_create_session`] or [`unstable_v2_resume_session`].
#[derive(Debug)]
pub struct ArchonSession {
    id: String,
    sessions_dir: PathBuf,
    model: String,
    max_tokens: u32,
    system_prompt: Option<String>,
    auth: SdkAuth,
}

impl ArchonSession {
    /// Unique session identifier.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Send a message and return a stream of [`SdkMessage`] items.
    ///
    /// The session's conversation history is updated with each exchange.
    pub fn send(&self, prompt: impl Into<String>, _opts: Option<crate::SdkOptions>) -> SdkStream {
        let prompt = prompt.into();
        let sessions_dir = self.sessions_dir.clone();
        let id = self.id.clone();
        let model = self.model.clone();
        let max_tokens = self.max_tokens;
        let system_prompt = self.system_prompt.clone();
        let auth = self.auth.clone();

        let sdk_opts = crate::SdkOptions {
            auth,
            model,
            max_tokens,
            system_prompt,
            ..Default::default()
        };

        (crate::query::query_internal(prompt.clone(), sdk_opts, Some((id, sessions_dir)))) as _
    }
}

// ── On-disk session data ──────────────────────────────────────────────────────

/// Internal session data persisted to disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SessionData {
    id: String,
    title: Option<String>,
    tags: Vec<String>,
    created_at: u64,
    messages: Vec<StoredMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredMessage {
    role: String,
    content: String,
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn default_sessions_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("~/.local/share"))
        .join("archon")
        .join("sessions")
}

fn resolve_sessions_dir(opt: Option<&PathBuf>) -> PathBuf {
    opt.cloned().unwrap_or_else(default_sessions_dir)
}

fn session_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.json"))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn load_session(dir: &Path, id: &str) -> Result<SessionData, SdkError> {
    let path = session_path(dir, id);
    let json = std::fs::read_to_string(&path)
        .map_err(|_| SdkError::Session(format!("session '{id}' not found")))?;
    serde_json::from_str(&json)
        .map_err(|e| SdkError::Session(format!("corrupt session '{id}': {e}")))
}

fn save_session(dir: &Path, data: &SessionData) -> Result<(), SdkError> {
    std::fs::create_dir_all(dir)?;
    let path = session_path(dir, &data.id);
    let json = serde_json::to_string_pretty(data)?;
    std::fs::write(path, json)?;
    Ok(())
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Create a new persistent multi-turn session.
///
/// Returns an [`ArchonSession`] whose conversation history is stored on disk.
/// This function is prefixed `unstable_v2_` to signal that the multi-turn API
/// is alpha-quality and may change.
pub async fn unstable_v2_create_session(
    options: SessionOptions,
) -> Result<ArchonSession, SdkError> {
    let id = uuid::Uuid::new_v4()
        .to_string()
        .replace('-', "")
        .chars()
        .take(16)
        .collect::<String>();

    let dir = resolve_sessions_dir(options.sessions_dir.as_ref());
    let data = SessionData {
        id: id.clone(),
        title: None,
        tags: Vec::new(),
        created_at: now_ms(),
        messages: Vec::new(),
    };
    save_session(&dir, &data)?;

    Ok(ArchonSession {
        id,
        sessions_dir: dir,
        model: options.model.unwrap_or_else(|| "claude-sonnet-4-6".into()),
        max_tokens: options.max_tokens.unwrap_or(8192),
        system_prompt: options.system_prompt,
        auth: options.auth.unwrap_or(SdkAuth::FromEnv),
    })
}

/// Resume an existing session by ID.
///
/// Returns `Err(SdkError::Session)` if the session does not exist.
pub async fn unstable_v2_resume_session(
    id: &str,
    options: SessionOptions,
) -> Result<ArchonSession, SdkError> {
    let dir = resolve_sessions_dir(options.sessions_dir.as_ref());
    let data = load_session(&dir, id)?;

    Ok(ArchonSession {
        id: data.id,
        sessions_dir: dir,
        model: options.model.unwrap_or_else(|| "claude-sonnet-4-6".into()),
        max_tokens: options.max_tokens.unwrap_or(8192),
        system_prompt: options.system_prompt,
        auth: options.auth.unwrap_or(SdkAuth::FromEnv),
    })
}

/// List all sessions in the sessions directory.
pub async fn list_sessions(
    options: Option<SessionOptions>,
) -> Result<Vec<SdkSessionInfo>, SdkError> {
    let dir = resolve_sessions_dir(options.as_ref().and_then(|o| o.sessions_dir.as_ref()));
    if !dir.exists() {
        return Ok(vec![]);
    }

    let mut infos = Vec::new();
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let json = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(_) => continue,
        };
        if let Ok(data) = serde_json::from_str::<SessionData>(&json) {
            infos.push(SdkSessionInfo {
                id: data.id,
                title: data.title,
                tags: data.tags,
                created_at: data.created_at,
                message_count: data.messages.len(),
            });
        }
    }
    infos.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(infos)
}

/// Return metadata for a specific session.
pub async fn get_session_info(
    id: &str,
    options: Option<SessionOptions>,
) -> Result<SdkSessionInfo, SdkError> {
    let dir = resolve_sessions_dir(options.as_ref().and_then(|o| o.sessions_dir.as_ref()));
    let data = load_session(&dir, id)?;
    Ok(SdkSessionInfo {
        message_count: data.messages.len(),
        id: data.id,
        title: data.title,
        tags: data.tags,
        created_at: data.created_at,
    })
}

/// Update the session's display title.
pub async fn rename_session(
    id: &str,
    title: &str,
    options: Option<SessionOptions>,
) -> Result<(), SdkError> {
    let dir = resolve_sessions_dir(options.as_ref().and_then(|o| o.sessions_dir.as_ref()));
    let mut data = load_session(&dir, id)?;
    data.title = Some(title.to_string());
    save_session(&dir, &data)
}

/// Add a tag to the session (idempotent — duplicate tags are ignored).
pub async fn tag_session(
    id: &str,
    tag: &str,
    options: Option<SessionOptions>,
) -> Result<(), SdkError> {
    let dir = resolve_sessions_dir(options.as_ref().and_then(|o| o.sessions_dir.as_ref()));
    let mut data = load_session(&dir, id)?;
    if !data.tags.contains(&tag.to_string()) {
        data.tags.push(tag.to_string());
    }
    save_session(&dir, &data)
}

/// Fork a session: create a copy with a new ID and return it as a new session.
pub async fn fork_session(
    id: &str,
    options: Option<SessionOptions>,
) -> Result<ArchonSession, SdkError> {
    let dir = resolve_sessions_dir(options.as_ref().and_then(|o| o.sessions_dir.as_ref()));
    let source = load_session(&dir, id)?;

    let new_id = uuid::Uuid::new_v4()
        .to_string()
        .replace('-', "")
        .chars()
        .take(16)
        .collect::<String>();

    let forked = SessionData {
        id: new_id.clone(),
        title: source.title.map(|t| format!("{t} (fork)")),
        tags: source.tags,
        created_at: now_ms(),
        messages: source.messages,
    };
    save_session(&dir, &forked)?;

    let auth = options
        .as_ref()
        .and_then(|o| o.auth.clone())
        .unwrap_or(SdkAuth::FromEnv);

    Ok(ArchonSession {
        id: new_id,
        sessions_dir: dir,
        model: options
            .as_ref()
            .and_then(|o| o.model.clone())
            .unwrap_or_else(|| "claude-sonnet-4-6".into()),
        max_tokens: options.as_ref().and_then(|o| o.max_tokens).unwrap_or(8192),
        system_prompt: options.and_then(|o| o.system_prompt),
        auth,
    })
}
