//! Import extraction: the source-text half of the dependency graph.
//!
//! Split out of `cartographer.rs` so the extracted names can be asserted on
//! directly. The graph itself keeps no record of which import produced an edge
//! and the on-disk cache stores only symbols and mtimes, so testing this
//! through a scan was not possible.

/// Import names referenced by `source`, in source order.
///
/// Returns an empty `Vec` for a language with no import pattern. Names are
/// returned as written: how a name resolves to a file is the caller's problem,
/// and for every language here except Java it is a heuristic.
pub fn extract_dependencies(source: &str, language: &str) -> Vec<String> {
    if language == "java" {
        return java_dependencies(source);
    }

    let pattern = match language {
        "rust" => r#"(?m)^use\s+([\w:]+)"#,
        "python" => r#"(?m)^(?:import|from)\s+([\w.]+)"#,
        "typescript" | "javascript" => r##"(?m)from\s+['"]([^'"]+)['"]"##,
        "go" => r#"(?m)import\s+"([\w./]+)""#,
        _ => return Vec::new(),
    };

    let re = match regex::Regex::new(pattern) {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("Failed to compile import regex: {e}");
            return Vec::new();
        }
    };

    re.captures_iter(source)
        .filter_map(|cap| cap.get(1))
        .map(|m| m.as_str().to_string())
        .collect()
}

/// Java imports name a fully qualified type, so unlike the heuristic patterns
/// above these names are exact rather than a guess at what a relative path
/// resolves to. Two forms need handling the generic pattern cannot express: an
/// on-demand import (`import java.util.*;`) names the package, and a static
/// import (`import static org.junit.Assert.assertEquals;`) ends in a member
/// name, which is stripped so the edge points at the declaring type.
fn java_dependencies(source: &str) -> Vec<String> {
    static IMPORT_RE: std::sync::LazyLock<Option<regex::Regex>> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"(?m)^\s*import\s+(static\s+)?([\w.]+?)(?:\.\*)?\s*;")
            .inspect_err(|e| tracing::warn!("Failed to compile Java import regex: {e}"))
            .ok()
    });

    let Some(re) = IMPORT_RE.as_ref() else {
        return Vec::new();
    };

    re.captures_iter(source)
        .filter_map(|cap| {
            let path = cap.get(2)?.as_str();
            let name = if cap.get(1).is_some() {
                // Static import: drop the trailing member so the edge points at
                // the type that declares it.
                path.rsplit_once('.').map_or(path, |(ty, _member)| ty)
            } else {
                path
            };
            (!name.is_empty()).then(|| name.to_string())
        })
        .collect()
}
