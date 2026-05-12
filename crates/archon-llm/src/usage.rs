use crate::streaming::StreamEvent;
use crate::types::Usage;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UsageAccumulator {
    pub context_input_tokens: u64,
    pub billable_input_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub cache_read_input_tokens: u64,
    pub output_tokens: u64,
    saw_start_usage: bool,
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
        self.saw_start_usage = true;
        self.billable_input_tokens += usage.input_tokens;
        self.cache_creation_input_tokens += usage.cache_creation_input_tokens;
        self.cache_read_input_tokens += usage.cache_read_input_tokens;
        self.context_input_tokens +=
            usage.input_tokens + usage.cache_creation_input_tokens + usage.cache_read_input_tokens;
        self.output_tokens += usage.output_tokens;
    }

    pub fn record_delta(&mut self, usage: &Usage) {
        if !self.saw_start_usage {
            self.billable_input_tokens += usage.input_tokens;
            self.cache_creation_input_tokens += usage.cache_creation_input_tokens;
            self.cache_read_input_tokens += usage.cache_read_input_tokens;
            self.context_input_tokens += usage.input_tokens
                + usage.cache_creation_input_tokens
                + usage.cache_read_input_tokens;
        }
        self.output_tokens += usage.output_tokens;
    }

    pub fn cache_tokens(&self) -> u64 {
        self.cache_creation_input_tokens + self.cache_read_input_tokens
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
