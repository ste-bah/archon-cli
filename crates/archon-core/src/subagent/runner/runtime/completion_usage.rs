use std::sync::{Arc, Mutex};

use super::super::super::ProgressTracker;
use super::super::SubagentCompletion;

pub(super) fn shutdown() -> SubagentCompletion {
    SubagentCompletion {
        content: "[Agent shutdown requested]".to_string(),
        stop_reason: None,
        tokens_in: None,
        tokens_out: None,
    }
}

pub(super) fn from_stream(
    content: String,
    stop_reason: Option<String>,
    progress: Option<&Arc<Mutex<ProgressTracker>>>,
) -> SubagentCompletion {
    let (tokens_in, tokens_out) = progress
        .and_then(|progress| progress.lock().ok())
        .map(|progress| {
            (
                progress
                    .input_tokens_available
                    .then_some(progress.cumulative_input_tokens),
                progress
                    .output_tokens_available
                    .then_some(progress.cumulative_output_tokens),
            )
        })
        .unwrap_or((None, None));
    SubagentCompletion {
        content,
        stop_reason,
        tokens_in,
        tokens_out,
    }
}
