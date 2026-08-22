use super::*;

fn repo_with(markers: &[&str]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    for marker in markers {
        std::fs::write(dir.path().join(marker), "").expect("marker");
    }
    dir
}

fn value_of<'a>(env: &'a [(String, String)], key: &str) -> Option<&'a str> {
    env.iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.as_str())
}

/// A repository the engine recognises gets that toolchain's variables, and
/// nobody else's — a Rust project must not be handed Go or Node cache paths.
#[test]
fn only_the_toolchains_present_contribute_variables() {
    let repo = repo_with(&["Cargo.toml"]);
    let lease = std::path::Path::new("/leases/build-cache-0");

    let env = cache_env_for_repository(repo.path(), lease, &[]);

    assert!(value_of(&env, "CARGO_TARGET_DIR").is_some());
    assert!(value_of(&env, "GOCACHE").is_none());
    assert!(value_of(&env, "npm_config_cache").is_none());
}

/// The engine is not a Rust engine. A Go repository gets Go's variables and no
/// mention of cargo.
#[test]
fn a_non_rust_repository_gets_its_own_toolchain() {
    let repo = repo_with(&["go.mod"]);

    let env = cache_env_for_repository(repo.path(), std::path::Path::new("/leases/x"), &[]);

    assert!(value_of(&env, "GOCACHE").is_some());
    assert!(value_of(&env, "GOMODCACHE").is_some());
    assert!(value_of(&env, "CARGO_TARGET_DIR").is_none());
}

/// A polyglot repository gets both, in separate subdirectories so one
/// toolchain cannot overwrite the other's cache.
#[test]
fn a_polyglot_repository_separates_each_toolchain() {
    let repo = repo_with(&["Cargo.toml", "package.json"]);

    let env = cache_env_for_repository(repo.path(), std::path::Path::new("/leases/x"), &[]);

    let cargo = value_of(&env, "CARGO_TARGET_DIR").expect("cargo");
    let node = value_of(&env, "npm_config_cache").expect("node");
    assert_ne!(cargo, node, "toolchains must not share one directory");
}

/// A repository using nothing recognised gets nothing invented for it.
#[test]
fn an_unrecognised_repository_gets_no_cache_variables() {
    let repo = repo_with(&["README.md"]);

    let env = cache_env_for_repository(repo.path(), std::path::Path::new("/leases/x"), &[]);

    assert!(env.is_empty(), "nothing should be assumed: {env:?}");
}

/// A project whose toolchain the table does not know declares its own key,
/// rather than waiting for the engine to learn about it.
#[test]
fn a_project_declared_key_is_honoured_without_a_marker() {
    let repo = repo_with(&["README.md"]);

    let env = cache_env_for_repository(
        repo.path(),
        std::path::Path::new("/leases/x"),
        &["ZIG_GLOBAL_CACHE_DIR".to_string()],
    );

    assert!(value_of(&env, "ZIG_GLOBAL_CACHE_DIR").is_some());
}

/// A project-declared key never overwrites a toolchain's own value — the
/// recognised mapping wins, so a project cannot accidentally redirect cargo.
#[test]
fn a_project_declared_key_does_not_override_a_known_toolchain() {
    let repo = repo_with(&["Cargo.toml"]);
    let lease = std::path::Path::new("/leases/x");

    let env = cache_env_for_repository(repo.path(), lease, &["CARGO_TARGET_DIR".to_string()]);

    let value = value_of(&env, "CARGO_TARGET_DIR").expect("cargo");
    assert!(
        value.ends_with("cargo"),
        "toolchain mapping must win: {value}"
    );
    assert_eq!(
        env.iter()
            .filter(|(name, _)| name == "CARGO_TARGET_DIR")
            .count(),
        1,
        "one value only: {env:?}"
    );
}

/// The slot names the directory, not the agent. Two agents holding slot 0 at
/// different times get the same path — which is what lets the second one find
/// the first one's dependency artifacts already built.
#[test]
fn a_slot_directory_is_stable_across_the_agents_that_hold_it() {
    let root = std::path::Path::new("/leases");

    assert_eq!(lease_slot_dir(root, 0), lease_slot_dir(root, 0));
    assert_ne!(lease_slot_dir(root, 0), lease_slot_dir(root, 1));
}

/// A wrapper reaches only a repository whose toolchain reads it. A Go project
/// must not be handed RUSTC_WRAPPER.
#[test]
fn a_compiler_cache_wrapper_applies_only_to_a_toolchain_that_reads_it() {
    let rust = repo_with(&["Cargo.toml"]);
    let go = repo_with(&["go.mod"]);

    assert!(!compiler_cache_env_for_repository(rust.path(), "sccache").is_empty());
    assert!(compiler_cache_env_for_repository(go.path(), "sccache").is_empty());
}

/// Nothing is inferred: no configured wrapper means no wrapper. A build that
/// silently routes through a tool nobody chose is harder to explain than a slow
/// one.
#[test]
fn no_configured_wrapper_means_no_wrapper_variables() {
    let repo = repo_with(&["Cargo.toml"]);

    assert!(compiler_cache_env_for_repository(repo.path(), "").is_empty());
    assert!(compiler_cache_env_for_repository(repo.path(), "   ").is_empty());
}

/// An absolute path is honoured, and matched on its file name — an operator
/// pinning a specific binary should not have to also be lucky about how the
/// table spells it.
#[test]
fn an_absolute_wrapper_path_is_matched_by_its_name() {
    let repo = repo_with(&["Cargo.toml"]);

    let env = compiler_cache_env_for_repository(repo.path(), "/opt/homebrew/bin/sccache");

    assert_eq!(
        value_of(&env, "RUSTC_WRAPPER"),
        Some("/opt/homebrew/bin/sccache"),
        "the configured path must be used verbatim: {env:?}"
    );
}

/// A wrapper this engine has no mapping for is not guessed at.
#[test]
fn an_unknown_wrapper_sets_nothing() {
    let repo = repo_with(&["Cargo.toml"]);

    assert!(compiler_cache_env_for_repository(repo.path(), "some-unknown-cache").is_empty());
}
