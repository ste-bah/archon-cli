//! Issue-13: a formatter or fixer run over the whole tree is refused up front,
//! in its scoped form it runs, and the operator switch disables the refusal.
use archon_tools::workflow_read_guard::{
    TreeWideMutator, WorkflowReadGuard, WorkflowReadGuardSettings, default_tree_wide_mutators,
};
use serde_json::json;

fn refusal(guard: &WorkflowReadGuard, command: &str) -> Option<String> {
    guard.before_tool("Bash", &json!({"command": command}))
}

#[test]
fn every_default_tree_wide_mutator_form_is_refused_with_its_scoped_form() {
    let guard = WorkflowReadGuard::new(40, 20, true, true);
    for (command, scoped) in [
        ("cargo fmt", "cargo fmt -p <crate>"),
        ("cargo fmt --all", "cargo fmt -p <crate>"),
        ("cargo +nightly fmt --all", "cargo fmt -p <crate>"),
        ("cargo fmt --all -- --config x=y", "cargo fmt -p <crate>"),
        (
            "cargo fmt --manifest-path Cargo.toml",
            "cargo fmt -p <crate>",
        ),
        ("cargo check && cargo fmt --all", "cargo fmt -p <crate>"),
        ("ROOT=/x; cd $ROOT; cargo fmt", "cargo fmt -p <crate>"),
        ("rustfmt src", "rustfmt <file>"),
        ("rustfmt .", "rustfmt <file>"),
        ("rustfmt --edition 2024 src/", "rustfmt <file>"),
        ("gofmt -w .", "gofmt -w <file>"),
        ("gofmt -l -w .", "gofmt -w <file>"),
        ("gofmt -w ./...", "gofmt -w <file>"),
        ("goimports -w .", "goimports -w <file>"),
        ("black .", "black <file>"),
        ("black src", "black <file>"),
        ("black --line-length 100 src/", "black <file>"),
        ("python -m black .", "black <file>"),
        ("poetry run black .", "black <file>"),
        ("prettier --write .", "prettier --write <file>"),
        ("prettier -w src", "prettier --write <file>"),
        ("prettier --write 'src/**/*.ts'", "prettier --write <file>"),
        ("npx prettier --write .", "prettier --write <file>"),
        ("npx --yes prettier --write .", "prettier --write <file>"),
        ("pnpm exec prettier --write .", "prettier --write <file>"),
        ("eslint --fix .", "eslint --fix <file>"),
        ("eslint . --fix", "eslint --fix <file>"),
        ("npx eslint --fix src/", "eslint --fix <file>"),
        ("dotnet format", "dotnet format <project> --include <file>"),
        (
            "dotnet format MySolution.sln",
            "dotnet format <project> --include <file>",
        ),
        ("ruff format .", "ruff format <file>"),
        ("ruff format", "ruff format <file>"),
        ("ruff check --fix .", "ruff check --fix <file>"),
        ("isort .", "isort <file>"),
        ("isort src/", "isort <file>"),
    ] {
        let refused = refusal(&guard, command).unwrap_or_else(|| panic!("{command} was allowed"));
        assert!(refused.contains("is refused"), "{command}: {refused}");
        assert!(
            refused.contains("rewrites the whole tree"),
            "{command}: {refused}"
        );
        assert!(refused.contains(scoped), "{command}: {refused}");
        assert!(
            refused.contains("workflow.generated.allow_tree_wide_mutators"),
            "{command}: {refused}"
        );
    }
}

#[test]
fn scoped_read_only_and_unrelated_forms_run() {
    let guard = WorkflowReadGuard::new(40, 20, true, true);
    for command in [
        "cargo fmt -p foo",
        "cargo fmt --package foo",
        "cargo fmt --package=foo",
        "cargo +nightly fmt -p foo -- --check",
        "cargo fmt -- src/x.rs",
        "cargo fmt -- src/x.rs src/y.rs",
        "cargo fmt --all --check",
        "cargo fmt --check",
        "cargo fmt -- --check",
        "cargo fmt --all -- --check",
        "cargo check --all",
        "cargo build --all",
        "cargo test -p foo",
        "rustfmt src/x.rs",
        "rustfmt --edition 2024 src/x.rs crates/a/src/lib.rs",
        "rustfmt --check src/x.rs",
        "gofmt -w main.go",
        "gofmt -l .",
        "gofmt -d .",
        "goimports -w cmd/main.go",
        "black src/a.py",
        "black --check .",
        "black --diff src",
        "python -m black src/a.py",
        "prettier --write src/a.ts",
        "prettier --write src/a.ts src/b.tsx",
        "prettier --check .",
        "prettier src/a.ts",
        "npx prettier --write src/a.ts",
        "eslint .",
        "eslint --fix src/a.ts",
        "eslint --fix-dry-run .",
        "dotnet format --verify-no-changes",
        "dotnet format MySolution.sln --include src/A.cs",
        "dotnet build",
        "ruff format src/a.py",
        "ruff check .",
        "ruff check --fix src/a.py",
        "ruff format --check .",
        "isort src/a.py",
        "isort --check-only .",
        "isort --diff src",
        "echo cargo fmt --all",
        "# cargo fmt --all",
        "git status",
    ] {
        assert!(refusal(&guard, command).is_none(), "{command} was refused");
    }
}

