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
        "x=/work/repo;y",
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
        "ROOT=/work/repo; ls \"$ROOT\"/src",
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
        (r"x=C:\Users\Runner\Repo;y", &forms[1]),
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

/// On Windows a drive root, a verbatim drive root, a UNC share root and a
/// drive-relative path have no form; a root has its plain and its verbatim
/// spelling, and a path that leads back under it is named.
#[cfg(windows)]
#[test]
fn windows_forms_drop_every_filesystem_root_and_keep_both_spellings() {
    // No share is resolved: a UNC root is judged by its components alone.
    for root in [
        r"C:\",
        r"\\?\C:\",
        r"\\server\share\",
        r"\\server\share",
        "D:",
        r"\\?\D:",
    ] {
        assert!(!below_a_filesystem_root(Path::new(root)), "{root}");
    }
    assert_eq!(
        root_forms([r"C:\", r"\\?\C:\", "D:", "/"].map(Path::new)),
        Vec::<PathBuf>::new()
    );
    assert_eq!(
        verbatim(Path::new(r"\\server\share\x")),
        Some(PathBuf::from(r"\\?\UNC\server\share\x"))
    );
    let forms = root_forms([Path::new(r"C:\Work\Repo\")]);
    for form in [r"\\?\C:\Work\Repo", r"C:\Work\Repo"] {
        assert!(
            forms.iter().any(|f| f.as_os_str() == form),
            "{form}: {forms:?}"
        );
    }
    assert!(named_root(r"cd C:\Work\Repo-x\..\repo\src", &forms).is_some());
    assert!(named_root(r"cat C:\Work\\Repo\x", &forms).is_some());
    let live = tempfile::tempdir().unwrap();
    let canonical = live.path().canonicalize().unwrap();
    let forms = root_forms([live.path()]);
    for form in [canonical.clone(), archon_shell::paths::plain(canonical)] {
        assert!(forms.contains(&form), "{form:?}: {forms:?}");
    }
}

#[cfg(windows)]
#[test]
fn windows_short_name_alias_is_matched_by_file_identity() {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetShortPathNameW(input: *const u16, output: *mut u16, capacity: u32) -> u32;
    }

    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("Long Directory Name").join("Repository");
    std::fs::create_dir_all(&root).unwrap();
    let input: Vec<u16> = root.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut output = vec![0u16; 32_768];
    // SAFETY: both buffers are valid for the duration of the call, and the
    // output buffer's capacity is supplied to the Win32 API.
    let length =
        unsafe { GetShortPathNameW(input.as_ptr(), output.as_mut_ptr(), output.len() as u32) };
    if length == 0 || length as usize >= output.len() {
        println!("skipping 8.3 alias assertion: GetShortPathNameW is unavailable");
        return;
    }
    output.truncate(length as usize);
    let short = PathBuf::from(OsString::from_wide(&output));
    if short == root {
        println!("skipping 8.3 alias assertion: short names are disabled for this volume");
        return;
    }
    let forms = root_forms([root.as_path()]);
    assert!(named_root(&format!("type {}", short.display()), &forms).is_some());
}
