//! The critic-logic list cannot rot (#434c, #434d): the listed files hash to
//! the pinned digest, so no edit to them lands without a decision about
//! `CRITIC_LOGIC_VERSION`; every module a listed file loads is listed; every
//! fidelity file beside them is listed or excluded with a reason; and the
//! frozen earlier-schema digest is what its comment says it is.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::super::legacy::LEGACY_CRITIC_LOGIC;
use super::super::logic_digest::{
    CRITIC_LOGIC_VERSION, EXCLUDED, FILES, SOURCES_DIGEST, digest, digest_of, pin_check,
};
use crate::command::workflow_host_command_logic_items::{child_file, facts, is_test_file};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn relative(root: &Path, file: &Path) -> String {
    file.strip_prefix(root)
        .unwrap_or(file)
        .to_string_lossy()
        .replace('\\', "/")
}

/// The non-test modules and spliced Rust sources `files` load that are not
/// among `files`.
fn unlisted_children(root: &Path, files: &[&str]) -> BTreeSet<String> {
    let listed: BTreeSet<&str> = files.iter().copied().collect();
    let mut missing = BTreeSet::new();
    for file in files {
        let path = root.join(file);
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{file}: {e}"));
        let facts = facts(&text);
        let mut children: Vec<PathBuf> = facts
            .mods
            .iter()
            .map(|(scope, name, attr)| child_file(&path, scope, name, attr.as_deref()))
            .collect();
        for (rust, manifest_relative, target) in &facts.includes {
            assert!(
                !(*rust && *manifest_relative),
                "{file}: resolve the manifest-relative include {target} by hand"
            );
            if *rust {
                children.push(path.parent().unwrap().join(target));
            }
        }
        for child in children {
            let child = relative(root, &child);
            if !is_test_file(Path::new(&child)) && !listed.contains(child.as_str()) {
                missing.insert(child);
            }
        }
    }
    missing
}

fn read_source(file: &str) -> Vec<u8> {
    std::fs::read(root().join(file)).unwrap_or_else(|error| panic!("{file}: {error}"))
}

/// The guard: an edit to a listed file is reviewed against the version.
#[test]
fn fidelity_critic_sources_match_their_pinned_digest() {
    if let Err(stale) = pin_check(SOURCES_DIGEST, read_source) {
        panic!("{stale}");
    }
}

/// The guard can go red: one byte changed in one listed file, and no re-pin,
/// fails it with the decision to make; re-pinned, it passes, and the
/// version — what the store keys on — has not moved.
#[test]
fn fidelity_a_source_edit_without_a_re_pin_trips_the_guard() {
    let edited = |file: &str| {
        let mut bytes = read_source(file);
        if file == FILES[0] {
            bytes.extend_from_slice(b"\n// an edit\n");
        }
        bytes
    };
    let stale = pin_check(SOURCES_DIGEST, edited).expect_err("an unreviewed edit");
    assert!(
        stale.contains("CRITIC_LOGIC_VERSION") && stale.contains("SOURCES_DIGEST"),
        "{stale}"
    );
    let re_pinned = digest(edited);
    assert_ne!(re_pinned, SOURCES_DIGEST);
    assert_eq!(pin_check(&re_pinned, edited), Ok(()));
    assert_eq!(
        CRITIC_LOGIC_VERSION, 1,
        "#434d keeps the meaning of every verdict"
    );
}

#[test]
fn every_module_a_listed_file_loads_is_listed() {
    let mut sorted = FILES.to_vec();
    sorted.sort_unstable();
    assert_eq!(sorted, FILES, "FILES is sorted");
    assert_eq!(
        unlisted_children(&root(), FILES),
        BTreeSet::new(),
        "add each to FILES in fidelity_logic_digest.rs"
    );
}

