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
