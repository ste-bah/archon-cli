//! Issue-226: the run policy's external data allowlist, by its rules.
use super::*;

fn metadata(project: &Path, repository: &Path, listed: &[&Path]) -> serde_json::Value {
    serde_json::json!({"observer_snapshot": {"native_execution": {
        "policy": {"project": project, "repository": repository},
        "external_data_roots": listed,
    }}})
}

struct World {
    _temp: tempfile::TempDir,
    project: PathBuf,
    repository: PathBuf,
    allowed: PathBuf,
    outside: PathBuf,
}

fn world() -> World {
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path().canonicalize().unwrap();
    let dirs = ["project", "repository", "allowed", "outside"].map(|name| {
        let dir = base.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    });
    let [project, repository, allowed, outside] = dirs;
    World {
        _temp: temp,
        project,
        repository,
        allowed,
        outside,
    }
}

#[test]
fn entries_are_canonical_and_never_the_project_the_repository_or_an_ancestor() {
    let w = world();
    let link = w.outside.join("allowed-link");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&w.allowed, &link).unwrap();
    #[cfg(not(unix))]
    let link = w.allowed.clone();
    let parent = w.project.parent().unwrap();
    let missing = w.outside.join("missing");
    let in_repository = w.repository.join("data");
    let listed = [
        link.as_path(),
        w.project.as_path(),
        in_repository.as_path(),
        parent,
        missing.as_path(),
        Path::new("/"),
    ];
    std::fs::create_dir_all(w.repository.join("data")).unwrap();
    let roots = ExternalRoots::from_metadata(&metadata(&w.project, &w.repository, &listed));
    assert_eq!(roots.allowed(), std::slice::from_ref(&w.allowed));
    let none = ExternalRoots::from_metadata(&metadata(&w.project, &w.repository, &[]));
    assert!(none.is_empty());
    assert!(ExternalRoots::from_metadata(&serde_json::json!({})).is_empty());
}

#[test]
fn a_declared_path_is_admitted_only_under_an_allowlisted_directory() {
    let w = world();
    let roots = ExternalRoots::from_allowed([w.allowed.clone()]);
    let file = w.allowed.join("lake/new/bars.json");
    assert_eq!(
        roots.admit(&file),
        Ok(Admitted {
            tree: w.allowed.clone(),
            destination: file.clone()
        })
    );
    let outside = roots.admit(&w.outside.join("bars.json")).unwrap_err();
    assert!(outside.contains(EXTERNAL_ROOTS_KEY), "{outside}");
    assert!(
        outside.contains(&w.outside.display().to_string()),
        "{outside}"
    );
    let climbed = w.allowed.join("lake/../../outside/bars.json");
    let why = roots.admit(&climbed).unwrap_err();
    assert!(
        why.contains("`..`") && why.contains(EXTERNAL_ROOTS_KEY),
        "{why}"
    );
    assert!(
        roots.admit(&w.allowed).is_err(),
        "never the directory itself"
    );
    assert!(roots.admit(Path::new("lake/bars.json")).is_err());
    let empty = ExternalRoots::default().admit(&file).unwrap_err();
    assert!(empty.contains(EXTERNAL_ROOTS_KEY), "{empty}");
}

#[cfg(unix)]
#[test]
fn a_link_out_of_an_allowlisted_directory_is_refused() {
    let w = world();
    let roots = ExternalRoots::from_allowed([w.allowed.clone()]);
    std::os::unix::fs::symlink(&w.outside, w.allowed.join("vault")).unwrap();
    let why = roots.admit(&w.allowed.join("vault/keys.json")).unwrap_err();
    assert!(why.contains(EXTERNAL_ROOTS_KEY), "{why}");
    // A dangling link below it is never written through either.
    std::os::unix::fs::symlink(w.outside.join("gone"), w.allowed.join("dangling")).unwrap();
    let why = roots
        .admit(&w.allowed.join("dangling/keys.json"))
        .unwrap_err();
    assert!(why.contains("symlink"), "{why}");
}

#[test]
fn created_directories_are_listed_and_removed_deepest_first() {
    let w = world();
    let file = w.allowed.join("a/b/c.json");
    let missing = missing_dirs(&w.allowed, &file);
    assert_eq!(missing, vec![w.allowed.join("a"), w.allowed.join("a/b")]);
    std::fs::create_dir_all(w.allowed.join("a/b")).unwrap();
    std::fs::write(w.allowed.join("a/b/other.json"), "kept").unwrap();
    remove_created(&missing).unwrap();
    assert!(
        w.allowed.join("a/b/other.json").exists(),
        "never what it holds"
    );
    std::fs::remove_file(w.allowed.join("a/b/other.json")).unwrap();
    remove_created(&missing).unwrap();
    assert!(!w.allowed.join("a").exists());
    assert!(w.allowed.exists());
    assert_eq!(
        stored_rel("/x/y.json"),
        PathBuf::from(".external").join("x").join("y.json")
    );
    assert_eq!(stored("data/y.json"), PathBuf::from("data/y.json"));
}
