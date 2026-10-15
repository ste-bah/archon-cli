//! Issue 366: the one live-root rule the freeze and the author step share.
use super::*;
use serde_json::json;

fn forms() -> Vec<PathBuf> {
    root_forms([Path::new("/work/repo"), Path::new("/work/repo/project/")])
}

#[test]
fn forms_drop_trailing_separators_and_the_filesystem_root() {
    assert_eq!(
        forms(),
        [
            PathBuf::from("/work/repo"),
            PathBuf::from("/work/repo/project")
        ]
    );
    assert!(root_forms([Path::new("/"), Path::new("")]).is_empty());
    // A POSIX path has no prefix, so no verbatim spelling.
    assert_eq!(verbatim(Path::new("/work/repo")), None);
}

#[test]
fn a_root_a_path_under_it_or_a_root_before_shell_syntax_is_named() {
    let forms = forms();
    for text in [
        "test -f /work/repo",
        "test -f /work/repo/out.json",
        "cat \"/work/repo/a b\"",
        "cd '/work/repo' && true",
        "ls /work/repo/*.json",
        "ls /work/repo$SUFFIX",
    ] {
        assert_eq!(
            named_root(text, &forms),
            Some(&PathBuf::from("/work/repo")),
            "{text}"
        );
    }
    assert_eq!(
        named_root("ls /work/repo/project", &forms),
        Some(&PathBuf::from("/work/repo")),
        "the first form in order names the finding, as the freeze has"
    );
}

#[test]
fn a_longer_name_or_a_relative_path_is_not_a_root() {
    let forms = forms();
    for text in [
        "test -f /work/repo-other/x",
        "test -f /work/repo.bak",
        "test -f /work/repo2",
        "test -f repo/out.json",
        "test -f out/work/repoX",
        "",
    ] {
        assert_eq!(named_root(text, &forms), None, "{text}");
    }
    // One occurrence is a sibling, a later one is the root: still named.
    assert!(named_root("cp /work/repo-x/a /work/repo/b", &forms).is_some());
}

#[test]
fn the_executed_text_is_a_command_or_a_non_blank_floor_verifier() {
    let command = json!({"kind": "command", "command": "true", "cwd": "project_root"});
    assert_eq!(executed_check_text(&command), Some("true"));
    let floor =
        |verifier: &str| json!({"kind": "floor", "contract": {"typed_verifier_command": verifier}});
    assert_eq!(executed_check_text(&floor("run")), Some("run"));
    assert_eq!(executed_check_text(&floor("  ")), None);
    assert_eq!(
        executed_check_text(&json!({"kind": "floor", "contract": {}})),
        None
    );
    assert_eq!(executed_check_text(&json!("text")), None);
}

#[test]
fn the_finding_is_the_freeze_text() {
    assert_eq!(
        live_root_finding("AC-1", Path::new("/work/repo")),
        "check 'AC-1': it names the live root /work/repo by its absolute path, so no hermetic copy can keep it off the live tree and the host never runs it; name every path relative to the check's working directory"
    );
}

#[test]
fn every_form_the_old_substring_rule_refused_is_still_named_but_a_true_sibling() {
    let forms = forms();
    for text in [
        // printf and concatenation forms that build the root's path.
        "ls \"$(printf /work/repo%s /src)\"",
        "ls \"/work/repo\"/src",
        "ROOT=/work/repo ls \"$ROOT\"/src",
        "ls /work/repo@x /work/repo~ /work/repo+x",
        // A sibling's path that leads back under the root.
        "cd /work/repo-x/../repo/src",
        // The root's text spelled with redundant separators.
        "cat /work//repo/x",
        "cat /work/./repo/x",
        "cat /work/x/../repo/y",
    ] {
        assert_eq!(
            named_root(text, &forms),
            Some(&PathBuf::from("/work/repo")),
            "{text}"
        );
    }
}

#[test]
#[cfg(unix)]
fn adjacent_shell_quotes_and_literal_assignments_are_checked_as_joined_words() {
    let forms = forms();
    for (case, text) in [
        ("double-quote", "cat \"/work\"\"/repo\"/src"),
        ("single-quote", "cat '/work''/repo'/src"),
        (
            "mixed-and-assigned",
            "ROOT='/work'\"/repo\" cat \"$ROOT\"/src",
        ),
    ] {
        let before = legacy_tokenized_root(text, &forms);
        let after = named_root(text, &forms);
        println!("EVIDENCE #366 {case} BEFORE: {before:?} AFTER: {after:?}");
        assert!(
            before.is_none(),
            "legacy detector should expose the split-word gap: {text}"
        );
        assert!(after.is_some(), "{text}");
    }
}

