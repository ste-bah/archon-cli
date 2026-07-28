fn prompt_line(prompt: &str, prefix: &str) -> Option<String> {
    prompt
        .lines()
        .find_map(|line| line.trim().strip_prefix(prefix).map(str::trim))
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn prompt_input(prompt: &str) -> serde_json::Value {
    let Some(after) = prompt.split("## Input\n```json\n").nth(1) else {
        return serde_json::Value::Null;
    };
    let Some(raw) = after.split("\n```").next() else {
        return serde_json::Value::Null;
    };
    serde_json::from_str(raw).unwrap_or(serde_json::Value::Null)
}

fn find_item(value: &serde_json::Value) -> Option<&serde_json::Value> {
    match value {
        serde_json::Value::Object(object) => {
            if object.contains_key("canonical_task_ids")
                && (object.contains_key("target_files")
                    || object.contains_key("focused_verification"))
            {
                return Some(value);
            }
            object.values().find_map(find_item)
        }
        serde_json::Value::Array(values) => values.iter().find_map(find_item),
        _ => None,
    }
}

fn first_string(value: Option<&serde_json::Value>) -> Option<String> {
    value
        .and_then(serde_json::Value::as_array)
        .and_then(|values| values.first())
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}

fn find_string_key(value: &serde_json::Value, target: &str) -> Option<String> {
    match value {
        serde_json::Value::Object(object) => object
            .get(target)
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .or_else(|| {
                object
                    .values()
                    .find_map(|value| find_string_key(value, target))
            }),
        serde_json::Value::Array(values) => values
            .iter()
            .find_map(|value| find_string_key(value, target)),
        _ => None,
    }
}

fn init_git_repo(repo: &std::path::Path) {
    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.name", "archon-test"]);
    run_git(
        repo,
        &["config", "user.email", "archon-test@example.invalid"],
    );
    run_git(repo, &["add", "."]);
    run_git(repo, &["commit", "-m", "initial"]);
}

fn run_git(repo: &std::path::Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .expect("git command starts");
    assert!(
        output.status.success(),
        "git {:?} failed: stdout={} stderr={}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
