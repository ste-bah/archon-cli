use super::*;

fn finding(command: &str) -> Option<String> {
    let tree = tempfile::tempdir().unwrap();
    let project = tree.path().join("project");
    let repo = tree.path().join("repo");
    std::fs::create_dir(&project).unwrap();
    std::fs::create_dir(&repo).unwrap();
    let contract: AcceptanceContract = serde_json::from_value(serde_json::json!({
        "schema_version":1, "prd":{"path":"p.md","digest":"d"},
        "gap_policy":{"permitted_acceptance_ids":[],"forbidden_phrases":[],"required_fields":[]},
        "acceptance":[{"id":"check", "criterion":"checks the deliverable", "check":{"kind":"command","cwd":"project_root","command":command}, "judgment":{"verdict":"accepted","counterexample":"","reason":"","host_call_id":"test"}}],
        "supplementary":[]
    })).unwrap();
    super::super::super::siblings::missing_for_checks(
        &project,
        &repo,
        &contract,
        &BTreeSet::from(["check".into()]),
    )
    .remove("check")
}
#[test]
#[allow(non_snake_case)]
fn B1_cd_existing_sibling_allows_new_file() {
    // Input: cd into existing repo, absent deliverable -> no setup finding.
    assert!(finding("cd ../repo && test -f src/new.rs").is_none());
}
#[test]
#[allow(non_snake_case)]
fn B1_direct_existing_sibling_allows_new_file() {
    // Input: direct absent deliverable under existing repo -> no setup finding.
    assert!(finding("test -f ../repo/src/new.rs").is_none());
}
#[test]
#[allow(non_snake_case)]
fn B1_cd_absent_sibling_names_root() {
    // Input: cd into absent root -> finding names only that sibling root.
    assert_eq!(
        finding("cd ../absent && test -f x").unwrap(),
        "check check names sibling directory ../absent, which does not exist next to the working directory; name an existing directory"
    );
}
#[test]
#[allow(non_snake_case)]
fn B1_direct_absent_sibling_names_root() {
    // Input: file under absent root -> finding names only that sibling root.
    assert!(
        finding("test -f ../absent/x")
            .unwrap()
            .contains("sibling directory ../absent,")
    );
}
#[test]
#[allow(non_snake_case)]
fn B1_absent_subdirectory_inside_existing_sibling_is_allowed() {
    // Input: absent subdirectory the work may add -> no setup finding.
    assert!(finding("cd ../repo && cd sub && test -f y").is_none());
}

#[test]
#[allow(non_snake_case)]
fn S3_probe_uses_existing_site_scoping_for_required_reads() {
    // Input: data strings versus proven program and shell file reads.
    // Expected: exactly the existing site scoper's reads yield a missing-root finding.
    for (command, refused) in [
        ("echo ../missing/item", false),
        ("grep ../missing/item input", false),
        ("python3 -c 'print(\"../missing/x\")'", false),
        ("python3 -c 'open(\"../missing/x\")'", true),
        (
            "node -e 'require(\"fs\").readFileSync(\"../missing/x\")'",
            false,
        ),
        ("perl -e 'open F, \"../missing/x\"'", false),
        ("node -e 'fs.readFileSync(\"../missing/x\")'", true),
        ("echo --manifest-path ../missing/Cargo.toml", false),
        ("cargo run -- --manifest-path ../missing/Cargo.toml", false),
        (
            "cd $place; cargo metadata --manifest-path ../missing/Cargo.toml",
            false,
        ),
        ("sh -c 'cat ../missing/x'", true),
        ("cat <<'EOF'\nopen('../missing/x')\nEOF", false),
        ("sh <<'EOF'\ncat ../missing/x\nEOF", true),
        ("test -e ../missing/item", true),
        ("runner - <<'EOF'\nopen('../missing/x')\nEOF", true),
        ("cargo metadata --manifest-path ../missing/Cargo.toml", true),
        (
            "test -f input || cargo metadata --manifest-path ../missing/Cargo.toml",
            true,
        ),
        ("cat ../missing/item", true),
        ("cat < ../missing/item", true),
        ("cd ../missing && test -f x", true),
    ] {
        assert_eq!(finding(command).is_some(), refused, "{command}");
    }
}

#[test]
#[allow(non_snake_case)]
fn N3_or_exit_missing_cd_names_root_at_probe() {
    // Input: missing cd followed by || exit -> B1 root finding before running.
    assert_eq!(
        finding("cd ../absent || exit 1; test -f f").unwrap(),
        "check check names sibling directory ../absent, which does not exist next to the working directory; name an existing directory"
    );
}
