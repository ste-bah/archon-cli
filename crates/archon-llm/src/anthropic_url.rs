const DEFAULT_MESSAGES_URL: &str = "https://api.anthropic.com/v1/messages";

pub(crate) fn messages_url(configured: Option<String>) -> String {
    let Some(raw) = configured else {
        return DEFAULT_MESSAGES_URL.to_string();
    };
    normalize_messages_url(&raw)
}

fn normalize_messages_url(raw: &str) -> String {
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return DEFAULT_MESSAGES_URL.to_string();
    }
    if trimmed.ends_with("/messages") {
        trimmed.to_string()
    } else {
        format!("{trimmed}/v1/messages")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn none_uses_anthropic_messages_endpoint() {
        assert_eq!(messages_url(None), DEFAULT_MESSAGES_URL);
    }

    #[test]
    fn base_url_appends_messages_path() {
        assert_eq!(
            messages_url(Some("https://api.deepseek.com/anthropic".into())),
            "https://api.deepseek.com/anthropic/v1/messages"
        );
    }

    #[test]
    fn full_messages_endpoint_is_preserved() {
        assert_eq!(
            messages_url(Some("http://localhost:4000/v1/messages".into())),
            "http://localhost:4000/v1/messages"
        );
    }
}
