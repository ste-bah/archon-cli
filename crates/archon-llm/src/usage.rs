use crate::streaming::StreamEvent;
use crate::types::Usage;

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
    saw_start_input: bool,
    saw_start_cache_creation: bool,
    saw_start_cache_read: bool,
    context_input_overflowed: bool,
    input_overflowed: bool,
    output_overflowed: bool,
    cache_creation_overflowed: bool,
    cache_read_overflowed: bool,
}

impl UsageAccumulator {
    pub fn record_event(&mut self, event: &StreamEvent) {
        match event {
            StreamEvent::MessageStart { usage, .. } => self.record_start(usage),
            StreamEvent::MessageDelta {
                usage: Some(usage), ..
            } => self.record_delta(usage),
            _ => {}
        }
    }

    pub fn record_start(&mut self, usage: &Usage) {
        self.saw_start_input = usage.input_tokens_available || usage.input_tokens != 0;
        self.saw_start_cache_creation =
            usage.cache_creation_input_tokens_available || usage.cache_creation_input_tokens != 0;
        self.saw_start_cache_read =
            usage.cache_read_input_tokens_available || usage.cache_read_input_tokens != 0;
        add_usage_field(
            &mut self.billable_input_tokens,
            &mut self.input_tokens_available,
            &mut self.input_overflowed,
            usage.input_tokens,
            usage.input_tokens_available,
        );
        add_usage_field(
            &mut self.cache_creation_input_tokens,
            &mut self.cache_creation_input_tokens_available,
            &mut self.cache_creation_overflowed,
            usage.cache_creation_input_tokens,
            usage.cache_creation_input_tokens_available,
        );
        add_usage_field(
            &mut self.cache_read_input_tokens,
            &mut self.cache_read_input_tokens_available,
            &mut self.cache_read_overflowed,
            usage.cache_read_input_tokens,
            usage.cache_read_input_tokens_available,
        );
        add_context_input(
            &mut self.context_input_tokens,
            &mut self.context_input_overflowed,
            usage,
        );
        add_usage_field(
            &mut self.output_tokens,
            &mut self.output_tokens_available,
            &mut self.output_overflowed,
            usage.output_tokens,
            usage.output_tokens_available,
        );
    }

    pub fn record_delta(&mut self, usage: &Usage) {
        if !self.saw_start_input {
            add_usage_field(
                &mut self.billable_input_tokens,
                &mut self.input_tokens_available,
                &mut self.input_overflowed,
                usage.input_tokens,
                usage.input_tokens_available,
            );
        }
        if !self.saw_start_cache_creation {
            add_usage_field(
                &mut self.cache_creation_input_tokens,
                &mut self.cache_creation_input_tokens_available,
                &mut self.cache_creation_overflowed,
                usage.cache_creation_input_tokens,
                usage.cache_creation_input_tokens_available,
            );
        }
        if !self.saw_start_cache_read {
            add_usage_field(
                &mut self.cache_read_input_tokens,
                &mut self.cache_read_input_tokens_available,
                &mut self.cache_read_overflowed,
                usage.cache_read_input_tokens,
                usage.cache_read_input_tokens_available,
            );
        }
        if !self.saw_start_input || !self.saw_start_cache_creation || !self.saw_start_cache_read {
            add_context_delta(
                &mut self.context_input_tokens,
                &mut self.context_input_overflowed,
                usage,
                self.saw_start_input,
                self.saw_start_cache_creation,
                self.saw_start_cache_read,
            );
        }
        add_usage_field(
            &mut self.output_tokens,
            &mut self.output_tokens_available,
            &mut self.output_overflowed,
            usage.output_tokens,
            usage.output_tokens_available,
        );
    }

    pub fn cache_tokens(&self) -> u64 {
        self.cache_creation_input_tokens
            .checked_add(self.cache_read_input_tokens)
            .unwrap_or(u64::MAX)
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

fn add_context_delta(
    total: &mut u64,
    overflowed: &mut bool,
    usage: &Usage,
    saw_input: bool,
    saw_cache_creation: bool,
    saw_cache_read: bool,
) {
    let input = (!saw_input).then_some(usage.input_tokens).unwrap_or(0);
    let cache_creation = (!saw_cache_creation)
        .then_some(usage.cache_creation_input_tokens)
        .unwrap_or(0);
    let cache_read = (!saw_cache_read)
        .then_some(usage.cache_read_input_tokens)
        .unwrap_or(0);
    add_context_parts(total, overflowed, input, cache_creation, cache_read);
}

fn add_context_input(total: &mut u64, overflowed: &mut bool, usage: &Usage) {
    add_context_parts(
        total,
        overflowed,
        usage.input_tokens,
        usage.cache_creation_input_tokens,
        usage.cache_read_input_tokens,
    );
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
}
