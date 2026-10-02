//! The one place archon-mcp builds a `reqwest::Client`.
//!
//! Every MCP HTTP client (Streamable HTTP, legacy SSE stream and POST
//! channel, OAuth token endpoint) starts from [`mcp_http_client_builder`], so
//! the redirect policy below cannot drift between transports. A guard test
//! fails if any other non-test module constructs a client or sets a redirect
//! policy itself.

/// A `reqwest::ClientBuilder` with automatic redirects disabled.
///
/// reqwest's default policy follows 307/308 and strips only `Authorization`,
/// `Cookie` and `Proxy-Authorization` on a cross-origin hop. Custom auth
/// headers such as `X-Api-Key` would be replayed to the redirect target
/// (GHSA-9g45-5xwm-f3wc), and a 307/308 on the OAuth token endpoint would
/// replay the form body with the code verifier or refresh token. A redirect
/// response is returned to the caller as an ordinary non-success status.
///
/// Callers add their own timeouts; they must not override the redirect policy.
pub(crate) fn mcp_http_client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder().redirect(reqwest::redirect::Policy::none())
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    const THIS_FILE: &str = "http_client.rs";
    const FORBIDDEN: [&str; 3] = ["Client::builder(", "Client::new(", ".redirect("];

    fn rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).expect("read src dir") {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                rust_sources(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                out.push(path);
            }
        }
    }

    /// Production code only: test-only files are skipped and each file is cut
    /// at its first `#[cfg(test)]`, where the in-file test module starts.
    fn production_code(path: &Path) -> Option<String> {
        let name = path.file_name()?.to_str()?;
        if name == THIS_FILE || name == "tests.rs" || name.ends_with("_tests.rs") {
            return None;
        }
        let source = std::fs::read_to_string(path).expect("read source");
        let end = source.find("#[cfg(test)]").unwrap_or(source.len());
        Some(source[..end].to_string())
    }

    #[test]
    fn every_mcp_http_client_comes_from_the_shared_builder() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        rust_sources(&src, &mut files);
        assert!(
            files.len() > 10,
            "source scan found too few files: {files:?}"
        );

        let mut offenders = Vec::new();
        for path in &files {
            let Some(code) = production_code(path) else {
                continue;
            };
            for (index, line) in code.lines().enumerate() {
                let code_part = line.split("//").next().unwrap_or("");
                if FORBIDDEN.iter().any(|needle| code_part.contains(needle)) {
                    offenders.push(format!("{}:{}: {}", path.display(), index + 1, line.trim()));
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "build MCP HTTP clients with http_client::mcp_http_client_builder(); found:\n{}",
            offenders.join("\n")
        );
    }

    #[test]
    fn shared_builder_builds() {
        super::mcp_http_client_builder()
            .build()
            .expect("shared MCP HTTP client builds");
    }
}
