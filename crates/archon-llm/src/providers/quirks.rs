//! `ProviderQuirks` — per-provider deviations from the baseline OpenAI wire
//! format, kept out of impl code so there are zero
//! `if provider_id == "groq"` branches.
//!
//! TECH-AGS-PROVIDERS implementation_notes / quirks. This is a placeholder
//! struct for TASK-AGS-705, which will populate it with the actual quirk
//! fields (currently just the three fields named in the Phase 7 scope).

use std::collections::HashMap;

/// Per-provider quirks that an `OpenAiCompatProvider` applies when
/// translating requests and parsing responses.
///
/// Every field is optional / empty by default. `None` or an empty map means
/// "use the OpenAI baseline unchanged".
#[derive(Debug, Clone, Default)]
pub struct ProviderQuirks {
    /// Non-standard tool-call format name, e.g. Groq's XML-tagged format.
    /// `None` means standard OpenAI `tool_calls`.
    pub tool_call_format: Option<String>,
    /// Non-standard SSE delimiter, e.g. Mistral's `"\n\n"` vs OpenAI's
    /// `"data: "` prefix. `None` means use the OpenAI default.
    pub stream_delimiter: Option<String>,
    /// Field renames on parse, e.g. DeepSeek's `logprobs` shape. Maps the
    /// wire field name to the canonical archon field name.
    pub response_field_overrides: HashMap<String, String>,
}
