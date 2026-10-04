//! Literal value redaction shared by host-command outputs and tracing.
use once_cell::sync::Lazy;
use parking_lot::RwLock;

/// Marker for configured secret values.
pub const REDACTED_VALUE: &str = "[REDACTED]";
/// Values shorter than eight characters would garble ordinary diagnostics.
const MIN_SECRET_LEN: usize = 8;

/// Secret replacement used by the host-command boundary, independent of names.
#[derive(Clone, Default)]
pub struct SecretValues(Vec<String>);

impl SecretValues {
    /// Select substantial values, deduplicate, and replace longer overlaps first.
    pub fn new<'a>(values: impl IntoIterator<Item = &'a str>) -> Self {
        let mut values = values
            .into_iter()
            .filter(|value| value.chars().count() >= MIN_SECRET_LEN)
            .map(str::to_string)
            .collect::<Vec<_>>();
        values.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
        values.dedup();
        Self(values)
    }

    /// Replace configured values in free text using the host-command algorithm.
    pub fn text(&self, text: &str) -> String {
        self.0.iter().fold(text.to_string(), |text, value| {
            text.replace(value.as_str(), REDACTED_VALUE)
        })
    }

    /// Whether this set has no values to replace.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Protect all tracing sinks and background tasks for the process lifetime.
    /// Values remain registered after failed startup or client drop because
    /// transport cleanup and cancellation tasks can still emit diagnostics.
    pub fn register(&self) {
        let mut registered = REGISTERED.write();
        for value in &self.0 {
            registered.0.push(value.clone());
            // Tracing Debug fields and JSON diagnostics escape strings before
            // the visitor sees them. Mask those representations as well.
            for encoded in [
                serde_json::to_string(value).ok(),
                Some(format!("{value:?}")),
            ]
            .into_iter()
            .flatten()
            {
                registered.0.push(encoded[1..encoded.len() - 1].to_string());
            }
        }
        registered
            .0
            .sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
        registered.0.dedup();
    }
}
static REGISTERED: Lazy<RwLock<SecretValues>> = Lazy::new(|| RwLock::new(SecretValues::default()));

pub(crate) fn redact_registered(text: &str) -> String {
    REGISTERED.read().text(text)
}
