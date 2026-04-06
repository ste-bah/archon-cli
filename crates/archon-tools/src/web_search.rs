use std::sync::OnceLock;
use std::time::Duration;

use regex::Regex;
use serde_json::json;

use crate::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

static RE_RESULT_LINK: OnceLock<Regex> = OnceLock::new();
static RE_RESULT_SNIPPET: OnceLock<Regex> = OnceLock::new();
static RE_STRIP_TAGS: OnceLock<Regex> = OnceLock::new();

/// Request timeout for DuckDuckGo searches.
const TIMEOUT: Duration = Duration::from_secs(15);

/// User-Agent header value.
const USER_AGENT: &str = "archon-cli/0.1.0";

pub struct WebSearchTool;

/// Decode common HTML entities.
pub(crate) fn decode_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
}

/// Strip HTML tags from a string.
pub(crate) fn strip_tags(s: &str) -> String {
    let re = RE_STRIP_TAGS.get_or_init(|| Regex::new(r"<[^>]*>").expect("valid regex"));
    re.replace_all(s, "").to_string()
}

/// Simple percent-decoding for URL-encoded strings.
pub(crate) fn urlencoded_decode(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.bytes();
    while let Some(b) = chars.next() {
        if b == b'%' {
            let hi = chars.next();
            let lo = chars.next();
            if let (Some(h), Some(l)) = (hi, lo) {
                let hex = [h, l];
                if let Ok(decoded) = u8::from_str_radix(
                    &String::from_utf8_lossy(&hex),
                    16,
                ) {
                    result.push(decoded as char);
                } else {
                    result.push('%');
                    result.push(h as char);
                    result.push(l as char);
                }
            } else {
                result.push('%');
                if let Some(h) = hi {
                    result.push(h as char);
                }
            }
        } else if b == b'+' {
            result.push(' ');
        } else {
            result.push(b as char);
        }
    }
    result
}

/// Extract the actual URL from a DuckDuckGo redirect href.
///
/// DDG wraps URLs like `//duckduckgo.com/l/?uddg=ACTUAL_URL&...`.
/// This extracts and decodes the actual URL from the `uddg` parameter.
pub(crate) fn extract_ddg_url(href: &str) -> String {
    if let Some(pos) = href.find("uddg=") {
        let start = pos + 5;
        let end = href[start..]
            .find('&')
            .map(|i| start + i)
            .unwrap_or(href.len());
        let encoded = &href[start..end];
        urlencoded_decode(encoded)
    } else {
        href.to_string()
    }
}

/// A single search result.
pub(crate) struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// Parse DuckDuckGo HTML response into search results.
pub(crate) fn parse_results(html: &str, max_results: usize) -> Vec<SearchResult> {
    let re_link = RE_RESULT_LINK.get_or_init(|| {
        Regex::new(r#"<a[^>]*class="result__a"[^>]*href="([^"]*)"[^>]*>([^<]*)</a>"#)
            .expect("valid regex")
    });
    let re_snippet = RE_RESULT_SNIPPET.get_or_init(|| {
        Regex::new(r#"<a[^>]*class="result__snippet"[^>]*>(.*?)</a>"#).expect("valid regex")
    });

    let links: Vec<(String, String)> = re_link
        .captures_iter(html)
        .map(|cap| {
            let raw_url = cap.get(1).map_or("", |m| m.as_str());
            let title = cap.get(2).map_or("", |m| m.as_str());
            (
                extract_ddg_url(&decode_entities(raw_url)),
                decode_entities(title),
            )
        })
        .collect();

    let snippets: Vec<String> = re_snippet
        .captures_iter(html)
        .map(|cap| {
            let raw = cap.get(1).map_or("", |m| m.as_str());
            decode_entities(&strip_tags(raw)).trim().to_string()
        })
        .collect();

    let count = links.len().min(max_results);
    let mut results = Vec::with_capacity(count);
    for i in 0..count {
        let (url, title) = &links[i];
        let snippet = snippets.get(i).cloned().unwrap_or_default();
        results.push(SearchResult {
            title: title.clone(),
            url: url.clone(),
            snippet,
        });
    }
    results
}

/// Format search results as a numbered list.
pub(crate) fn format_results(results: &[SearchResult]) -> String {
    let mut out = String::new();
    for (i, r) in results.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&format!("{}. {}\n", i + 1, r.title));
        out.push_str(&format!("   URL: {}\n", r.url));
        if !r.snippet.is_empty() {
            out.push_str(&format!("   {}\n", r.snippet));
        }
    }
    out
}