/// The guard above can go red: a `#[path]` child and a `mod` child outside
/// the list are both reported; test modules are not.
#[test]
fn an_unlisted_child_module_is_reported() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("src");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("critic.rs"),
        "#[path = \"critic_reply.rs\"]\nmod reply;\nmod parse;\n#[cfg(test)]\nmod tests;\n",
    )
    .unwrap();
    std::fs::create_dir_all(dir.join("critic")).unwrap();
    for child in ["critic_reply.rs", "critic/parse.rs", "tests.rs"] {
        std::fs::write(dir.join(child), "").unwrap();
    }
    let missing = unlisted_children(temp.path(), &["src/critic.rs"]);
    assert_eq!(
        missing,
        BTreeSet::from(["src/critic/parse.rs".into(), "src/critic_reply.rs".into()])
    );
    let missing = unlisted_children(temp.path(), &["src/critic.rs", "src/critic_reply.rs"]);
    assert_eq!(missing, BTreeSet::from(["src/critic/parse.rs".into()]));
}

/// A new fidelity file must be hashed or excluded with a reason.
#[test]
fn every_fidelity_file_is_listed_or_excluded_with_a_reason() {
    let root = root();
    let mut seen = BTreeSet::new();
    for (dir, prefix) in [
        ("src/command/topology_lint", "fidelity"),
        ("crates/archon-workflow/src", "fidelity_audit"),
    ] {
        for entry in std::fs::read_dir(root.join(dir)).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            if name.starts_with(prefix) && name.ends_with(".rs") && !is_test_file(&path) {
                seen.insert(relative(&root, &path));
            }
        }
    }
    let excluded: BTreeSet<String> = EXCLUDED.iter().map(|(file, _)| file.to_string()).collect();
    for (file, reason) in EXCLUDED {
        assert!(!reason.trim().is_empty(), "{file} needs a reason");
        assert!(!FILES.contains(file), "{file} is both hashed and excluded");
        assert!(root.join(file).is_file(), "stale exclusion {file}");
    }
    let unaccounted: Vec<&String> = seen
        .iter()
        .filter(|file| !FILES.contains(&file.as_str()) && !excluded.contains(*file))
        .collect();
    assert!(unaccounted.is_empty(), "hash or exclude: {unaccounted:?}");
}

/// The files `LEGACY_CRITIC_LOGIC` was computed over, as listed when it was
/// frozen; the list in `fidelity_logic_digest.rs` may grow, this one never.
const LEGACY_FILES: &[&str] = &[
    "crates/archon-workflow/src/fidelity_audit.rs",
    "crates/archon-workflow/src/fidelity_audit_check.rs",
    "crates/archon-workflow/src/fidelity_audit_reply.rs",
    "src/command/topology_lint/fidelity_critic.rs",
];
/// The commit that introduced schema 4.
const SCHEMA_FOUR_COMMIT: &str = "bc8cf2693";
/// The last commit that wrote schema 5 (#434d moved to schema 6).
const LAST_SCHEMA_FIVE_COMMIT: &str = "22e024436";

/// `LEGACY_FILES` as they were at `commit`.
fn digest_at(commit: &str) -> String {
    let root = root();
    digest_of(LEGACY_FILES, |file| {
        let shown = std::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["show", &format!("{commit}:{file}")])
            .output()
            .expect("git");
        assert!(shown.status.success(), "git show {file} failed");
        shown.stdout
    })
}

/// Reads git history, which a shallow clone lacks; run it with `--ignored`
/// in a full clone.
#[test]
#[ignore = "needs git history back to bc8cf2693; run with --ignored in a full clone"]
fn legacy_critic_logic_is_the_schema_four_sources_digest() {
    assert_eq!(digest_at(SCHEMA_FOUR_COMMIT), LEGACY_CRITIC_LOGIC);
}

/// The digest schema-5 records written by 22e024436 carry.
#[test]
#[ignore = "needs git history back to 22e024436; run with --ignored in a full clone"]
fn legacy_critic_logic_is_the_last_schema_five_sources_digest() {
    assert_eq!(digest_at(LAST_SCHEMA_FIVE_COMMIT), LEGACY_CRITIC_LOGIC);
}
