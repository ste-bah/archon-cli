use super::*;

/// A workspace with one package, `pkg`, under `crates/pkg`.
fn workspace() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let write = |path: &str, text: &str| {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    };
    write("Cargo.toml", "[workspace]\nmembers = [\"crates/pkg\"]\n");
    write(
        "crates/pkg/Cargo.toml",
        "[package]\nname = \"pkg\"\n\n[[test]]\nname = \"declared\"\npath = \"tests/elsewhere/declared_main.rs\"\n",
    );
    write(
        "crates/pkg/src/lib.rs",
        "pub fn f() {}\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn lib_check() {}\n}\n",
    );
    write(
        "crates/pkg/tests/it.rs",
        "mod support;\n#[test]\nfn it_case() {}\n",
    );
    write("crates/pkg/tests/support/mod.rs", "pub fn help() {}\n");
    write(
        "crates/pkg/tests/elsewhere/declared_main.rs",
        "#[test]\nfn d() {}\n",
    );
    write("scripts/check.sh", "exit 0\n");
    write("py/tests/test_a.py", "def test_a(): pass\n");
    write("py/tests/conftest.py", "");
    dir
}

fn run(dir: &tempfile::TempDir, command: &str) -> Resolution {
    let roots = Roots {
        repository: dir.path(),
        project: dir.path(),
    };
    resolve(command, dir.path(), &roots)
}

/// Roles every cargo resolution carries (its build settings): asserted
/// in their own test and left out of the others.
const SETTINGS: [&str; 3] = [
    "build script",
    "cargo configuration",
    "manifest test setting",
];

fn files(resolution: &Resolution) -> Vec<String> {
    resolution
        .found
        .iter()
        .filter(|found| !SETTINGS.contains(&found.role.as_str()))
        .map(|found| match &found.item {
            Some(item) => format!("{}#{}", found.path, item),
            None => found.path.clone(),
        })
        .collect()
}

#[test]
fn an_integration_target_pins_its_file_and_every_module_it_loads() {
    let dir = workspace();
    let r = run(&dir, "cargo test -p pkg --test it it_case -- --exact");
    assert_eq!(
        files(&r),
        [
            "crates/pkg/Cargo.toml#toml:test:it",
            "crates/pkg/tests/it.rs",
            "crates/pkg/tests/support/mod.rs"
        ]
    );
    assert!(r.watches.is_empty() && r.unresolved.is_empty(), "{r:?}");
    let declared = run(&dir, "cargo nextest run -p pkg --test declared");
    assert_eq!(
        files(&declared),
        [
            "crates/pkg/Cargo.toml#toml:test:declared",
            "crates/pkg/tests/elsewhere/declared_main.rs"
        ]
    );
}

#[test]
fn a_target_missing_at_pin_time_is_pinned_absent_and_its_directory_watched() {
    let dir = workspace();
    let r = run(&dir, "cd crates/pkg && cargo test --test later");
    assert_eq!(
        files(&r),
        [
            "crates/pkg/Cargo.toml#toml:test:later",
            "crates/pkg/tests/later.rs"
        ]
    );
    let target = r
        .found
        .iter()
        .find(|f| f.path == "crates/pkg/tests/later.rs")
        .unwrap();
    assert_eq!(target.role, "integration test target (absent at pin)");
    assert_eq!(r.watches[0].dir, "crates/pkg/tests/later");
    assert_eq!(r.watches[0].test_name, None);
}

#[test]
fn a_filter_pins_unit_tests_as_items_and_watches_a_name_defined_nowhere() {
    let dir = workspace();
    let r = run(&dir, "cargo test -p pkg lib_check");
    assert_eq!(
        files(&r),
        [
            "crates/pkg/src/lib.rs#cfg:",
            "crates/pkg/src/lib.rs#fn:tests::lib_check",
            "crates/pkg/src/lib.rs#mod:tests",
        ]
    );
    let it = run(&dir, "cargo test -p pkg tests::it_case");
    assert_eq!(
        files(&it),
        ["crates/pkg/tests/it.rs", "crates/pkg/tests/support/mod.rs"]
    );
    let later = run(&dir, "cargo test -p pkg not_written_yet -- --exact");
    assert!(files(&later).is_empty());
    assert_eq!(later.watches.len(), 1);
    assert_eq!(later.watches[0].dir, "crates/pkg");
    assert_eq!(
        later.watches[0].test_name.as_deref(),
        Some("not_written_yet")
    );
    assert!(later.watches[0].exact);
}

