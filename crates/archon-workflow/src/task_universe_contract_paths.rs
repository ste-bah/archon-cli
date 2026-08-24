//! Repairing the two declared-path mistakes task authors actually make.
//!
//! Split from `task_universe_parsing.rs` to keep that file inside the project's
//! 500-line budget; these are parse-time rules and they belong beside the
//! parser that uses them.
//!
//! Both rules repair rather than reject, for the same reason: one task file
//! that fails to parse refuses the whole task *set*, so a single mistake in one
//! of fifteen specs costs the other fourteen.

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

/// Token names that mean "the project root", which a declared path must not
/// carry: every relative path is already resolved against it.
const ROOT_TOKEN_NAMES: [&str; 4] = [
    "PROJECT_ROOT",
    "REPO_ROOT",
    "REPOSITORY_ROOT",
    "WORKSPACE_ROOT",
];

/// Every path-valued key of a deliverable contract.
const PATH_KEYS: [&str; 4] = [
    "artifact_path",
    "registry_path",
    "instance_source_path",
    "payload_path",
];

/// Rewrite shell-style `${NAME}` placeholders into the `<NAME>` form the
/// verifier binds.
///
/// # Why this is a parse-time repair and not a run-time failure
///
/// The verifier supports templated declared paths: `<DATASET_ID>` bound by
/// `min_instances >= 1` or by an instance source. It refuses `${DATASET_ID}`
/// unconditionally, and correctly — an unset shell variable expands to nothing
/// and silently turns an absolute path relative, so a gate built on one can
/// pass without anyone looking at a file.
///
/// But the two syntaxes look identical to a task author, and nothing between
/// writing the file and running the gate said so. Observed live: a task
/// declared `${PROJECT_ROOT}/…/${DATASET_ID}/${VERSION}/validation.json`
/// *together with* `min_instances: 1` — every binding the verifier asks for,
/// in the one syntax it will not read. The defect surfaced seventeen hours into
/// the run, as a contract failure on a task whose code was complete and whose
/// eleven focused tests passed, and it consumed four remediation cycles that
/// could not have fixed it, because no code change can satisfy a path the gate
/// refuses to parse.
///
/// So the repair happens here, where the file is read, for every workflow and
/// every task standard rather than for one PRD.
///
/// # What is repaired and what is left alone
///
/// * `${PROJECT_ROOT}/` and its synonyms are **dropped**, with the separator
///   that follows: a relative declared path is already resolved against the
///   project root, so keeping the token would join the root to itself.
/// * `${NAME}` becomes `<NAME>`, but **only when `NAME` is a plain
///   identifier**. `${HOST:-127.0.0.1}` carries a shell default, which is a
///   value this cannot invent — it is left exactly as written so the verifier's
///   existing fail-closed error still names it.
/// * A path with no `${` is returned untouched.
pub(super) fn normalize_shell_path_tokens(declared: &serde_json::Value) -> serde_json::Value {
    let serde_json::Value::Array(entries) = declared else {
        return declared.clone();
    };
    let repaired = entries
        .iter()
        .map(|entry| {
            let Some(object) = entry.as_object() else {
                return entry.clone();
            };
            let mut object = object.clone();
            for key in PATH_KEYS {
                let Some(serde_json::Value::String(path)) = object.get(key) else {
                    continue;
                };
                let rewritten = rewrite_path(path);
                if &rewritten != path {
                    object.insert(key.to_string(), serde_json::Value::String(rewritten));
                }
            }
            serde_json::Value::Object(object)
        })
        .collect();
    serde_json::Value::Array(repaired)
}