/// Clamp max_results to 1..=20, defaulting to 5.
pub(crate) fn clamp_max_results(input: &serde_json::Value) -> usize {
    let v = input
        .get("max_results")
        .and_then(|v| v.as_i64())
        .unwrap_or(5);
    (v.max(1).min(20)) as usize
}

#[async_trait::async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "WebSearch"
    }

    fn description(&self) -> &str {
        "Searches the web using DuckDuckGo and returns a list of results \
         with titles, URLs, and snippets."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query"
                },
                "max_results": {
                    "type": "integer",
                    "description": "Max results (default 5, max 20)"
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let query = match input.get("query").and_then(|v| v.as_str()) {
            Some(q) if !q.trim().is_empty() => q,
            _ => return ToolResult::error("query is required and must be a non-empty string"),
        };

        let max_results = clamp_max_results(&input);

        let client = match reqwest::Client::builder()
            .timeout(TIMEOUT)
            .user_agent(USER_AGENT)
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()
        {
            Ok(c) => c,
            Err(e) => return ToolResult::error(format!("Failed to create HTTP client: {e}")),
        };

        let response = match client
            .get("https://html.duckduckgo.com/html/")
            .query(&[("q", query)])
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                if e.is_timeout() {
                    return ToolResult::error(format!(
                        "Search request timed out after {}s",
                        TIMEOUT.as_secs()
                    ));
                }
                return ToolResult::error(format!("Search request failed: {e}"));
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

        let html = match response.text().await {
            Ok(t) => t,
            Err(e) => return ToolResult::error(format!("Failed to read response: {e}")),
        };

        let results = parse_results(&html, max_results);

        if results.is_empty() {
            return ToolResult::success(format!("No results found for: {query}"));
        }

        ToolResult::success(format_results(&results))
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Safe
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
            extra_dirs: vec![],
        }
    }

    #[test]
    fn test_metadata() {
        let tool = WebSearchTool;
        assert_eq!(tool.name(), "WebSearch");
        assert!(!tool.description().is_empty());
        assert!(tool.description().contains("DuckDuckGo"));

        let schema = tool.input_schema();
        assert_eq!(schema["required"][0], "query");
        assert!(schema["properties"]["query"].is_object());
        assert!(schema["properties"]["max_results"].is_object());
    }

    #[test]
    fn test_permission_level() {
        let tool = WebSearchTool;
        assert_eq!(
            tool.permission_level(&json!({"query": "test"})),
            PermissionLevel::Safe
        );
    }

    #[tokio::test]
    async fn test_missing_query_is_error() {
        let tool = WebSearchTool;
        let result = tool.execute(json!({}), &test_ctx()).await;
        assert!(result.is_error);
        assert!(result.content.contains("query is required"));
    }

    #[tokio::test]
    async fn test_empty_query_is_error() {
        let tool = WebSearchTool;
        let result = tool.execute(json!({"query": "  "}), &test_ctx()).await;
        assert!(result.is_error);
        assert!(result.content.contains("query is required"));
    }

    #[test]
    fn test_max_results_clamping() {
        assert_eq!(clamp_max_results(&json!({})), 5);
        assert_eq!(clamp_max_results(&json!({"max_results": 0})), 1);
        assert_eq!(clamp_max_results(&json!({"max_results": -5})), 1);
        assert_eq!(clamp_max_results(&json!({"max_results": 10})), 10);
        assert_eq!(clamp_max_results(&json!({"max_results": 50})), 20);
        assert_eq!(clamp_max_results(&json!({"max_results": 20})), 20);
    }

    #[test]
    fn test_extract_ddg_url_with_uddg() {
        let href = "//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fpage&rut=abc";
        assert_eq!(extract_ddg_url(href), "https://example.com/page");
    }

    #[test]
    fn test_extract_ddg_url_without_uddg() {
        let href = "https://example.com/direct";
        assert_eq!(extract_ddg_url(href), "https://example.com/direct");
    }

    #[test]
    fn test_extract_ddg_url_uddg_at_end() {
        let href = "//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com";
        assert_eq!(extract_ddg_url(href), "https://example.com");
    }

    #[test]
    fn test_urlencoded_decode() {
        assert_eq!(urlencoded_decode("hello%20world"), "hello world");
        assert_eq!(urlencoded_decode("a+b"), "a b");
        assert_eq!(
            urlencoded_decode("https%3A%2F%2Fexample.com"),
            "https://example.com"
        );
        assert_eq!(urlencoded_decode("no_encoding"), "no_encoding");
    }

    #[test]
    fn test_strip_tags() {
        assert_eq!(strip_tags("<b>bold</b> text"), "bold text");
        assert_eq!(strip_tags("no tags"), "no tags");
        assert_eq!(
            strip_tags("<a href=\"x\">link</a>"),
            "link"
        );
    }

    #[test]
    fn test_decode_entities() {
        assert_eq!(decode_entities("A &amp; B"), "A & B");
        assert_eq!(decode_entities("&lt;tag&gt;"), "<tag>");
        assert_eq!(decode_entities("&quot;quoted&#39;"), "\"quoted'");
    }

    #[test]
    fn test_parse_results_basic() {
        let html = r#"
        <a class="result__a" href="https://example.com">Example Title</a>
        <a class="result__snippet">This is a <b>snippet</b> text</a>
        <a class="result__a" href="https://other.com">Other Title</a>
        <a class="result__snippet">Another snippet</a>
        "#;

        let results = parse_results(html, 5);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "Example Title");
        assert_eq!(results[0].url, "https://example.com");
        assert_eq!(results[0].snippet, "This is a snippet text");
        assert_eq!(results[1].title, "Other Title");
    }

    #[test]
    fn test_parse_results_respects_max() {
        let html = r#"
        <a class="result__a" href="https://a.com">A</a>
        <a class="result__snippet">Snippet A</a>
        <a class="result__a" href="https://b.com">B</a>
        <a class="result__snippet">Snippet B</a>
        <a class="result__a" href="https://c.com">C</a>
        <a class="result__snippet">Snippet C</a>
        "#;

        let results = parse_results(html, 2);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_parse_results_empty() {
        let html = "<html><body>No results here</body></html>";
        let results = parse_results(html, 5);
        assert!(results.is_empty());
    }

    #[test]
    fn test_format_results() {
        let results = vec![
            SearchResult {
                title: "First Result".to_string(),
                url: "https://first.com".to_string(),
                snippet: "First snippet".to_string(),
            },
            SearchResult {
                title: "Second Result".to_string(),
                url: "https://second.com".to_string(),
                snippet: "Second snippet".to_string(),
            },
        ];

        let formatted = format_results(&results);
        assert!(formatted.contains("1. First Result"));
        assert!(formatted.contains("URL: https://first.com"));
        assert!(formatted.contains("First snippet"));
        assert!(formatted.contains("2. Second Result"));
        assert!(formatted.contains("URL: https://second.com"));
    }

    #[test]
    fn test_format_results_empty_snippet() {
        let results = vec![SearchResult {
            title: "No Snippet".to_string(),
            url: "https://example.com".to_string(),
            snippet: String::new(),
        }];

        let formatted = format_results(&results);
        assert!(formatted.contains("1. No Snippet"));
        assert!(formatted.contains("URL: https://example.com"));
        // Should not have an empty snippet line
        let lines: Vec<&str> = formatted.lines().collect();
        assert_eq!(lines.len(), 2); // title + url only
    }
}
