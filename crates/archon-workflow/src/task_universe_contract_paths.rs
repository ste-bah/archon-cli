//! Tolerating a plural `artifact_paths` in a deliverable contract.
//!
//! Split from `task_universe_parsing.rs` to keep that file inside the project's
//! 500-line budget; it is one rule and it belongs beside the parser that uses
//! it.

/// Rewrite `artifact_paths: [a, b]` into one contract per path.
///
/// The field is `artifact_path`, singular, and an author who writes the plural
/// gets a serde error that refuses the whole task file — which refuses the
/// whole task *set*, because one unreadable file stops the universe loading.
/// Observed live: a single plural key in one of fifteen specs took down the
/// lint, so nothing was reported about the other fourteen.
///
/// Expanding rather than taking the first path keeps it lossless: two declared
/// artifacts stay two contracts of the same kind. A singular `artifact_path` is
/// untouched, and anything else is left exactly as written so the existing
/// error still names it.
pub(super) fn expand_plural_artifact_paths(declared: &serde_json::Value) -> serde_json::Value {
    let serde_json::Value::Array(entries) = declared else {
        return declared.clone();
    };
    let mut expanded = Vec::with_capacity(entries.len());
    for entry in entries {
        let Some(object) = entry.as_object() else {
            expanded.push(entry.clone());
            continue;
        };
        let Some(serde_json::Value::Array(paths)) = object.get("artifact_paths") else {
            expanded.push(entry.clone());
            continue;
        };
        for path in paths {
            let mut single = object.clone();
            single.remove("artifact_paths");
            single.insert("artifact_path".to_string(), path.clone());
            expanded.push(serde_json::Value::Object(single));
        }
    }
    serde_json::Value::Array(expanded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The live failure: one plural key in one of fifteen specs refused the
    /// whole task set, so the lint reported nothing about any of them.
    #[test]
    fn a_plural_entry_becomes_one_contract_per_path() {
        let out = expand_plural_artifact_paths(&json!([
            {"kind": "impl", "artifact_paths": ["a.rs", "b.rs"]}
        ]));
        assert_eq!(
            out,
            json!([
                {"kind": "impl", "artifact_path": "a.rs"},
                {"kind": "impl", "artifact_path": "b.rs"}
            ])
        );
    }

    #[test]
    fn a_singular_entry_is_untouched() {
        let input = json!([{"kind": "impl", "artifact_path": "a.rs"}]);
        assert_eq!(expand_plural_artifact_paths(&input), input);
    }

    /// Anything else passes through so the existing error still names it,
    /// rather than this quietly swallowing a shape nobody intended.
    #[test]
    fn an_unexpected_shape_is_left_for_the_real_error() {
        let input = json!({"not": "a list"});
        assert_eq!(expand_plural_artifact_paths(&input), input);
    }
}
