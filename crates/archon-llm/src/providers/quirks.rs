//! TASK-AGS-705: `ProviderQuirks` — per-provider deviations from the
//! baseline OpenAI wire format, kept out of impl code so there are zero
//! `if provider_id == "groq"` branches. This is the REQ-FOR-D6 core
//! contract: adding a new provider is a data-only change, never a code
//! branch.
//!
//! Design notes:
//! - `ProviderQuirks` is `Copy` so every `ProviderDescriptor` can carry
//!   its own value by-value, zero-allocation.
//! - `ignore_response_fields` is `&'static [&'static str]` so the four
//!   preset constants (`DEFAULT`, `GROQ`, `DEEPSEEK`, `MISTRAL`) are
//!   genuine `const` expressions evaluable at compile time.
//! - The struct is NOT serde-derived. Descriptors mark the `quirks`
//!   field with `#[serde(skip)]` / `Default` — quirks are an internal
//!   implementation detail, never configured via TOML/YAML.
//!
//! TECH-AGS-PROVIDERS lines 1155-1157, REQ-FOR-D6 quirks slice.

/// Tool-call wire format variant.
///
/// Groq returns `tool_calls` inside a nested wrapper rather than as a
/// top-level array on the assistant message. Adding a new variant here is
/// a data change, not a new `impl LlmProvider`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolCallFormat {
    /// OpenAI-canonical `tool_calls: [...]` array on the assistant message.
    Standard,
    /// Groq's nested wrapper around the tool-calls list.
    GroqNested,
}

/// Streaming wire delimiter.
///
/// OpenAI-style SSE uses `data: {json}\n\n`; Mistral streams NDJSON —
/// newline-delimited JSON objects, no `data:` prefix. TASK-AGS-707 will
/// branch on this enum to drive its chunk parser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamDelimiter {
    /// `data: {json}\n\n` Server-Sent Events (OpenAI default).
    Sse,
    /// `{json}\n` newline-delimited JSON (Mistral).
    MistralNdjson,
}

/// Per-provider quirks applied by `OpenAiCompatProvider` when building
/// requests and parsing responses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderQuirks {
    /// Wire format for assistant `tool_calls`.
    pub tool_call_format: ToolCallFormat,
    /// SSE vs NDJSON streaming delimiter. Consumed by TASK-AGS-707.
    pub stream_delimiter: StreamDelimiter,
    /// Top-level (and per-choice) response fields to strip before
    /// parsing. Used for DeepSeek's `logprobs` bag that contains
    /// deviant shapes our `LlmResponse` parser doesn't understand.
    pub ignore_response_fields: &'static [&'static str],
}

impl ProviderQuirks {
    /// Baseline quirks — vanilla OpenAI wire format, no field stripping.
    /// Used by 28 of the 31 compat providers and all 9 natives.
    pub const DEFAULT: Self = Self {
        tool_call_format: ToolCallFormat::Standard,
        stream_delimiter: StreamDelimiter::Sse,
        ignore_response_fields: &[],
    };

    /// Groq: nested tool-call format, otherwise vanilla OpenAI.
    pub const GROQ: Self = Self {
        tool_call_format: ToolCallFormat::GroqNested,
        stream_delimiter: StreamDelimiter::Sse,
        ignore_response_fields: &[],
    };

    /// DeepSeek: vanilla wire format, but returns a `logprobs` bag with
    /// a non-OpenAI shape that must be stripped before parsing.
    pub const DEEPSEEK: Self = Self {
        tool_call_format: ToolCallFormat::Standard,
        stream_delimiter: StreamDelimiter::Sse,
        ignore_response_fields: &["logprobs"],
    };

    /// Mistral: NDJSON streaming instead of SSE.
    pub const MISTRAL: Self = Self {
        tool_call_format: ToolCallFormat::Standard,
        stream_delimiter: StreamDelimiter::MistralNdjson,
        ignore_response_fields: &[],
    };

    /// Return the byte sequence that terminates one wire chunk. Exposed
    /// as a `const fn` so TASK-AGS-707's stream parser can pick between
    /// SSE and NDJSON without a string match.
    pub const fn delimiter_bytes(&self) -> &'static [u8] {
        match self.stream_delimiter {
            StreamDelimiter::Sse => b"\n\n",
            StreamDelimiter::MistralNdjson => b"\n",
        }
    }
}

impl Default for ProviderQuirks {
    fn default() -> Self {
        Self::DEFAULT
    }
}
