//! Issue-120: a write branch is shown exactly the scope its landing keeps.
//!
//! Live on wf-0ddadd81 a residual round's prompt listed eleven target
//! files, its tool guard refused an Edit of an undeclared file as "would be
//! lost", and the landing then declared fifteen: the ownership grant kept
//! four undeclared files inside the scope roots that no other item of the
//! single-item wave claimed, written by a shell heredoc the guard could not
//! see. This drives the production write wave with real Git worktrees and
//! checks the prompt the branch is dispatched with names the rule and
//! carries the grant's own roots and the OTHER items' claims for the guard,
//! and that the landing keeps exactly what that stamp admits: the unclaimed
//! in-root file lands, and a file the sibling claims refuses the branch.
#[path = "support/write_wave_fixture.rs"]
mod support;

use archon_workflow::*;
use serde_json::{Value, json};
use support::{Edits, Fixture, git};

const OWN: &str = "crates/a/src/lib.rs";
const SIBLING: &str = "crates/a/src/other.rs";
const UNCLAIMED: &str = "crates/a/src/extra.rs";

fn with_crate(f: &Fixture) {
    for (path, content) in [
        ("crates/a/Cargo.toml", "[package]\nname = \"a\"\n"),
        (OWN, "// a\n"),
        (SIBLING, "// other\n"),
        (UNCLAIMED, "// extra\n"),
    ] {
        let target = f.repo.join(path);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, content).unwrap();
    }
    git(&f.repo, &["add", "."]);
    git(&f.repo, &["commit", "-qm", "crate"]);
}

fn edits(files: Vec<(&'static str, &'static str)>) -> Edits {
    Edits {
        report: files.iter().map(|(path, _)| *path).collect(),
        files,
        via_adapter: false,
    }
}

/// The `_grantable_scope` object the dispatched prompt's input carries.
fn stamp_in(prompt: &str) -> Value {
    let key = "\"_grantable_scope\":";
    let at = prompt
        .find(key)
        .unwrap_or_else(|| panic!("no stamp: {prompt}"))
        + key.len();
    let mut stream = serde_json::Deserializer::from_str(&prompt[at..]).into_iter::<Value>();
    stream.next().unwrap().unwrap()
}

#[tokio::test]
async fn the_branch_is_told_and_stamped_the_scope_its_landing_keeps() {
    let f = Fixture::new();
    with_crate(&f);
    let (_, prompts) = f
        .wave_audited(
            "scope",
            vec![
                (
                    vec![OWN],
                    edits(vec![(OWN, "// a fixed\n"), (UNCLAIMED, "// extra fixed\n")]),
                ),
                (vec![SIBLING], edits(vec![(SIBLING, "// other fixed\n")])),
            ],
            None,
        )
        .await;
    let own = prompts
        .iter()
        .find(|prompt| prompt.contains("call_id: scope-0"))
        .unwrap_or_else(|| panic!("{prompts:#?}"));
    assert!(
        own.contains("Your declared scope is target_files plus every file, new or existing"),
        "{own}"
    );
    assert!(
        own.contains(
            "that no other item of this wave declares is granted to this branch at landing"
        ),
        "{own}"
    );
    // The sibling's claim is its declared file and that module's own
    // directory scope, exactly as the grant's wave claims hold them.
    assert_eq!(
        stamp_in(own),
        json!({"claimed": ["crates/a/src/other", SIBLING], "scope_roots": ["crates/a/"]})
    );
    // The landing keeps what the stamp admits: the unclaimed in-root file
    // was granted and landed under branch 0.
    assert_eq!(
        git(&f.repo, &["show", &format!("HEAD:{UNCLAIMED}")]),
        "// extra fixed"
    );
    let result = f.branch_result("scope", "scope-0");
    assert_eq!(
        result.data["scope_granted"],
        json!([UNCLAIMED]),
        "{result:#?}"
    );
}

#[tokio::test]
async fn a_file_the_stamp_names_as_another_items_claim_refuses_the_branch_at_landing() {
    let f = Fixture::new();
    with_crate(&f);
    let (_, prompts) = f
        .wave_audited(
            "scope",
            vec![
                (
                    vec![OWN],
                    edits(vec![(OWN, "// a fixed\n"), (SIBLING, "// other, by 0\n")]),
                ),
                (vec![SIBLING], edits(vec![(SIBLING, "// other fixed\n")])),
            ],
            None,
        )
        .await;
    // The branch was stamped the claim the landing then refused it over.
    let own = prompts
        .iter()
        .find(|prompt| prompt.contains("call_id: scope-0"))
        .unwrap_or_else(|| panic!("{prompts:#?}"));
    assert!(
        stamp_in(own)["claimed"]
            .as_array()
            .unwrap()
            .contains(&json!(SIBLING)),
        "{own}"
    );
    let result = f.branch_result("scope", "scope-0");
    assert_ne!(result.status, WorkflowV2Status::Accepted, "{result:#?}");
    assert_eq!(git(&f.repo, &["show", &format!("HEAD:{OWN}")]), "// a");
}
