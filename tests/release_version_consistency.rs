use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

const RELEASE_VERSION: &str = "1.9.2";

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn read(relative: &str) -> String {
    fs::read_to_string(root().join(relative))
        .unwrap_or_else(|error| panic!("failed to read {relative}: {error}"))
}

fn manifest_version(contents: &str) -> Option<String> {
    let parsed: toml::Value = toml::from_str(contents).expect("valid TOML");
    parsed
        .get("workspace")?
        .get("package")?
        .get("version")?
        .as_str()
        .map(ToOwned::to_owned)
}

fn workspace_package_names() -> BTreeSet<String> {
    let root_manifest: toml::Value = toml::from_str(&read("Cargo.toml")).expect("valid TOML");
    let members = root_manifest["workspace"]["members"]
        .as_array()
        .expect("workspace members");
    let mut names = BTreeSet::from(["archon-cli-workspace".to_string()]);

    for member in members {
        let pattern = member.as_str().expect("member pattern");
        let base = pattern
            .strip_suffix("/*")
            .expect("directory wildcard member");
        let entries = fs::read_dir(root().join(base)).expect("workspace member directory");
        for entry in entries {
            let manifest_path = entry.expect("member path").path().join("Cargo.toml");
            if !manifest_path.is_file() {
                continue;
            }
            let manifest = fs::read_to_string(manifest_path).expect("member manifest");
            let parsed: toml::Value = toml::from_str(&manifest).expect("valid member TOML");
            names.insert(
                parsed["package"]["name"]
                    .as_str()
                    .expect("member package name")
                    .to_string(),
            );
        }
    }
    names
}

fn local_lock_versions(contents: &str) -> Vec<(String, String)> {
    contents
        .split("[[package]]")
        .skip(1)
        .filter_map(|block| {
            if block.lines().any(|line| line.starts_with("source = ")) {
                return None;
            }
            let parsed: toml::Value = toml::from_str(block).ok()?;
            let name = parsed.get("name")?.as_str()?;
            let version = parsed.get("version")?.as_str()?;
            Some((name.to_string(), version.to_string()))
        })
        .collect()
}

#[test]
fn v1_9_2_release_surfaces_are_synchronized() {
    assert_eq!(
        manifest_version(&read("Cargo.toml")).as_deref(),
        Some(RELEASE_VERSION)
    );

    let web: serde_json::Value =
        serde_json::from_str(&read("web/package.json")).expect("valid web package JSON");
    assert_eq!(web["version"].as_str(), Some(RELEASE_VERSION));

    let web_lock: serde_json::Value =
        serde_json::from_str(&read("web/package-lock.json")).expect("valid web lock JSON");
    assert_eq!(web_lock["version"].as_str(), Some(RELEASE_VERSION));
    assert_eq!(
        web_lock["packages"][""]["version"].as_str(),
        Some(RELEASE_VERSION)
    );

    let local_versions = local_lock_versions(&read("Cargo.lock"));
    let local_names: BTreeSet<_> = local_versions
        .iter()
        .map(|(name, _)| name.to_string())
        .collect();
    assert_eq!(local_names, workspace_package_names());
    assert!(
        local_versions
            .iter()
            .all(|(_, version)| version == RELEASE_VERSION),
        "workspace lock versions differ: {local_versions:?}"
    );

    assert!(read("README.md").contains("Current release: v1.9.2"));
    assert!(read("docs/getting-started/installation.md").contains("archon 1.9.2 (<short-sha>)"));
    assert!(read("docs/README.md").contains("[v1.9.2](release-notes/v1.9.2.md) — Patch:"));
    assert!(read("docs/release-notes/v1.9.2.md").starts_with("# v1.9.2\n"));

    for snapshot in [
        "crates/archon-tui/tests/snapshots/tui_snapshots__splash_empty_activity.snap",
        "crates/archon-tui/tests/snapshots/tui_snapshots__splash_with_activity.snap",
    ] {
        assert!(read(snapshot).contains("Archon v1.9.2"), "{snapshot}");
    }
}

#[test]
fn historical_v1_9_1_release_surfaces_remain_available() {
    assert!(root().join("docs/release-notes/v1.9.1.md").is_file());
    assert!(read("docs/README.md").contains("[v1.9.1](release-notes/v1.9.1.md)"));
    assert!(read("README.md").contains("docs/release-notes/v1.9.1.md"));
}
