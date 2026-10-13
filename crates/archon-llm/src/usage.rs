use crate::streaming::StreamEvent;
use crate::types::Usage;

/// The usage of one stream, summed over its messages.
///
/// A message's usage arrives in `message_start` and in each `message_delta`.
/// Delta counts are cumulative for their message (the Anthropic streaming
/// spec: "The token counts shown in the `usage` field of the `message_delta`
/// event are *cumulative*"), so a delta replaces the message's count rather
/// than adding to it: `message_start` output 1 and a final `message_delta`
/// output 15 is 15 tokens, not 16 (Issue 424). A count that a delta reports
/// higher than `message_start` did (a proxy's placeholder 0 at start, the
/// real input in the final delta) is taken from the delta (Issue 369). Each
/// counter is the largest value reported for its message, as a cumulative
/// count only grows; a counter an event leaves out is not reset. Messages
/// end at `message_stop` or at the next `message_start`, and are summed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UsageAccumulator {
    pub context_input_tokens: u64,
    pub billable_input_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub cache_read_input_tokens: u64,
    pub output_tokens: u64,
    pub input_tokens_available: bool,
    pub output_tokens_available: bool,
    pub cache_creation_input_tokens_available: bool,
    pub cache_read_input_tokens_available: bool,
    /// The current message's counts so far; only growth over them is added.
    message: MessageCounts,
    context_input_overflowed: bool,
    input_overflowed: bool,
    output_overflowed: bool,
    cache_creation_overflowed: bool,
    cache_read_overflowed: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct MessageCounts {
    input: u64,
    cache_creation: u64,
    cache_read: u64,
    output: u64,
}

/// How much `reported` (a cumulative count) grows `seen`, which it raises.
fn growth(seen: &mut u64, reported: u64) -> u64 {
    let added = reported.saturating_sub(*seen);
    *seen = (*seen).max(reported);
    added
}

impl UsageAccumulator {
    pub fn record_event(&mut self, event: &StreamEvent) {
        match event {
            StreamEvent::MessageStart { usage, .. } => self.record_start(usage),
            StreamEvent::MessageDelta {
                usage: Some(usage), ..
            } => self.record_delta(usage),
            StreamEvent::MessageStop => self.end_message(),
            _ => {}
        }
    }

    /// A new message begins with `usage`.
    pub fn record_start(&mut self, usage: &Usage) {
        self.end_message();
        self.record_delta(usage);
    }

    /// The current message's cumulative `usage`.
    pub fn record_delta(&mut self, usage: &Usage) {
        let input = growth(&mut self.message.input, usage.input_tokens);
        let cache_creation = growth(
            &mut self.message.cache_creation,
            usage.cache_creation_input_tokens,
        );
        let cache_read = growth(&mut self.message.cache_read, usage.cache_read_input_tokens);
        let output = growth(&mut self.message.output, usage.output_tokens);
        add_usage_field(
            &mut self.billable_input_tokens,
            &mut self.input_tokens_available,
            &mut self.input_overflowed,
            input,
            usage.input_tokens_available,
        );
        add_usage_field(
            &mut self.cache_creation_input_tokens,
            &mut self.cache_creation_input_tokens_available,
            &mut self.cache_creation_overflowed,
            cache_creation,
            usage.cache_creation_input_tokens_available,
        );
        add_usage_field(
            &mut self.cache_read_input_tokens,
            &mut self.cache_read_input_tokens_available,
            &mut self.cache_read_overflowed,
            cache_read,
            usage.cache_read_input_tokens_available,
        );
        add_context_parts(
            &mut self.context_input_tokens,
            &mut self.context_input_overflowed,
            input,
            cache_creation,
            cache_read,
        );
        add_usage_field(
            &mut self.output_tokens,
            &mut self.output_tokens_available,
            &mut self.output_overflowed,
            output,
            usage.output_tokens_available,
        );
    }

    fn end_message(&mut self) {
        self.message = MessageCounts::default();
    }

