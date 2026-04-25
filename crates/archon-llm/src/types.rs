use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Secret wrapper
// ---------------------------------------------------------------------------

/// A wrapper that prevents accidental logging of sensitive values.
/// `Debug` output shows `***` instead of the actual value.
#[derive(Clone)]
pub struct Secret<T> {
    inner: T,
}

impl<T> Secret<T> {
    pub fn new(value: T) -> Self {
        Self { inner: value }
    }

    pub fn expose(&self) -> &T {
        &self.inner
    }
}

impl<T> std::fmt::Debug for Secret<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(***)")
    }
}

impl<T> std::fmt::Display for Secret<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("***")
    }
}

// ---------------------------------------------------------------------------
// API types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Usage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_creation_input_tokens: u64,
    #[serde(default)]
    pub cache_read_input_tokens: u64,
}

impl Usage {
    pub fn merge(&mut self, other: &Usage) {
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
        self.cache_creation_input_tokens += other.cache_creation_input_tokens;
        self.cache_read_input_tokens += other.cache_read_input_tokens;
    }
}

/// Token usage statistics for a session.
/// Alias for Usage - represents input/output token counts.
pub type TokenUsage = Usage;

/// Content block types returned by the API.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentBlockType {
    Text,
    Thinking,
    ToolUse,
    // TASK-P0-B.1a (#178): multi-modal image input.
    Image,
    // TASK-P0-B.1b (#179): multi-modal PDF document input.
    Document,
    // TASK-P0-B.1c (#180): multi-modal audio input (schema-forward).
    Audio,
}

// TASK-P0-B.1a (#178) + TASK-P0-B.1b (#179) + TASK-P0-B.1c (#180):
// ImageSource, DocumentSource, AudioSource live in the `multimodal`
// module (Gate-1 requires their definitions there) and are re-exported
// here so the ContentBlock::{Image,Document,Audio} variants can
// reference them without a cross-module visibility rename.
pub use crate::multimodal::{AudioSource, DocumentSource, ImageSource};

/// A message in the conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: Vec<ContentBlock>,
}

/// A content block within a message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "thinking")]
    Thinking { thinking: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(default)]
        is_error: bool,
    },
    // TASK-P0-B.1a (#178): image content block (Anthropic schema).
    #[serde(rename = "image")]
    Image { source: ImageSource },
    // TASK-P0-B.1b (#179): document content block (Anthropic schema).
    #[serde(rename = "document")]
    Document { source: DocumentSource },
    // TASK-P0-B.1c (#180): audio content block (schema-forward; mirrors
    // image/document shape — Anthropic does not currently accept audio).
    #[serde(rename = "audio")]
    Audio { source: AudioSource },
}
