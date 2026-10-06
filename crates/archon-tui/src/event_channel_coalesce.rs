use super::MAX_COALESCED_CONTENT_BYTES;
use crate::events::TuiEvent;
use std::collections::VecDeque;

pub(super) fn coalesce_with_metrics(
    queue: &mut VecDeque<TuiEvent>,
    event: TuiEvent,
) -> Option<TuiEvent> {
    let previous_bytes = queue
        .back()
        .map(crate::event_payload_size::heap_bytes)
        .unwrap_or(0);
    let event = enqueue_or_coalesce_content_delta(queue, event);
    if event.is_none() {
        let current_bytes = queue
            .back()
            .map(crate::event_payload_size::heap_bytes)
            .unwrap_or(0);
        crate::observability::record_tui_event_coalesced_bytes(
            current_bytes.saturating_sub(previous_bytes),
        );
    }
    event
}

fn enqueue_or_coalesce_content_delta(
    queue: &mut VecDeque<TuiEvent>,
    event: TuiEvent,
) -> Option<TuiEvent> {
    match event {
        TuiEvent::TextDelta(text) => {
            if let Some(TuiEvent::TextDelta(previous)) = queue.back_mut()
                && previous.len().saturating_add(text.len()) <= MAX_COALESCED_CONTENT_BYTES
            {
                let mut combined = String::with_capacity(previous.len() + text.len());
                combined.push_str(previous);
                combined.push_str(&text);
                *previous = combined;
                return None;
            }
            Some(TuiEvent::TextDelta(text))
        }
        TuiEvent::ThinkingDelta(text) => {
            if let Some(TuiEvent::ThinkingDelta(previous)) = queue.back_mut()
                && previous.len().saturating_add(text.len()) <= MAX_COALESCED_CONTENT_BYTES
            {
                let mut combined = String::with_capacity(previous.len() + text.len());
                combined.push_str(previous);
                combined.push_str(&text);
                *previous = combined;
                return None;
            }
            Some(TuiEvent::ThinkingDelta(text))
        }
        TuiEvent::TransientThinkingDelta(text) => {
            if let Some(TuiEvent::TransientThinkingDelta(previous)) = queue.back_mut()
                && previous.len().saturating_add(text.len()) <= MAX_COALESCED_CONTENT_BYTES
            {
                let mut combined = String::with_capacity(previous.len() + text.len());
                combined.push_str(previous);
                combined.push_str(&text);
                *previous = combined;
                return None;
            }
            Some(TuiEvent::TransientThinkingDelta(text))
        }
        event => Some(event),
    }
}