/// One declared path, with its shell tokens repaired.
fn rewrite_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    let mut rest = path;
    while let Some(open) = rest.find("${") {
        let Some(close) = rest[open..].find('}') else {
            break;
        };
        let name = &rest[open + 2..open + close];
        out.push_str(&rest[..open]);
        rest = &rest[open + close + 1..];
        if !is_plain_identifier(name) {
            // A shell default or an expression: not ours to invent a value for.
            out.push_str("${");
            out.push_str(name);
            out.push('}');
        } else if ROOT_TOKEN_NAMES.contains(&name) {
            // Drop the separator too, so the remainder stays relative.
            rest = rest.strip_prefix('/').unwrap_or(rest);
        } else {
            out.push('<');
            out.push_str(name);
            out.push('>');
        }
    }
    out.push_str(rest);
    out
}

/// Whether a token names a variable rather than carrying an expression.
fn is_plain_identifier(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
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

    /// The live defect: `${PROJECT_ROOT}/…/${DATASET_ID}/${VERSION}/…` declared
    /// beside `min_instances: 1` — every binding the verifier asks for, in the
    /// one syntax it refuses to read.
    #[test]
    fn the_shell_form_becomes_the_form_the_verifier_binds() {
        let out = normalize_shell_path_tokens(&json!([{
            "kind": "per_dataset_version_validation_report",
            "artifact_path": "${PROJECT_ROOT}/.archon/data/datasets/${DATASET_ID}/${VERSION}/validation.json",
            "min_instances": 1
        }]));
        assert_eq!(
            out[0]["artifact_path"],
            json!(".archon/data/datasets/<DATASET_ID>/<VERSION>/validation.json")
        );
        assert_eq!(
            out[0]["min_instances"],
            json!(1),
            "the binding must survive"
        );
    }

    /// A root token mid-path must not leave a doubled separator behind.
    #[test]
    fn every_root_synonym_is_dropped_with_its_separator() {
        for name in ROOT_TOKEN_NAMES {
            let out = normalize_shell_path_tokens(&json!([{
                "artifact_path": format!("${{{name}}}/src/a.rs")
            }]));
            assert_eq!(out[0]["artifact_path"], json!("src/a.rs"), "{name}");
        }
    }

    /// A shell default carries a value this cannot invent. Rewriting it would
    /// silently declare `<HOST:-127.0.0.1>` a template name; leaving it lets
    /// the verifier's existing fail-closed error name it instead.
    #[test]
    fn an_expression_is_left_for_the_verifier_to_refuse() {
        let path = "${OPENBB_HOST:-127.0.0.1}/out.json";
        let out = normalize_shell_path_tokens(&json!([{"artifact_path": path}]));
        assert_eq!(out[0]["artifact_path"], json!(path));
    }

    #[test]
    fn every_path_key_is_repaired_not_only_the_artifact() {
        let out = normalize_shell_path_tokens(&json!([{
            "artifact_path": "${PROJECT_ROOT}/a/${ID}.json",
            "registry_path": "${PROJECT_ROOT}/registry.json",
            "instance_source_path": "${PROJECT_ROOT}/source.json",
            "payload_path": "${PROJECT_ROOT}/payload.json"
        }]));
        assert_eq!(out[0]["artifact_path"], json!("a/<ID>.json"));
        assert_eq!(out[0]["registry_path"], json!("registry.json"));
        assert_eq!(out[0]["instance_source_path"], json!("source.json"));
        assert_eq!(out[0]["payload_path"], json!("payload.json"));
    }

    #[test]
    fn a_concrete_path_is_returned_untouched() {
        let declared = json!([{"artifact_path": "crates/archon-trading/src/data_lake.rs"}]);
        assert_eq!(normalize_shell_path_tokens(&declared), declared);
    }

    /// The plural expansion runs first, so a plural entry carrying shell tokens
    /// is repaired in both respects.
    #[test]
    fn the_two_repairs_compose() {
        let out = normalize_shell_path_tokens(&expand_plural_artifact_paths(&json!([{
            "kind": "impl",
            "artifact_paths": ["${PROJECT_ROOT}/a/${ID}.json", "${PROJECT_ROOT}/b.json"]
        }])));
        assert_eq!(out[0]["artifact_path"], json!("a/<ID>.json"));
        assert_eq!(out[1]["artifact_path"], json!("b.json"));
    }
}