#[test]
fn no_selection_pins_the_whole_suite() {
    let dir = workspace();
    let r = run(&dir, "cargo test -p pkg");
    assert_eq!(
        files(&r),
        [
            "crates/pkg/src/lib.rs#cfg:",
            "crates/pkg/src/lib.rs#fn:tests::lib_check",
            "crates/pkg/src/lib.rs#mod:tests",
            "crates/pkg/tests/elsewhere/declared_main.rs",
            "crates/pkg/tests/it.rs",
            "crates/pkg/tests/support/mod.rs"
        ]
    );
}

#[test]
fn scripts_pytest_and_nested_shells_are_followed() {
    let dir = workspace();
    let r = run(
        &dir,
        "FOO=1 timeout 60 bash scripts/check.sh > /dev/null && bash -c 'cd py && python3 -m pytest tests/test_a.py::test_a -q'",
    );
    assert_eq!(
        files(&r),
        [
            "py/tests/conftest.py",
            "py/tests/test_a.py",
            "scripts/check.sh"
        ]
    );
    let direct = run(&dir, "./scripts/check.sh; ./target/debug/tool --x");
    assert_eq!(files(&direct), ["scripts/check.sh"]);
    let missing = run(&dir, "node tools/new.js");
    assert_eq!(files(&missing), ["tools/new.js"]);
}

#[test]
fn what_cannot_be_followed_is_recorded_with_why() {
    let dir = workspace();
    let r = run(
        &dir,
        "make test && cargo test -p ghost && pytest -q && cargo build",
    );
    assert!(r.found.is_empty(), "{r:?}");
    assert_eq!(r.unresolved.len(), 3, "{:?}", r.unresolved);
    assert!(r.unresolved.iter().any(|u| u.contains("task-runner")));
    assert!(r.unresolved.iter().any(|u| u.contains("package `ghost`")));
    assert!(r.unresolved.iter().any(|u| u.contains("pytest discovers")));
    let outside = run(&dir, "bash /usr/local/bin/some-tool.sh");
    assert!(outside.unresolved[0].contains("outside the repository"));
}

#[test]
fn a_project_cwd_resolves_through_the_repository_it_overlays() {
    let repo = workspace();
    let project = tempfile::tempdir().unwrap();
    let roots = Roots {
        repository: repo.path(),
        project: project.path(),
    };
    let r = resolve(
        "cargo test -q -p pkg lib_check && bash scripts/check.sh",
        project.path(),
        &roots,
    );
    assert_eq!(
        files(&r),
        [
            "crates/pkg/src/lib.rs#cfg:",
            "crates/pkg/src/lib.rs#fn:tests::lib_check",
            "crates/pkg/src/lib.rs#mod:tests",
            "scripts/check.sh"
        ]
    );
    assert!(
        r.found
            .iter()
            .filter(|f| !SETTINGS.contains(&f.role.as_str()))
            .all(|f| f.root == SourceRoot::Repository)
    );
}

#[test]
fn a_here_document_body_is_data_not_commands() {
    let dir = workspace();
    let r = run(
        &dir,
        "python3 - <<'PY'\nimport os\n./scripts/check.sh\nPY\nx=$((1 << 2)); bash scripts/check.sh",
    );
    assert_eq!(files(&r), ["scripts/check.sh"]);
    let unclosed = run(&dir, "python3 -c 'print(1 << 2)'\n./scripts/check.sh");
    assert_eq!(
        files(&unclosed),
        ["scripts/check.sh"],
        "no delimiter closes it"
    );
}

#[test]
fn a_program_the_resolver_cannot_follow_is_recorded_and_inline_logic_is_noted() {
    let dir = workspace();
    let r = run(&dir, "curl -s localhost | jq -e .ok && mytool --check");
    assert!(r.found.is_empty());
    assert!(r.inline, "jq's filter is written in the command");
    assert_eq!(r.unresolved.len(), 2, "{:?}", r.unresolved);
    assert!(r.unresolved.iter().any(|u| u.contains("`curl`")));
    assert!(r.unresolved.iter().any(|u| u.contains("`mytool`")));
    let bare = run(&dir, "cargo run -q --bin pkg -- check");
    assert!(bare.found.is_empty() && !bare.inline, "{bare:?}");
}

