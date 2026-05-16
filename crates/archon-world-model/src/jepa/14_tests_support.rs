    use super::*;
    use crate::schema::{WorldActionKind, WorldTraceRow};

    fn jepa_test_guard() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn rows() -> Vec<WorldTraceRow> {
        let mut first = WorldTraceRow::new("s1", WorldActionKind::PlanUpdate).with_row_id("r1");
        first.agent = Some("planner".into());
        first.redacted_excerpt = Some("draft plan".into());
        let mut second = WorldTraceRow::new("s1", WorldActionKind::ToolCall).with_row_id("r2");
        second.provider = Some("local".into());
        second.agent = Some("coder".into());
        second.redacted_excerpt = Some("run cargo test".into());
        let mut third = WorldTraceRow::new("s1", WorldActionKind::Verification).with_row_id("r3");
        third.labels.verification_needed = true;
        third.redacted_excerpt = Some("tests failed".into());
        let mut fourth = WorldTraceRow::new("s1", WorldActionKind::Retry).with_row_id("r4");
        fourth.labels.retry = true;
        fourth.redacted_excerpt = Some("fix tests".into());
        vec![first, second, third, fourth]
    }

    fn long_rows() -> Vec<WorldTraceRow> {
        (0..8)
            .map(|idx| {
                let kind = match idx % 4 {
                    0 => WorldActionKind::PlanUpdate,
                    1 => WorldActionKind::ToolCall,
                    2 => WorldActionKind::Verification,
                    _ => WorldActionKind::Retry,
                };
                let mut row = WorldTraceRow::new("s1", kind).with_row_id(format!("r{idx}"));
                row.provider = Some("local".into());
                row.agent = Some(format!("agent-{}", idx % 2));
                row.redacted_excerpt = Some(format!("trace event {idx}"));
                row.labels.retry = idx % 3 == 0;
                row.labels.verification_needed = idx % 2 == 0;
                row
            })
            .collect()
    }

    #[cfg(feature = "cuda")]
    fn validation_rows(count: usize) -> Vec<WorldTraceRow> {
        (0..count)
            .map(|idx| {
                let kind = match idx % 5 {
                    0 => WorldActionKind::PlanUpdate,
                    1 => WorldActionKind::ToolCall,
                    2 => WorldActionKind::Verification,
                    3 => WorldActionKind::Retry,
                    _ => WorldActionKind::AgentAttempt,
                };
                let mut row = WorldTraceRow::new("validation-session", kind)
                    .with_row_id(format!("validation-row-{idx:04}"));
                row.provider = Some("local".into());
                row.model = Some("validation-model".into());
                row.agent = Some(format!("agent-{}", idx % 4));
                row.redacted_excerpt = Some(format!(
                    "validation trace event {idx} provider={} retry={} verify={}",
                    idx % 3,
                    idx % 7 == 0,
                    idx % 5 == 0
                ));
                row.labels.retry = idx % 7 == 0;
                row.labels.verification_needed = idx % 5 == 0;
                row.labels.plan_drift = idx % 11 == 0;
                row.labels.user_correction = idx % 13 == 0;
                row
            })
            .collect()
    }
