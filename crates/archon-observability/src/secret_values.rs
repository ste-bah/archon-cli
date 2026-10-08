//! Literal value redaction shared by host-command outputs and tracing.
use once_cell::sync::Lazy;
use parking_lot::RwLock;

/// Marker for configured secret values.
pub const REDACTED_VALUE: &str = "[REDACTED]";
/// Values shorter than eight characters would garble ordinary diagnostics.
const MIN_SECRET_LEN: usize = 8;
const SECRET_NAME_PARTS: &[&str] = &[
    "KEY",
    "TOKEN",
    "SECRET",
    "PASSWORD",
    "PASS",
    "AUTH",
    "CREDENTIAL",
    "PRIVATE",
];

/// Shared credential-name rule for configured environments and headers.
pub fn is_credential_name(name: &str) -> bool {
    let name = name.to_ascii_uppercase();
    SECRET_NAME_PARTS.iter().any(|part| name.contains(part))
}

/// Literal secrets and their escaped representations, independent of log regexes.
#[derive(Clone, Default)]
pub struct SecretValues(Vec<String>);

impl SecretValues {
    /// Select substantial values, including JSON, Debug and URL escaped forms.
    pub fn new<'a>(values: impl IntoIterator<Item = &'a str>) -> Self {
        let mut secrets = Self::default();
        for value in values
            .into_iter()
            .filter(|value| value.chars().count() >= MIN_SECRET_LEN)
        {
            secrets.add(value);
        }
        secrets.normalize();
        secrets
    }

    /// Every nonempty configured value for persisted evidence, including short
    /// credentials. Unlike tracing, this boundary cannot discard known values.
    pub fn for_evidence<'a>(values: impl IntoIterator<Item = &'a str>) -> Self {
        let mut secrets = Self::default();
        for value in values.into_iter().filter(|value| !value.is_empty()) {
            secrets.add(value);
            secrets = secrets.with_url_credentials(value);
        }
        secrets.normalize();
        secrets
    }

    /// Include a known Authorization scheme's bare credential, even if short.
    pub fn with_authorization(mut self, value: &str) -> Self {
        if let Some((scheme, credential)) = value.trim().split_once(char::is_whitespace)
            && (scheme.eq_ignore_ascii_case("Bearer") || scheme.eq_ignore_ascii_case("Basic"))
            && !credential.trim().is_empty()
        {
            self.add(credential.trim());
            self.normalize();
        }
        self
    }

    /// Credential-bearing URLs are secrets regardless of the variable name.
    /// Register the complete address and encoded/decoded credential values.
    pub fn with_url_credentials(mut self, value: &str) -> Self {
        let mut found = false;
        if let Some((_, rest)) = value.split_once("://") {
            let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
            if let Some((userinfo, _)) = authority.rsplit_once('@')
                && let Some((_, password)) = userinfo.split_once(':')
                && !password.is_empty()
            {
                found = true;
                self.add_url_credential(password, false);
            }
            // Query values use form encoding: '+' means space. Decode names
            // as well, so encoded names cannot evade the credential rule.
            if let Some((_, query)) = rest.split('#').next().unwrap_or_default().split_once('?') {
                for parameter in query.split('&') {
                    if let Some((name, credential)) = parameter.split_once('=') {
                        let name = name.replace('+', " ");
                        if let Ok(name) = urlencoding::decode(&name)
                            && is_credential_name(&name)
                            && !credential.is_empty()
                        {
                            found = true;
                            self.add_url_credential(credential, true);
                        }
                    }
                }
            }
        }
        if found {
            self.add(value);
        }
        self.normalize();
        self
    }

    fn add_url_credential(&mut self, credential: &str, form: bool) {
        self.add(credential);
        let value = if form {
            credential.replace('+', " ")
        } else {
            credential.to_string()
        };
        if let Ok(decoded) = urlencoding::decode(&value) {
            self.add(&decoded);
        }
    }

    fn add(&mut self, value: &str) {
        self.0.push(value.to_string());
        // JSON and Debug fields can escape a value before reaching a boundary.
        for encoded in [
            serde_json::to_string(value).ok(),
            Some(format!("{value:?}")),
        ]
        .into_iter()
        .flatten()
        {
            if let Some(inner) = encoded.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
                self.0.push(inner.to_string());
                // Python json.dumps defaults to ensure_ascii, including UTF-16
                // surrogate pairs for supplementary-plane characters.
                let mut ascii = String::new();
                for ch in inner.chars() {
                    if ch.is_ascii() && ch != '\u{007f}' {
                        ascii.push(ch);
                    } else {
                        for unit in ch.encode_utf16(&mut [0; 2]).iter() {
                            ascii.push_str(&format!("\\u{unit:04x}"));
                        }
                    }
                }
                self.0.push(ascii);
            }
        }
        let encoded = urlencoding::encode(value).into_owned();
        // Percent hex digits are case insensitive. Preserve literal letter case.
        let mut lower = encoded.as_bytes().to_vec();
        let mut index = 0;
        while index + 2 < lower.len() {
            if lower[index] == b'%' {
                lower[index + 1].make_ascii_lowercase();
                lower[index + 2].make_ascii_lowercase();
                index += 3;
            } else {
                index += 1;
            }
        }
        let lower = String::from_utf8_lossy(&lower).into_owned();
        for form in [encoded, lower] {
            self.0.push(form.replace("%20", "+"));
            self.0.push(form);
        }
    }

    fn normalize(&mut self) {
        self.0
            .sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
        self.0.dedup();
    }

    /// Replace only configured literal values, preserving diagnostic words.
    pub fn text(&self, text: &str) -> String {
        self.0.iter().fold(text.to_string(), |text, value| {
            text.replace(value.as_str(), REDACTED_VALUE)
        })
    }

    /// Every literal this set replaces, including its escaped forms.
    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(String::as_str)
    }

    /// Whether this set has no values to replace.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Protect tracing sinks for the process lifetime, including cleanup tasks.
    /// Returned errors and content should instead use their own set's `text`.
    pub fn register(&self) {
        #[cfg(any(test, feature = "test-support"))]
        if test_registry::register(self) {
            return;
        }
        self.register_in(&mut REGISTERED.write());
    }

    fn register_in(&self, registry: &mut Self) {
        registry.0.extend(self.0.iter().cloned());
        registry.normalize();
    }
}
static REGISTERED: Lazy<RwLock<SecretValues>> = Lazy::new(|| RwLock::new(SecretValues::default()));

pub(crate) fn redact_registered(text: &str) -> String {
    #[cfg(any(test, feature = "test-support"))]
    if let Some(text) = test_registry::text(text) {
        return text;
    }
    REGISTERED.read().text(text)
}

#[cfg(any(test, feature = "test-support"))]
#[path = "secret_values_test_registry.rs"]
mod test_registry;
#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub use test_registry::scoped_registry_for_tests;

#[cfg(test)]
#[path = "secret_values_tests.rs"]
mod tests;
