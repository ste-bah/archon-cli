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
    let changed = archon_shell::spawn::command("git")
        .current_dir(root)
        .args(["diff", "--name-only", "332c35a52"])
        .output()
        .unwrap();
    assert!(changed.status.success());
    let added = archon_shell::spawn::command("git")
        .current_dir(root)
        .args(["ls-files", "--others", "--exclude-standard"])
        .output()
        .unwrap();
    assert!(added.status.success());
    let paths = format!(
        "{}\n{}",
        String::from_utf8(changed.stdout).unwrap(),
        String::from_utf8(added.stdout).unwrap()
    );
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
