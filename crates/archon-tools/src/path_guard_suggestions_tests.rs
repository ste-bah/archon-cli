use super::resolve_existing_file_path;
use crate::tool::ToolContext;

fn context(root: &std::path::Path) -> ToolContext {
    ToolContext {
        working_dir: root.to_path_buf(),
        ..ToolContext::default()
    }
}

#[test]
fn missing_path_suggests_a_nearby_name_and_keeps_the_error_prefix() {
    let temp = tempfile::tempdir().unwrap();
    let parent = temp.path().join("author-context");
    std::fs::create_dir_all(&parent).unwrap();
    let existing_name = "4ef7d9f99ee6905655e982cfb9794767bdc2fb5f19c624ec29715574581460c8.json";
    std::fs::write(parent.join(existing_name), "context").unwrap();
    let missing_name = "4ef7d9f99ee690655e982cfb9794767bdc2fb5f19c624ec29715574581460c8.json";

    let error = resolve_existing_file_path(
        &parent.join(missing_name).display().to_string(),
        &context(temp.path()),
    )
    .unwrap_err()
    .to_string();

    assert!(
        error.starts_with(&format!(
            "File does not exist: {}",
            parent.join(missing_name).display()
        )),
        "{error}"
    );
    assert!(
        error.contains(&format!(
            "Did you mean: {}?",
            archon_shell::paths::plain(parent.canonicalize().unwrap())
                .join(existing_name)
                .display()
        )),
        "{error}"
    );
}

#[test]
fn missing_path_does_not_suggest_a_distant_name() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("completely-different.txt"), "content").unwrap();

    let error = resolve_existing_file_path(
        &temp.path().join("unrelated.rs").display().to_string(),
        &context(temp.path()),
    )
    .unwrap_err()
    .to_string();

    assert!(error.starts_with("File does not exist:"), "{error}");
    assert!(!error.contains("Did you mean:"), "{error}");
}

#[test]
fn missing_path_does_not_list_a_parent_outside_allowed_directories() {
    let temp = tempfile::tempdir().unwrap();
    let allowed = temp.path().join("allowed");
    let outside = temp.path().join("outside");
    std::fs::create_dir_all(&allowed).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("secret.rs"), "secret").unwrap();

    let error = resolve_existing_file_path(
        &outside.join("secref.rs").display().to_string(),
        &context(&allowed),
    )
    .unwrap_err()
    .to_string();

    assert!(error.starts_with("File does not exist:"), "{error}");
    assert!(!error.contains("secret.rs"), "{error}");
    assert!(!error.contains("Did you mean:"), "{error}");
}