#[test]
fn a_cargo_check_pins_its_build_settings_present_or_absent() {
    let dir = workspace();
    std::fs::create_dir_all(dir.path().join(".cargo")).unwrap();
    std::fs::write(dir.path().join(".cargo/config.toml"), "[build]\n").unwrap();
    let r = run(&dir, "cargo test -p pkg --test it");
    let settings: Vec<(String, Option<String>)> = r
        .found
        .iter()
        .filter(|f| SETTINGS.contains(&f.role.as_str()))
        .map(|f| (f.path.clone(), f.item.clone()))
        .collect();
    for expected in [
        ("crates/pkg/build.rs".to_string(), None),
        (".cargo/config.toml".to_string(), None),
        ("crates/pkg/.cargo/config.toml".to_string(), None),
        (
            "crates/pkg/Cargo.toml".to_string(),
            Some("toml:table:lib".to_string()),
        ),
        (
            "crates/pkg/Cargo.toml".to_string(),
            Some("toml:key:package.autotests".to_string()),
        ),
        (
            "Cargo.toml".to_string(),
            Some("toml:table:profile.test".to_string()),
        ),
    ] {
        assert!(
            settings.contains(&expected),
            "{expected:?} not in {settings:?}"
        );
    }
}

#[test]
fn a_test_pins_its_helpers_includes_fixtures_and_imports() {
    let dir = workspace();
    let write = |path: &str, text: &str| {
        let path = dir.path().join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    };
    write(
        "crates/pkg/src/lib.rs",
        "pub fn f() {}\n#[cfg(test)]\nmod tests {\n    fn helper() -> &'static str { include_str!(\"../tests/data/x.txt\") }\n    #[test]\n    fn lib_check() { let _ = helper(); let _ = std::fs::read(\"tests/data/y.json\"); }\n}\n",
    );
    write("crates/pkg/tests/data/x.txt", "x");
    write("crates/pkg/tests/data/y.json", "{}");
    write("scripts/check.sh", "source scripts/common.sh\nexit 0\n");
    write("scripts/common.sh", "true\n");
    write("py/run.py", "import helpers\nopen(\"data/in.csv\")\n");
    write("py/helpers.py", "X = 1\n");
    write("py/data/in.csv", "a\n");
    let r = run(&dir, "cargo test -p pkg lib_check");
    let found = files(&r);
    for expected in [
        "crates/pkg/src/lib.rs#fn:tests::helper",
        "crates/pkg/tests/data/x.txt",
        "crates/pkg/tests/data/y.json",
    ] {
        assert!(
            found.contains(&expected.to_string()),
            "{expected} not in {found:?}"
        );
    }
    let script = files(&run(
        &dir,
        "bash scripts/check.sh && cd py && python3 run.py",
    ));
    for expected in ["scripts/common.sh", "py/helpers.py", "py/data/in.csv"] {
        assert!(
            script.contains(&expected.to_string()),
            "{expected} not in {script:?}"
        );
    }
    let dynamic = run(&dir, "bash -c 'source \"$HOME/x.sh\"'");
    assert!(
        dynamic
            .unresolved
            .iter()
            .any(|u| u.contains("built at run time")),
        "{dynamic:?}"
    );
}

/// Review minors 5 and 6: a module path leaving the tree and a module chain
/// cycling back through `#[path]` are recorded as unresolved, not cut short
/// silently.
#[test]
fn an_escaping_module_path_and_a_cycling_chain_are_unresolved() {
    let dir = workspace();
    let write = |path: &str, text: &str| std::fs::write(dir.path().join(path), text).unwrap();
    write(
        "crates/pkg/src/lib.rs",
        "#[path = \"../../../../escape.rs\"]\nmod escape;\nmod a;\n",
    );
    write(
        "crates/pkg/src/a.rs",
        "#[path = \"lib.rs\"]\nmod again;\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn cycled() {}\n}\n",
    );
    let r = run(&dir, "cargo test -p pkg cycled");
    assert!(
        r.unresolved.iter().any(|u| u.contains("leaving its tree")),
        "{:?}",
        r.unresolved
    );
    assert!(
        r.unresolved.iter().any(|u| u.contains("cycles back")),
        "{:?}",
        r.unresolved
    );
    assert!(files(&r).contains(&"crates/pkg/src/a.rs#fn:tests::cycled".to_string()));
}
