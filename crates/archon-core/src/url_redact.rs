//! Redaction for URLs that reach logs, terminals and error strings.
//!
//! Hook and remote URLs are user configuration and routinely carry secrets:
//! `user:password@` userinfo, `?token=` query parameters, and webhook paths
//! whose last segment is itself the credential. Only the origin is safe to
//! print, so that is all [`redact_url`] keeps.

/// Marker appended when anything after the origin was dropped.
const REDACTED_TAIL: &str = "/<redacted>";

/// The origin (`scheme://host[:port]`) of `raw`, followed by a marker when
/// credentials, a path, a query or a fragment were dropped.
///
/// Input that does not parse as an absolute URL is never echoed, because the
/// secret could be anywhere in it.
pub fn redact_url(raw: &str) -> String {
    let Ok(url) = url::Url::parse(raw) else {
        return "<unparseable url>".to_string();
    };
    let Some(host) = url.host_str() else {
        return format!("{}:<redacted>", url.scheme());
    };
    let mut out = format!("{}://{host}", url.scheme());
    if let Some(port) = url.port() {
        out.push_str(&format!(":{port}"));
    }
    let has_tail = !url.username().is_empty()
        || url.password().is_some()
        || !matches!(url.path(), "" | "/")
        || url.query().is_some()
        || url.fragment().is_some();
    if has_tail {
        out.push_str(REDACTED_TAIL);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::redact_url;

    const SECRET: &str = "sk-live-9f8e7d6c5b4a";

    #[test]
    fn secrets_in_every_url_component_are_dropped() {
        for raw in [
            format!("https://user:{SECRET}@hooks.example.com/hook"),
            format!("https://{SECRET}@hooks.example.com/hook"),
            format!("https://hooks.example.com/services/T0/B0/{SECRET}"),
            format!("https://hooks.example.com/hook?token={SECRET}"),
            format!("wss://remote.example.com:8420/ws?token={SECRET}#{SECRET}"),
        ] {
            let shown = redact_url(&raw);
            assert!(!shown.contains(SECRET), "{SECRET} survived in {shown}");
            assert!(shown.ends_with("/<redacted>"), "{shown}");
        }
    }

    #[test]
    fn origin_is_kept_for_diagnosis() {
        assert_eq!(
            redact_url("https://hooks.example.com:8443/a?b=c"),
            "https://hooks.example.com:8443/<redacted>"
        );
        assert_eq!(redact_url("http://[::1]:9000/"), "http://[::1]:9000");
        assert_eq!(redact_url("http://127.0.0.1"), "http://127.0.0.1");
    }

    #[test]
    fn unparseable_input_is_not_echoed() {
        let shown = redact_url(&format!("not a url {SECRET}"));
        assert_eq!(shown, "<unparseable url>");
        let shown = redact_url(&format!("mailto:{SECRET}@example.com"));
        assert!(!shown.contains(SECRET), "{shown}");
    }
}