    /// The accumulated totals as one [`Usage`] (billable input as
    /// `input_tokens`).
    pub fn to_usage(&self) -> Usage {
        Usage {
            input_tokens: self.billable_input_tokens,
            output_tokens: self.output_tokens,
            cache_creation_input_tokens: self.cache_creation_input_tokens,
            cache_read_input_tokens: self.cache_read_input_tokens,
            input_tokens_available: self.input_tokens_available,
            output_tokens_available: self.output_tokens_available,
            cache_creation_input_tokens_available: self.cache_creation_input_tokens_available,
            cache_read_input_tokens_available: self.cache_read_input_tokens_available,
        }
    }

    pub fn cache_tokens(&self) -> u64 {
        self.cache_creation_input_tokens
            .saturating_add(self.cache_read_input_tokens)
    }
}

fn add_usage_field(
    total: &mut u64,
    available: &mut bool,
    overflowed: &mut bool,
    value: u64,
    value_available: bool,
) {
    if *overflowed {
        return;
    }
    match total.checked_add(value) {
        Some(sum) => {
            *total = sum;
            *available |= value_available;
        }
        None => {
            *total = u64::MAX;
            *available = false;
            *overflowed = true;
        }
    }
}

fn add_context_parts(
    total: &mut u64,
    overflowed: &mut bool,
    input: u64,
    cache_creation: u64,
    cache_read: u64,
) {
    if *overflowed {
        return;
    }
    let value = input
        .checked_add(cache_creation)
        .and_then(|sum| sum.checked_add(cache_read));
    match value.and_then(|value| total.checked_add(value)) {
        Some(sum) => *total = sum,
        None => {
            *total = u64::MAX;
            *overflowed = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_start_does_not_discard_final_input_and_cache_usage() {
        let mut acc = UsageAccumulator::default();
        acc.record_start(&Usage::default());
        acc.record_delta(&Usage {
            input_tokens: 12,
            output_tokens: 4,
            cache_read_input_tokens: 7,
            input_tokens_available: true,
            output_tokens_available: true,
            cache_read_input_tokens_available: true,
            ..Usage::default()
        });

        assert_eq!(acc.billable_input_tokens, 12);
        assert_eq!(acc.cache_read_input_tokens, 7);
        assert_eq!(acc.context_input_tokens, 19);
        assert!(acc.input_tokens_available);
        assert!(acc.cache_read_input_tokens_available);
    }

    #[test]
    fn accumulator_marks_overflowed_totals_unavailable() {
        let mut acc = UsageAccumulator::default();
        acc.record_start(&Usage {
            input_tokens: u64::MAX,
            input_tokens_available: true,
            ..Default::default()
        });
        acc.record_delta(&Usage {
            output_tokens: u64::MAX,
            output_tokens_available: true,
            ..Default::default()
        });
        // A second message: its count adds to the first's and overflows.
        acc.record_start(&Usage::default());
        acc.record_delta(&Usage {
            output_tokens: 1,
            output_tokens_available: true,
            ..Default::default()
        });

        assert!(!acc.output_tokens_available);
    }

    #[test]
    fn start_usage_counts_cache_once() {
        let mut acc = UsageAccumulator::default();
        acc.record_start(&Usage {
            input_tokens: 10,
            cache_creation_input_tokens: 3,
            cache_read_input_tokens: 7,
            ..Default::default()
        });
        acc.record_delta(&Usage {
            input_tokens: 10,
            output_tokens: 5,
            cache_creation_input_tokens: 3,
            cache_read_input_tokens: 7,
            ..Default::default()
        });

        assert_eq!(acc.context_input_tokens, 20);
        assert_eq!(acc.output_tokens, 5);
    }

    #[test]
    fn delta_input_is_fallback_when_start_missing() {
        let mut acc = UsageAccumulator::default();
        acc.record_delta(&Usage {
            input_tokens: 11,
            output_tokens: 4,
            ..Default::default()
        });
        assert_eq!(acc.context_input_tokens, 11);
        assert_eq!(acc.output_tokens, 4);
    }

    fn reported(input: Option<u64>, output: Option<u64>) -> Usage {
        Usage {
            input_tokens: input.unwrap_or(0),
            output_tokens: output.unwrap_or(0),
            input_tokens_available: input.is_some(),
            output_tokens_available: output.is_some(),
            ..Usage::default()
        }
    }

    fn start_event(usage: Usage) -> StreamEvent {
        StreamEvent::MessageStart {
            id: "msg".into(),
            model: "model".into(),
            usage,
        }
    }

    fn delta_event(usage: Usage) -> StreamEvent {
        StreamEvent::MessageDelta {
            stop_reason: Some("end_turn".into()),
            usage: Some(usage),
        }
    }

    // #424: message_delta usage is cumulative for the message (Anthropic
    // streaming spec), so the delta replaces the start value; it is not added.
    #[test]
    fn delta_output_is_cumulative_with_start_output() {
        let mut acc = UsageAccumulator::default();
        acc.record_event(&start_event(reported(Some(25), Some(1))));
        acc.record_event(&delta_event(reported(None, Some(15))));
        acc.record_event(&StreamEvent::MessageStop);

        assert_eq!(acc.output_tokens, 15);
        assert!(acc.output_tokens_available);
        assert_eq!(acc.billable_input_tokens, 25);
        assert_eq!(acc.context_input_tokens, 25);
    }

    #[test]
    fn delta_input_is_cumulative_with_start_input() {
        // The spec's server-tool example: start input 2679, final delta 10682.
        let mut acc = UsageAccumulator::default();
        acc.record_event(&start_event(reported(Some(2679), Some(3))));
        acc.record_event(&delta_event(reported(Some(10682), Some(510))));

        assert_eq!(acc.billable_input_tokens, 10682);
        assert_eq!(acc.context_input_tokens, 10682);
        assert_eq!(acc.output_tokens, 510);
    }

    #[test]
    fn repeated_cumulative_deltas_count_the_latest_value_once() {
        let mut acc = UsageAccumulator::default();
        acc.record_event(&delta_event(reported(Some(9), Some(3))));
        acc.record_event(&delta_event(reported(Some(9), Some(7))));

        assert_eq!(acc.billable_input_tokens, 9);
        assert_eq!(acc.output_tokens, 7);
    }

    #[test]
    fn output_reported_only_at_start_is_kept() {
        let mut acc = UsageAccumulator::default();
        acc.record_event(&start_event(reported(Some(5), Some(4))));
        acc.record_event(&StreamEvent::MessageDelta {
            stop_reason: Some("end_turn".into()),
            usage: None,
        });
        acc.record_event(&StreamEvent::MessageStop);

        assert_eq!(acc.output_tokens, 4);
        assert!(acc.output_tokens_available);
    }

    #[test]
    fn a_delta_that_omits_a_field_does_not_reset_it() {
        let mut acc = UsageAccumulator::default();
        acc.record_event(&start_event(reported(Some(12), Some(2))));
        acc.record_event(&delta_event(reported(None, Some(6))));

        assert_eq!(acc.billable_input_tokens, 12);
        assert_eq!(acc.output_tokens, 6);
    }

    #[test]
    fn separate_messages_are_summed() {
        let mut acc = UsageAccumulator::default();
        acc.record_event(&start_event(reported(Some(10), Some(1))));
        acc.record_event(&delta_event(reported(None, Some(15))));
        acc.record_event(&StreamEvent::MessageStop);
        acc.record_event(&start_event(reported(Some(20), Some(1))));
        acc.record_event(&delta_event(reported(None, Some(5))));
        acc.record_event(&StreamEvent::MessageStop);
        // A provider that sends no message_start: the stop still ends a message.
        acc.record_event(&delta_event(reported(Some(3), Some(2))));
        acc.record_event(&StreamEvent::MessageStop);

        assert_eq!(acc.output_tokens, 22);
        assert_eq!(acc.billable_input_tokens, 33);
        assert_eq!(acc.context_input_tokens, 33);
    }

    #[test]
    fn to_usage_reports_the_accumulated_totals() {
        let mut acc = UsageAccumulator::default();
        acc.record_event(&start_event(reported(Some(25), Some(1))));
        acc.record_event(&delta_event(reported(None, Some(15))));

        let usage = acc.to_usage();
        assert_eq!(usage.input_tokens, 25);
        assert_eq!(usage.output_tokens, 15);
        assert!(usage.input_tokens_available);
        assert!(usage.output_tokens_available);
    }
}