#[cfg(unix)]
#[test]
fn joined_prose_is_not_a_path_word_and_unused_assignment_is_ignored() {
    let forms = forms();
    let prose = r#"printf '%s' "example: /work""/repo is unavailable""#;
    let before = named_root(prose, &forms);
    println!("EVIDENCE #366 joined-prose BEFORE: {before:?} AFTER: prose must not refuse");
    assert_eq!(before, None, "joined prose is not a path word");
    assert!(named_root("cat \"/work\"\"/repo\"", &forms).is_some());

    let unused = "ROOT=/work/repo; printf '%s' ready";
    let before = named_root(unused, &forms);
    println!("EVIDENCE #366 unused-assignment BEFORE: {before:?} AFTER: None");
    assert_eq!(
        before, None,
        "unused assignment must not refuse the command"
    );
    assert!(named_root("ROOT=/work/repo cat \"$ROOT\"", &forms).is_some());
}

#[cfg(unix)]
#[test]
fn only_shell_assignment_words_expand_live_root_variables() {
    let forms = forms();
    for (case, text, expected) in [
        (
            "assignment-looking printf argument",
            r#"printf '%s' "DOC=/work/repo" "$DOC""#,
            false,
        ),
        (
            "assignment followed by use",
            "R=/work/repo; cat \"$R/x\"",
            true,
        ),
        ("command-prefix assignment", "R=/work/repo cmd \"$R\"", true),
        ("export assignment", "export R=/work/repo; ls $R", true),
        ("readonly assignment", "readonly R=/work/repo; ls $R", true),
        ("local assignment", "local R=/work/repo; ls $R", true),
        ("unused assignment", "R=/work/repo", false),
    ] {
        let actual = named_root(text, &forms);
        println!(
            "EVIDENCE #366 assignment-scope {case} EXPECTED: {expected} ACTUAL: {}",
            actual.is_some()
        );
        assert_eq!(actual.is_some(), expected, "{case}: {text}");
    }
}

#[cfg(unix)]
#[test]
fn assignment_only_segments_carry_only_across_parent_shell_connectors() {
    let forms = forms();
    for (case, text, expected) in [
        (
            "pipeline",
            "ROOT=/work/repo | printf \"%s\" \"$ROOT\"",
            false,
        ),
        (
            "background",
            "ROOT=/work/repo & printf \"%s\" \"$ROOT\"",
            false,
        ),
        (
            "pipeline before assignment-only segment",
            "true | ROOT=/work/repo; cat \"$ROOT\"",
            false,
        ),
        (
            "background before assignment-only segment",
            "sleep 1 & ROOT=/work/repo; cat \"$ROOT\"",
            true,
        ),
        (
            "background after assignment-only segment",
            "ROOT=/work/repo & cat \"$ROOT\"",
            false,
        ),
        ("semicolon", "ROOT=/work/repo; cat \"$ROOT\"", true),
        ("and", "ROOT=/work/repo && cat \"$ROOT\"", true),
        // The right side of `||` may run, so this conservative scan carries
        // the assignment through the following list separator.
        (
            "conditional",
            "true || ROOT=/work/repo; cat \"$ROOT\"",
            true,
        ),
        ("newline", "ROOT=/work/repo\ncat \"$ROOT\"", true),
    ] {
        let actual = named_root(text, &forms).is_some();
        println!("EVIDENCE #366 assignment connector {case} EXPECTED: {expected} ACTUAL: {actual}");
        assert_eq!(actual, expected, "{case}: {text}");
    }
}

