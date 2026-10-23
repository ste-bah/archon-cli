use super::*;

#[test]
fn shell_positions_and_quoted_fragments_are_classified() {
    // EXPECTED: all required positions parse quotes and `=` forms; prose
    // remains a copy candidate but is not required.
    for command in [
        "cd '../a b'",
        "pushd \"../a\"'b'",
        "git -C=../a status",
        "cargo test --manifest-path=\"../a/Cargo.toml\"",
        "tool --path '../a'",
        "echo hi > '../a/out'",
        "test -f '../a/file'",
        "[ -d '../a/dir' ]",
    ] {
        let paths = scan(command);
        assert!(!paths.is_empty(), "{command}");
        assert!(
            paths.iter().all(|path| path.required),
            "{command}: {paths:?}"
        );
    }
    let prose = scan("echo '../missing'");
    assert_eq!(prose.len(), 1);
    assert!(!prose[0].required);
}

#[test]
fn leading_assignments_are_skipped_when_finding_commands() {
    // EXPECTED before implementation: assignments are environment words,
    // so paths in cd, test, and [ positions remain required.
    for command in [
        "ROOT=1 cd ../missing",
        "A=1 B=2 test -f ../missing",
        "ROOT=1 [ -e ../missing ]",
    ] {
        let paths = scan(command);
        assert_eq!(paths.len(), 1, "{command}: {paths:?}");
        assert!(paths[0].required, "{command}: {paths:?}");
    }
}

#[test]
fn program_text_paths_are_scanned_but_project_paths_are_not_siblings() {
    for command in [
        "python3 <<'PY'\nMANIFEST='../sibling/Cargo.toml'\nPY",
        "python3 -c 'MANIFEST=\"../sibling/Cargo.toml\"'",
    ] {
        let paths = scan(command);
        assert_eq!(paths.len(), 1, "{command}: {paths:?}");
        assert_eq!(paths[0].path, PathBuf::from("../sibling/Cargo.toml"));
        assert!(paths[0].required, "{command}: {paths:?}");
    }
    assert_eq!(
        scan("python3 -c 'MANIFEST=\"crates/inside/Cargo.toml\"'"),
        []
    );
}

#[test]
fn program_paths_trim_list_and_call_separators() {
    for (command, expected) in [
        (
            "runner -c 'run([\"--input\", \"../sibling/package.manifest\"])'",
            "../sibling/package.manifest",
        ),
        (
            "runner -c 'check(\"../sibling/package.manifest\")'",
            "../sibling/package.manifest",
        ),
    ] {
        let paths = scan(command);
        assert_eq!(paths.len(), 1, "{command}: {paths:?}");
        assert_eq!(paths[0].path, PathBuf::from(expected));
        assert!(paths[0].required, "{command}: {paths:?}");
    }
}

#[test]
fn sibling_path_trimming_preserves_non_ascii_letters() {
    let paths = scan("runner -c 'check(\"../sib/café\')'");
    assert_eq!(paths.len(), 1, "{paths:?}");
    assert_eq!(paths[0].path, PathBuf::from("../sib/café"));
}

#[test]
fn parent_paths_are_not_sibling_entries() {
    for command in [
        "echo ../",
        "echo ..",
        "echo ../..",
        "echo ../../x",
        "echo ../x/../../outside",
        "python3 -c 'value = \"../../x\"'",
    ] {
        assert!(scan(command).is_empty(), "{command}: {:?}", scan(command));
    }
}

#[test]
fn short_letter_prefixes_before_quotes_preserve_sibling_paths() {
    for command in [
        "echo f\"../x/y\"",
        "echo r'../x'",
        "echo b\"../x\"",
        "python3 -c 'value = f\"../x/y\"'",
    ] {
        let paths = scan(command);
        assert_eq!(paths.len(), 1, "{command}: {paths:?}");
        assert!(paths[0].path.starts_with("../"), "{command}: {paths:?}");
    }
}

#[test]
fn discovery_never_returns_the_project_parent_or_ancestor() {
    let tree = tempfile::tempdir().unwrap();
    let project = tree.path().join("parent/project");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(tree.path().join("parent/file"), "parent").unwrap();
    let checks = [
        "echo ../".to_string(),
        "echo ../..".to_string(),
        "echo ../../file".to_string(),
        "echo ../project/../..".to_string(),
    ];
    for check in checks {
        assert!(discover(&project, &[check.clone()]).is_empty(), "{check}");
    }
}

#[test]
fn diagnostic_discovery_keeps_missing_and_sibling_paths() {
    let tree = tempfile::tempdir().unwrap();
    let project = tree.path().join("project");
    let sibling = tree.path().join("sibling");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(&sibling).unwrap();
    std::fs::write(sibling.join("input.txt"), "ready").unwrap();
    let found = discover_with_diagnostics(
        &project,
        &["test -f ../sibling/input.txt && test -f ../missing/input.txt".into()],
    );
    assert_eq!(found.siblings.len(), 1);
    assert!(found.candidates.contains(&SiblingCandidate {
        path: PathBuf::from("../sibling/input.txt"),
        reason: SiblingExclusionReason::SiblingTreeFromSelectedRoot,
    }));
    assert!(found.candidates.contains(&SiblingCandidate {
        path: PathBuf::from("../missing/input.txt"),
        reason: SiblingExclusionReason::MissingAtRoot,
    }));
}

/// R2: the shapes a here-document's program text names a sibling path in:
/// an assignment with spaces around `=`, a call that opens a list, and a
/// list of quoted words. Each is seen, and each is required.
#[test]
fn here_document_program_text_names_its_sibling_paths() {
    let command = "set -eu\nrunner - <<'PROGRAM'\nMANIFEST = '../sibling/package.manifest'\nresult = run(['../sibling/bin/tool', '--version'])\nother = run(['tool', '--input', '../sibling/data/input.txt'])\nPROGRAM\n";
    let paths = scan(command);
    for expected in [
        "../sibling/package.manifest",
        "../sibling/bin/tool",
        "../sibling/data/input.txt",
    ] {
        let found = paths
            .iter()
            .find(|path| path.path == PathBuf::from(expected))
            .unwrap_or_else(|| panic!("{expected} is not seen: {paths:?}"));
        assert!(found.required, "{expected}: {paths:?}");
    }
    assert_eq!(paths.len(), 3, "{paths:?}");
}

/// R2: a quoted sibling path glued to the code around it, in a list written
/// without spaces, is still seen in a here-document's program text.
#[test]
fn a_quoted_path_glued_into_a_list_without_spaces_is_seen() {
    let command = "runner - <<'PROGRAM'\nresult = run(['tool','--input','../sibling/data/input.txt','--']+args)\nPROGRAM\n";
    let paths = scan(command);
    assert_eq!(paths.len(), 1, "{paths:?}");
    assert_eq!(paths[0].path, PathBuf::from("../sibling/data/input.txt"));
    assert!(paths[0].required, "{paths:?}");
}

/// R2: a shell comment is no shell text: an apostrophe in it opens no
/// quote, so every path the commands after it name is still seen.
#[test]
fn an_apostrophe_in_a_shell_comment_hides_no_later_path() {
    let command = "# the tool's own manifest decides\ntest -f ../sibling/package.manifest\n";
    let paths = scan(command);
    assert_eq!(paths.len(), 1, "{paths:?}");
    assert_eq!(paths[0].path, PathBuf::from("../sibling/package.manifest"));
    assert!(paths[0].required, "{paths:?}");
}
