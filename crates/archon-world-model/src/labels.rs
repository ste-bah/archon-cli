use crate::schema::{WorldActionKind, WorldLabelSet, WorldTraceRow};

#[derive(Debug, Clone, Default)]
pub struct DeterministicLabelBuilder;

impl DeterministicLabelBuilder {
    pub fn label_row(&self, row: &WorldTraceRow) -> WorldLabelSet {
        let mut labels = row.labels.clone();
        let text = row.redacted_excerpt.as_deref().unwrap_or_default();
        let normalized = text.to_ascii_lowercase();

        if normalized.contains("failed") || normalized.contains("error") {
            labels.failure = true;
            labels.success = Some(false);
        }

        if normalized.contains("success") || normalized.contains("completed") {
            labels.success = Some(true);
        }

        if matches!(row.action_kind, WorldActionKind::Retry) || normalized.contains("retry") {
            labels.retry = true;
        }

        if normalized.contains("rate limit")
            || normalized.contains("provider incident")
            || normalized.contains("auth failed")
        {
            labels.provider_incident = true;
        }

        if normalized.contains("verify")
            || normalized.contains("test failed")
            || normalized.contains("needs verification")
        {
            labels.verification_needed = true;
        }

        if normalized.contains("user corrected") || normalized.contains("correction") {
            labels.user_correction = true;
        }

        if normalized.contains("plan drift") || normalized.contains("changed plan") {
            labels.plan_drift = true;
        }

        labels
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{WorldActionKind, WorldTraceRow};

    #[test]
    fn deterministic_labels_detect_failure_retry_and_verification() {
        let mut row = WorldTraceRow::new("session-1", WorldActionKind::Retry);
        row.redacted_excerpt = Some("Tool failed; retry after test failed verification".into());

        let labels = DeterministicLabelBuilder.label_row(&row);

        assert!(labels.failure);
        assert!(labels.retry);
        assert!(labels.verification_needed);
        assert_eq!(labels.success, Some(false));
    }
}
