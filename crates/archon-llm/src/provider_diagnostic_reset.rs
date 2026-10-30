//! Typed quota reset hints captured before credential redaction.
use chrono::{DateTime, Duration, Utc};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) enum ResetHint {
    At(DateTime<Utc>),
    AfterSeconds(i64),
}
impl ResetHint {
    pub(super) fn at(&self, observed_at: DateTime<Utc>) -> DateTime<Utc> {
        match self {
            Self::At(at) => *at,
            Self::AfterSeconds(seconds) => observed_at + Duration::seconds(*seconds),
        }
    }
}
pub(super) fn parse(message: &str) -> Option<ResetHint> {
    rfc3339_hint(message)
        .map(ResetHint::At)
        .or_else(|| relative_duration_hint(message).map(ResetHint::AfterSeconds))
}

fn rfc3339_hint(message: &str) -> Option<DateTime<Utc>> {
    message
        .split_ascii_whitespace()
        .map(|token| {
            token
                .trim_matches(|c: char| matches!(c, '"' | '\'' | ',' | ';' | ')' | '(' | '[' | ']'))
        })
        .find_map(|token| {
            DateTime::parse_from_rfc3339(token)
                .ok()
                .map(|dt| dt.with_timezone(&Utc))
        })
}

fn relative_duration_hint(message: &str) -> Option<i64> {
    let tokens: Vec<String> = message
        .to_ascii_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(ToOwned::to_owned)
        .collect();

    for pair in tokens.windows(2) {
        let Ok(amount) = pair[0].parse::<i64>() else {
            continue;
        };
        let Some(seconds) = unit_seconds(&pair[1]).map(|unit| amount.saturating_mul(unit)) else {
            continue;
        };
        if seconds > 0 {
            return Some(seconds);
        }
    }
    None
}

fn unit_seconds(unit: &str) -> Option<i64> {
    match unit {
        "second" | "seconds" | "sec" | "secs" => Some(1),
        "minute" | "minutes" | "min" | "mins" => Some(60),
        "hour" | "hours" | "hr" | "hrs" => Some(60 * 60),
        "day" | "days" => Some(24 * 60 * 60),
        _ => None,
    }
}