#[test]
fn the_operator_switch_disables_the_refusal_and_the_rules_are_configurable() {
    let allowed = WorkflowReadGuard::from_settings(&WorkflowReadGuardSettings {
        allow_tree_wide_mutators: true,
        ..WorkflowReadGuardSettings::default()
    });
    for command in ["cargo fmt --all", "black .", "prettier --write ."] {
        assert!(refusal(&allowed, command).is_none(), "{command}");
    }
    // The guard's other refusals are untouched by the switch.
    assert!(refusal(&allowed, "cargo build --release").is_some());
    assert!(refusal(&allowed, "git stash pop").is_some());

    let custom = WorkflowReadGuard::from_settings(&WorkflowReadGuardSettings {
        tree_wide_mutators: vec![TreeWideMutator {
            program: "myfmt".into(),
            mutating_flags: vec!["--write".into()],
            scoped_form: "myfmt --write <file>".into(),
            ..TreeWideMutator::default()
        }],
        ..WorkflowReadGuardSettings::default()
    });
    assert!(
        refusal(&custom, "cargo fmt --all").is_none(),
        "a configured list replaces the defaults"
    );
    let refused = refusal(&custom, "myfmt --write .").expect("configured mutator refused");
    assert!(refused.contains("myfmt --write <file>"), "{refused}");
    assert!(refusal(&custom, "myfmt --write src/a.x").is_none());
    assert!(
        refusal(&custom, "myfmt .").is_none(),
        "no mutating flag, no write"
    );
}

#[test]
fn a_refused_mutator_never_spends_the_read_budget() {
    let guard = WorkflowReadGuard::new(2, 20, true, true);
    assert!(refusal(&guard, "cargo fmt --all").is_some());
    assert!(refusal(&guard, "black .").is_some());
    // The read allowance is untouched: both reads are still admitted.
    assert!(
        guard
            .before_tool("Read", &json!({"file_path":"f"}))
            .is_none()
    );
    assert!(
        guard
            .before_tool("Read", &json!({"file_path":"f"}))
            .is_none()
    );
    assert!(
        guard
            .before_tool("Read", &json!({"file_path":"f"}))
            .is_some()
    );
}

#[test]
fn generated_config_carries_the_switch_and_the_rules() {
    let default: crate::config::GeneratedWorkflowConfig =
        serde_json::from_value(json!({})).unwrap();
    assert!(!default.allow_tree_wide_mutators);
    assert_eq!(default.tree_wide_mutators, default_tree_wide_mutators());
    let guard = WorkflowReadGuard::from_settings(&WorkflowReadGuardSettings {
        allow_tree_wide_mutators: default.allow_tree_wide_mutators,
        tree_wide_mutators: default.tree_wide_mutators.clone(),
        ..WorkflowReadGuardSettings::default()
    });
    assert!(refusal(&guard, "cargo fmt --all").is_some());

    let custom: crate::config::GeneratedWorkflowConfig = serde_json::from_value(json!({
        "allow_tree_wide_mutators": true,
        "tree_wide_mutators": [{"program": "myfmt", "scoped_form": "myfmt <file>"}]
    }))
    .unwrap();
    assert!(custom.allow_tree_wide_mutators);
    assert_eq!(custom.tree_wide_mutators.len(), 1);
    assert_eq!(custom.tree_wide_mutators[0].program, "myfmt");
    assert!(custom.tree_wide_mutators[0].file_operands_scope);
    // The built-in list is not written back out; a custom one is.
    let dumped = serde_json::to_value(&default).unwrap();
    assert!(dumped.get("tree_wide_mutators").is_none(), "{dumped}");
    let dumped = serde_json::to_value(&custom).unwrap();
    assert_eq!(dumped["tree_wide_mutators"][0]["program"], "myfmt");
}
