//! #241: no spawned agent writes the records a resume trusts.

use super::*;

fn real_temp() -> (tempfile::TempDir, std::path::PathBuf) {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = std::fs::canonicalize(temp.path()).expect("real temp");
    (temp, root)
}

#[test]
fn a_write_under_the_records_is_refused_and_one_beside_them_is_not() {
    let (_t, home) = real_temp();
    let root = home.join(".archon").join("sessions");
    let meta = root.join("s1").join("subagents").join("agent-a.meta.json");
    let refusal = refuse_under(&meta, &meta, &root).expect_err("a record is refused");
    assert!(refusal.contains("agent records"), "{refusal}");
    assert!(refusal.contains(&meta.display().to_string()), "{refusal}");

    let beside = home.join(".archon").join("notes.md");
    assert!(refuse_under(&beside, &beside, &root).is_ok());
    let prefix = home.join(".archon").join("sessions-other").join("x");
    assert!(refuse_under(&prefix, &prefix, &root).is_ok());
}

#[cfg(unix)]
#[test]
fn a_link_into_the_records_is_refused() {
    let (_t, home) = real_temp();
    let root = home.join(".archon").join("sessions");
    std::fs::create_dir_all(root.join("s1")).unwrap();
    let workspace = home.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let link = workspace.join("records");
    std::os::unix::fs::symlink(root.join("s1"), &link).unwrap();
    let named = link.join("agent-a.meta.json");
    assert!(refuse_under(&named, &named, &root).is_err());
}

#[test]
fn the_top_level_agent_is_not_judged() {
    let ctx = ToolContext::default();
    let path = Path::new("/anywhere/at/all");
    assert!(ensure_not_agent_record(path, path, &ctx).is_ok());
}
