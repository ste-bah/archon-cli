//! Workspace lint: non-test code replaces a child's environment only through
//! `archon_shell::spawn::replace_environment`, which applies the jobserver
//! policy after the overlay. A bare `env_clear` followed by `envs(host)`
//! brings this process's stale `--jobserver-auth=R,W` back to the child.
use super::*;

fn env_violations(relative: &str, source: &str) -> Vec<String> {
    if relative == HELPER || is_test_path(relative) {
        return Vec::new();
    }
    let masked = lex::code(source);
    let all_lines: Vec<&str> = masked.lines().collect();
    production_lines(source)
        .into_iter()
        .filter(|(number, _)| all_lines[number - 1].contains(".env_clear("))
        .map(|(number, line)| format!("{relative}:{number}: {}", line.trim()))
        .collect()
}

#[test]
fn non_test_code_replaces_child_environments_only_through_the_helper() {
    let root = workspace_root();
    let files = workspace_rust_files(&root);
    assert!(root.join(HELPER).is_file(), "workspace root not found");
    assert!(files.len() > 100, "only {} files scanned", files.len());
    let mut found = Vec::new();
    for path in files {
        let relative = path
            .strip_prefix(&root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        let Ok(source) = std::fs::read_to_string(&path) else {
            continue;
        };
        found.extend(env_violations(&relative, &source));
    }
    assert!(
        found.is_empty(),
        "replace a child environment with archon_shell::spawn::replace_environment, \
         which sanitizes jobserver flags after the overlay:\n{}",
        found.join("\n")
    );
}

#[test]
fn the_env_lint_sees_production_code_and_not_test_items_or_comments() {
    let source = "fn run(c: &mut Command) {\n    c.env_clear().envs(host());\n}\n\
                  // c.env_clear() in a comment\n\
                  #[cfg(test)]\nmod tests {\n    fn t(c: &mut Command) { c.env_clear(); }\n}\n";
    assert_eq!(
        env_violations("crates/x/src/run.rs", source),
        vec!["crates/x/src/run.rs:2: c.env_clear().envs(host());".to_string()]
    );
    assert!(env_violations("crates/x/src/run_tests.rs", source).is_empty());
    assert!(env_violations(HELPER, source).is_empty());
}