#[cfg(unix)]
fn legacy_tokenized_root<'a>(text: &str, forms: &'a [PathBuf]) -> Option<&'a PathBuf> {
    let text = Spelling::HOST.fold(text);
    let ends = |c: char| Spelling::HOST.ends_token(c);
    let paths: Vec<PathBuf> = text.split(ends).filter_map(lexical).collect();
    forms.iter().find(|form| {
        let root = Spelling::HOST.fold(&form.to_string_lossy());
        let form = Path::new(&root);
        let named_at = |at: usize| match text[at + root.len()..].chars().next() {
            Some(next) if continues_name(next) => {
                let token = text[at..].split(ends).next().unwrap_or_default();
                lexical(token).is_some_and(|path| path.starts_with(form))
            }
            _ => true,
        };
        !root.is_empty()
            && (text.match_indices(&root).any(|(at, _)| named_at(at))
                || paths.iter().any(|path| path.starts_with(form)))
    })
}

#[test]
fn a_relative_root_never_matches_a_command() {
    // A relative root would match nearly any text ("bar " holds "r "): it
    // has no form at all, and only absolute roots are matched.
    assert!(root_forms([Path::new("r"), Path::new("p"), Path::new("./p")]).is_empty());
    let forms = root_forms([Path::new("r"), Path::new("/work/repo")]);
    assert_eq!(forms, [PathBuf::from("/work/repo")]);
    assert_eq!(named_root("bar baz r p", &forms), None);
}

#[cfg(unix)]
#[test]
fn filesystem_identity_catches_symlink_aliases_and_paths_inside_the_root() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("live-root");
    std::fs::create_dir(&root).unwrap();
    let inside = root.join("child");
    std::fs::create_dir(&inside).unwrap();
    let alias = temp.path().join("alias");
    std::os::unix::fs::symlink(&root, &alias).unwrap();
    let forms = root_forms([root.as_path()]);

    assert!(named_root(&format!("cd {}", alias.display()), &forms).is_some());
    assert!(named_root(&format!("cat {}/file", inside.display()), &forms).is_some());
    let missing_through_alias = alias.join("not-created");
    assert!(named_root(&format!("cat {}", missing_through_alias.display()), &forms).is_some());
}

#[cfg(unix)]
#[test]
fn unrelated_paths_on_the_same_volume_do_not_name_the_root() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("live-root");
    std::fs::create_dir(&root).unwrap();
    let forms = root_forms([root.as_path()]);

    let outside_temp = temp.path().join("scratch").join("not-created");
    assert_eq!(
        named_root(&format!("cat {}", outside_temp.display()), &forms),
        None
    );

    let outside_file = temp.path().join("outside-file");
    std::fs::write(&outside_file, "outside the live root").unwrap();
    assert_eq!(
        named_root(&format!("cat {}", outside_file.display()), &forms),
        None
    );
}

#[test]
fn unresolved_paths_are_rebuilt_from_the_longest_resolvable_ancestor() {
    let path = Path::new("/short/live/missing/child");
    let root = Path::new("/canonical/live");
    let canonicalize = |ancestor: &Path| match ancestor {
        p if p == Path::new("/short/live") => Some(root.to_path_buf()),
        p if p == Path::new("/short") => Some(PathBuf::from("/canonical")),
        _ => None,
    };

    let rebuilt = reconstruct_from_longest_existing_ancestor(path, canonicalize);
    assert_eq!(
        rebuilt,
        Some(PathBuf::from("/canonical/live/missing/child"))
    );

    let sibling = Path::new("/short/live-other/child");
    let rebuilt_sibling = reconstruct_from_longest_existing_ancestor(sibling, |ancestor| {
        (ancestor == Path::new("/short")).then(|| PathBuf::from("/canonical"))
    });
    assert_eq!(
        rebuilt_sibling,
        Some(PathBuf::from("/canonical/live-other/child"))
    );
    assert!(!path_starts_with(rebuilt_sibling.as_deref().unwrap(), root));
    assert_eq!(
        reconstruct_from_longest_existing_ancestor(Path::new("/unknown/missing"), |_| None),
        None,
        "no resolvable ancestor leaves fail-closed handling to the volume rule"
    );
}

/// The three `repository.lock` texts no freeze may believe (Issue 366, N1):
/// one that is not JSON, one of another schema, and one whose recorded
/// root is gone. Each leaves the true repository root unknown.
pub(crate) const UNBELIEVABLE_LOCKS: [(&str, &str); 3] = [
    ("corrupt", "{not json"),
    (
        "another schema",
        r#"{"schema_version":99,"repository_root":"/","base_commit":"unborn","decomposition_run_id":"x","recorded_at":"x"}"#,
    ),
    (
        "a root that is gone",
        r#"{"schema_version":1,"repository_root":"/nonexistent/archon-366-gone","base_commit":"unborn","decomposition_run_id":"x","recorded_at":"x"}"#,
    ),
];

