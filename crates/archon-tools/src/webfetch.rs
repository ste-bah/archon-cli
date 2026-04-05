use std::sync::OnceLock;
use std::time::Duration;

use regex::Regex;
use serde_json::json;

use crate::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

static RE_SCRIPT: OnceLock<Regex> = OnceLock::new();
static RE_STYLE: OnceLock<Regex> = OnceLock::new();
static RE_TAGS: OnceLock<Regex> = OnceLock::new();
static RE_WS: OnceLock<Regex> = OnceLock::new();

/// Maximum response body size in bytes (1 MB).
const MAX_BODY_BYTES: usize = 1_024 * 1_024;

/// Request timeout.
const TIMEOUT: Duration = Duration::from_secs(30);

/// Maximum redirects to follow.
const MAX_REDIRECTS: usize = 5;

/// User-Agent header value.
const USER_AGENT: &str = "archon-cli/0.1.0";

pub struct WebFetchTool;

/// Strip HTML to plain text: remove script/style blocks, tags, then collapse whitespace.
fn extract_text(html: &str) -> String {
    // Remove <script>...</script> blocks (case-insensitive, dotall)
    let re_script = RE_SCRIPT
        .get_or_init(|| Regex::new(r"(?is)<script[\s>].*?</script>").expect("valid regex"));
    let text = re_script.replace_all(html, " ");

    // Remove <style>...</style> blocks
    let re_style =
        RE_STYLE.get_or_init(|| Regex::new(r"(?is)<style[\s>].*?</style>").expect("valid regex"));
    let text = re_style.replace_all(&text, " ");

    // Strip remaining HTML tags
    let re_tags = RE_TAGS.get_or_init(|| Regex::new(r"<[^>]*>").expect("valid regex"));
    let text = re_tags.replace_all(&text, " ");

    // Decode common HTML entities
    let text = text
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ");

    // Collapse whitespace
    let re_ws = RE_WS.get_or_init(|| Regex::new(r"\s+").expect("valid regex"));
    re_ws.replace_all(&text, " ").trim().to_string()
}

#[async_trait::async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &str {
        "WebFetch"
    }

    fn description(&self) -> &str {
        "Fetches a URL via HTTP GET and returns the response body. \
         By default, HTML content is stripped to plain text. \
         Set extract_content to false for the raw response."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL to fetch"
                },
                "extract_content": {
                    "type": "boolean",
                    "description": "Strip HTML to plain text (default: true)"
                }
            },
            "required": ["url"]
        })
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let url = match input.get("url").and_then(|v| v.as_str()) {
            Some(u) => u,
            None => return ToolResult::error("url is required and must be a string"),
        };

        let extract = input
            .get("extract_content")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let client = match reqwest::Client::builder()
            .timeout(TIMEOUT)
            .redirect(reqwest::redirect::Policy::limited(MAX_REDIRECTS))
            .user_agent(USER_AGENT)
            .build()
        {
            Ok(c) => c,
            Err(e) => return ToolResult::error(format!("Failed to create HTTP client: {e}")),
        };

        let response = match client.get(url).send().await {
            Ok(r) => r,
            Err(e) => {
                if e.is_timeout() {
                    return ToolResult::error(format!(
                        "Request timed out after {}s",
                        TIMEOUT.as_secs()
                    ));
                }
                if e.is_connect() {
                    return ToolResult::error(format!("Connection failed: {e}"));
                }
                return ToolResult::error(format!("HTTP request failed: {e}"));
            }
        };

        let status = response.status();
        if !status.is_success() {
            return ToolResult::error(format!(
                "HTTP {}: {}",
                status.as_u16(),
                status.canonical_reason().unwrap_or("Unknown")
            ));
        }

        // Read body with size limit
        let bytes = match response.bytes().await {
            Ok(b) => b,
            Err(e) => return ToolResult::error(format!("Failed to read response body: {e}")),
        };

        let truncated = bytes.len() > MAX_BODY_BYTES;
        let body_bytes = if truncated {
            &bytes[..MAX_BODY_BYTES]
        } else {
            &bytes[..]
        };

        let body = String::from_utf8_lossy(body_bytes).into_owned();

        let mut result = if extract { extract_text(&body) } else { body };

        if truncated {
            result.push_str("\n\n[WARNING: Response truncated at 1 MB]");
        }

        ToolResult::success(result)
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Risky
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool::{AgentMode, ToolContext};

    fn test_ctx() -> ToolContext {
        ToolContext {
            working_dir: std::env::temp_dir(),
            session_id: "test".into(),
            mode: AgentMode::Normal,
        }
    }

    #[test]
    fn metadata() {
        let tool = WebFetchTool;
        assert_eq!(tool.name(), "WebFetch");
        assert!(!tool.description().is_empty());

        let schema = tool.input_schema();
        assert_eq!(schema["required"][0], "url");
    }

    #[test]
    fn permission_is_risky() {
        let tool = WebFetchTool;
        assert_eq!(
            tool.permission_level(&json!({"url": "https://example.com"})),
            PermissionLevel::Risky
        );
    }

    #[tokio::test]
    async fn missing_url_is_error() {
        let tool = WebFetchTool;
        let result = tool.execute(json!({}), &test_ctx()).await;
        assert!(result.is_error);
        assert!(result.content.contains("url is required"));
    }

    #[test]
    fn extract_text_strips_scripts() {
        let html = "<html><script>alert('xss')</script><p>Hello</p></html>";
        let text = extract_text(html);
        assert!(!text.contains("alert"));
        assert!(text.contains("Hello"));
    }

    #[test]
    fn extract_text_strips_styles() {
        let html = "<html><style>body{color:red}</style><p>Content</p></html>";
        let text = extract_text(html);
        assert!(!text.contains("color:red"));
        assert!(text.contains("Content"));
    }

    #[test]
    fn extract_text_strips_tags_and_collapses_whitespace() {
        let html = "<div>  <p>Hello</p>   <p>World</p>  </div>";
        let text = extract_text(html);
        assert_eq!(text, "Hello World");
    }

    #[test]
    fn extract_text_decodes_entities() {
        let html = "<p>A &amp; B &lt; C &gt; D</p>";
        let text = extract_text(html);
        assert!(text.contains("A & B < C > D"));
    }

    #[tokio::test]
    async fn invalid_url_is_error() {
        let tool = WebFetchTool;
        let result = tool.execute(json!({"url": "not-a-url"}), &test_ctx()).await;
        assert!(result.is_error);
    }
}
