use crate::events::TuiEvent;

use super::event_channel::MAX_COALESCED_CONTENT_BYTES;

#[derive(Debug)]
pub(super) enum ContentFrames {
    Single(Option<TuiEvent>),
    Text {
        text: String,
        next_offset: usize,
        emitted_empty: bool,
        thinking: bool,
        transient: bool,
    },
    ToolOutput {
        id: String,
        output: String,
        next_offset: usize,
        emitted_empty: bool,
        completion: Option<(String, bool, Option<String>)>,
    },
}

impl ContentFrames {
    pub(super) fn new(event: TuiEvent) -> Self {
        match event {
            TuiEvent::TextDelta(text) => Self::Text {
                text: compact_string(text),
                next_offset: 0,
                emitted_empty: false,
                thinking: false,
                transient: false,
            },
            TuiEvent::ThinkingDelta(text) => Self::Text {
                text: compact_string(text),
                next_offset: 0,
                emitted_empty: false,
                thinking: true,
                transient: false,
            },
            TuiEvent::TransientThinkingDelta(text) => Self::Text {
                text: compact_string(text),
                next_offset: 0,
                emitted_empty: false,
                thinking: true,
                transient: true,
            },
            TuiEvent::ToolOutputChunk { id, chunk } => {
                Self::tool_output(compact_string(id), compact_string(chunk), None)
            }
            TuiEvent::ToolComplete {
                name,
                id,
                success,
                output,
                transcript_summary,
            } => Self::tool_complete(name, id, success, output, transcript_summary),
            event => Self::Single(Some(event)),
        }
    }

    pub(super) fn frame_count(&self) -> usize {
        match self {
            Self::Single(event) => usize::from(event.is_some()),
            Self::Text { text, .. } => utf8_frame_count(text, MAX_COALESCED_CONTENT_BYTES),
            Self::ToolOutput {
                id,
                output,
                completion,
                ..
            } => utf8_frame_count(output, MAX_COALESCED_CONTENT_BYTES - id.len())
                .saturating_add(usize::from(completion.is_some())),
        }
    }

    fn tool_output(
        id: String,
        output: String,
        completion: Option<(String, bool, Option<String>)>,
    ) -> Self {
        let frame_budget = MAX_COALESCED_CONTENT_BYTES.saturating_sub(id.len());
        if frame_budget == 0
            || output
                .chars()
                .any(|character| character.len_utf8() > frame_budget)
        {
            return match completion {
                Some((name, success, transcript_summary)) => {
                    Self::Single(Some(TuiEvent::ToolComplete {
                        name,
                        id,
                        success,
                        output,
                        transcript_summary,
                    }))
                }
                None => Self::Single(Some(TuiEvent::ToolOutputChunk { id, chunk: output })),
            };
        }
        Self::ToolOutput {
            id,
            output,
            next_offset: 0,
            emitted_empty: false,
            completion,
        }
    }

    fn tool_complete(
        name: String,
        id: String,
        success: bool,
        output: String,
        transcript_summary: Option<String>,
    ) -> Self {
        let name = compact_string(name);
        let id = compact_string(id);
        let output = compact_string(output);
        let metadata_bytes = name.len().saturating_add(id.len());
        if metadata_bytes >= MAX_COALESCED_CONTENT_BYTES {
            return Self::Single(Some(TuiEvent::ToolComplete {
                name,
                id,
                success,
                output,
                transcript_summary,
            }));
        }
        let transcript_summary = transcript_summary.and_then(|summary| {
            let summary = compact_string(summary);
            if summary.len() > MAX_COALESCED_CONTENT_BYTES - metadata_bytes {
                crate::observability::record_tui_event_oversized_metadata_rejected();
                None
            } else {
                Some(summary)
            }
        });
        let completion_bytes = metadata_bytes
            .saturating_add(output.len())
            .saturating_add(transcript_summary.as_ref().map_or(0, String::len));
        if completion_bytes <= MAX_COALESCED_CONTENT_BYTES {
            return Self::Single(Some(TuiEvent::ToolComplete {
                name,
                id,
                success,
                output,
                transcript_summary,
            }));
        }
        Self::tool_output(id, output, Some((name, success, transcript_summary)))
    }
}

impl Iterator for ContentFrames {
    type Item = TuiEvent;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Single(event) => event.take(),
            Self::Text {
                text,
                next_offset,
                emitted_empty,
                thinking,
                transient,
            } => next_utf8_chunk(
                text,
                next_offset,
                emitted_empty,
                MAX_COALESCED_CONTENT_BYTES,
            )
            .map(|chunk| {
                if *transient {
                    TuiEvent::TransientThinkingDelta(chunk)
                } else if *thinking {
                    TuiEvent::ThinkingDelta(chunk)
                } else {
                    TuiEvent::TextDelta(chunk)
                }
            }),
            Self::ToolOutput {
                id,
                output,
                next_offset,
                emitted_empty,
                completion,
            } => {
                let max_bytes = MAX_COALESCED_CONTENT_BYTES - id.len();
                if let Some(chunk) = next_utf8_chunk(output, next_offset, emitted_empty, max_bytes)
                {
                    return Some(TuiEvent::ToolOutputChunk {
                        id: id.clone(),
                        chunk,
                    });
                }
                completion
                    .take()
                    .map(
                        |(name, success, transcript_summary)| TuiEvent::ToolComplete {
                            name,
                            id: id.clone(),
                            success,
                            output: String::new(),
                            transcript_summary,
                        },
                    )
            }
        }
    }
}

fn utf8_frame_count(text: &str, max_bytes: usize) -> usize {
    if text.is_empty() {
        return 1;
    }
    let mut count = 0;
    let mut offset = 0;
    while offset < text.len() {
        let mut end = offset.saturating_add(max_bytes).min(text.len());
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        offset = end;
        count += 1;
    }
    count
}

fn next_utf8_chunk(
    text: &str,
    next_offset: &mut usize,
    emitted_empty: &mut bool,
    max_bytes: usize,
) -> Option<String> {
    if text.is_empty() {
        if *emitted_empty {
            return None;
        }
        *emitted_empty = true;
        return Some(String::new());
    }
    if *next_offset >= text.len() {
        return None;
    }
    let start = *next_offset;
    let mut end = start.saturating_add(max_bytes).min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    *next_offset = end;
    Some(text[start..end].to_owned())
}

fn compact_string(value: String) -> String {
    value.into_boxed_str().into_string()
}