/// Issue 366 N1: only a missing lock is the legacy set (the project); a lock
/// that cannot be believed is an error naming it and its repair, for the
/// freeze probe, the author step and the host re-author alike.
#[test]
fn only_a_missing_lock_falls_back_to_the_project() {
    let project = tempfile::tempdir().unwrap();
    let tasks = project.path().join("tasks");
    std::fs::create_dir_all(&tasks).unwrap();
    assert_eq!(
        recorded_repository(project.path(), &tasks),
        Ok(project.path().to_path_buf())
    );
    let lock = archon_workflow::repository_record::repository_record_path(&tasks);
    for (case, text) in UNBELIEVABLE_LOCKS {
        std::fs::write(&lock, text).unwrap();
        for why in [
            recorded_repository(project.path(), &tasks).unwrap_err(),
            freeze_repository(project.path(), &tasks, None).unwrap_err(),
            task_set_roots(project.path(), &tasks).unwrap_err(),
        ] {
            assert!(
                why.contains(&lock.display().to_string()) && why.contains("restore"),
                "{case}: {why}"
            );
        }
    }
}

/// The Windows spelling, modelled on any host: a root's text is matched
/// without case, `/` reads as `\`, and `\`, `:` and `?` do not end a path.
#[test]
fn the_windows_spelling_matches_without_case_and_either_separator() {
    let forms = [
        PathBuf::from(r"\\?\C:\Users\Runner\Repo"),
        PathBuf::from(r"C:\Users\Runner\Repo"),
    ];
    let windows = |text: &str| named_root_in(Spelling::Windows, text, &forms);
    for (text, form) in [
        (r"test -f \\?\C:\Users\Runner\Repo\out.json", &forms[0]),
        (r"cd \\?\c:\users\runner\repo//. && true", &forms[0]),
        (r"test -f C:\Users\Runner\Repo\out.json", &forms[1]),
        ("test -f c:/users/RUNNER/repo/out.json", &forms[1]),
        (r#"cat "C:\Users\Runner\Repo"/x"#, &forms[1]),
        (r#"cat "C:\Users\Runner""\Repo"/x"#, &forms[1]),
        (r#"x=C:\Users\Runner\Repo cat "$x""#, &forms[1]),
    ] {
        assert_eq!(windows(text), Some(form), "{text}");
    }
    for text in [
        r"test -f C:\Users\Runner\Repo-other\x",
        r"test -f C:\Users\Runner\Repo.bak",
        r"test -f D:\Users\Runner\Repo\x",
        "test -f out/repo",
    ] {
        assert_eq!(windows(text), None, "{text}");
    }
    // The POSIX spelling keeps case and reads `\` as shell syntax.
    let posix = |text: &str| named_root_in(Spelling::Posix, text, &forms);
    assert_eq!(posix("test -f c:/users/runner/repo/x"), None);
    assert_eq!(
        Spelling::Windows.fold(r"C:/Users\X"),
        r"c:\users\x".to_string()
    );
    assert!(Spelling::Posix.ends_token('\\') && Spelling::Posix.ends_token(':'));
    assert!(!(['\\', ':', '?'].into_iter()).any(|c| Spelling::Windows.ends_token(c)));
}

/// Shell command words are text even when the host parses paths with Windows
/// rules: POSIX-rooted words still name their textual root.
#[test]
fn windows_comparison_matches_posix_rooted_shell_words_on_every_host() {
    let forms = [PathBuf::from("/work/repo")];
    let windows = |text: &str| named_root_in(Spelling::Windows, text, &forms);

    for text in ["ls /work/repo$SUFFIX", "ls \"$(printf /work/repo%s /src)\""] {
        assert_eq!(windows(text), Some(&forms[0]), "{text}");
    }
    assert_eq!(windows("ls /work/repo-other/x"), None);
}

/// On Windows a drive root, a verbatim drive root, a UNC share root and a
/// drive-relative path have no form; a root has its plain and its verbatim
/// spelling, and a path that leads back under it is named.
#[path = "workflow_acceptance_live_root_windows_tests.rs"]
mod windows_tests;
