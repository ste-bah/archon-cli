//! Cover the entire fix, including test fixtures, with one source boundary.
fn direct_constructor(source: &str) -> bool {
    let compact: String = source.chars().filter(|c| !c.is_whitespace()).collect();
    ["std", "tokio"]
        .iter()
        .any(|root| compact.contains(&format!("{root}::process::{}::{}(", "Command", "new")))
}
#[test]
fn entire_fix_uses_shared_spawn_boundary() {
    for (root, spacing) in [("std", ""), ("tokio", ""), ("std", " \n ")] {
        assert!(direct_constructor(&format!(
            "{root}::{spacing}process::Command::{spacing}new(\"child\")"
        )));
    }

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    // The fix's own files, fixed at landing: a CI checkout is shallow and has
    // none of the fix's commits, and its merge brings other work whose files
    // are covered by their own spawn lints, not by this one.
    let paths = include_str!("workflow_host_spawn_r3_files.txt");
    assert!(paths.lines().count() > 50, "the fix's file list is present");
    let violations: Vec<_> = paths
        .lines()
        .filter(|p| !p.is_empty() && root.join(p).is_file())
        .filter(|p| {
            direct_constructor(&String::from_utf8_lossy(
                &std::fs::read(root.join(p)).unwrap(),
            ))
        })
        .collect();
    assert!(
        violations.is_empty(),
        "direct child constructors in fix: {violations:?}"
    );
}
