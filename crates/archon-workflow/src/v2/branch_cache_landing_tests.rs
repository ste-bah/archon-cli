//! Issue-108: a landing stands on the tree the run's own later landings
//! left, in the order the host committed them, and on nothing else.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::*;

const RUN: &str = "run-1";
const FILE: &str = "src/lib.rs";

struct Repo {
    _temp: tempfile::TempDir,
    root: PathBuf,
}

fn git(root: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn hash(text: &str) -> String {
    blake3::hash(text.as_bytes()).to_hex().to_string()
}

fn repo() -> Repo {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("repo");
    std::fs::create_dir_all(root.join("src")).unwrap();
    git(&root, &["init", "-q"]);
    std::fs::write(root.join(FILE), "x").unwrap();
    std::fs::write(root.join(".gitignore"), "ignored/\n").unwrap();
    commit(&root, "someone", "baseline", &[FILE, ".gitignore"]);
    Repo { _temp: temp, root }
}

fn commit(root: &Path, author: &str, message: &str, paths: &[&str]) {
    let mut add = vec!["add", "-A", "--"];
    add.extend(paths);
    git(root, &add);
    git(
        root,
        &[
            "-c",
            &format!("user.name={author}"),
            "-c",
            "user.email=a@b",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            message,
        ],
    );
}

impl Repo {
    fn write(&self, text: Option<&str>) {
        let path = self.root.join(FILE);
        match text {
            Some(text) => std::fs::write(path, text).unwrap(),
            None => {
                let _ = std::fs::remove_file(path);
            }
        }
    }

    /// The host lands `stage`: FILE from `pre` to `post` (None: no file),
    /// committed exactly as `wave_commit` commits it.
    fn land(&self, stage: &str, pre: Option<&str>, post: Option<&str>) -> PatchManifest {
        self.land_in(RUN, stage, pre, post)
    }

    fn land_in(
        &self,
        run: &str,
        stage: &str,
        pre: Option<&str>,
        post: Option<&str>,
    ) -> PatchManifest {
        self.write(post);
        commit(
            &self.root,
            LANDING_AUTHOR,
            &format!("archon: wave 0 outputs (run {run}, stage {stage})"),
            &[FILE],
        );
        manifest(run, stage, pre, post)
    }

    fn holds(&self, manifest: &PatchManifest) -> Result<(), String> {
        landing_holds(&self.root, manifest)
    }
}

fn manifest(run: &str, stage: &str, pre: Option<&str>, post: Option<&str>) -> PatchManifest {
    let state = |text: Option<&str>, none: &str| text.map_or(none.to_string(), hash);
    let only = |yes: bool| if yes { vec![FILE.to_string()] } else { vec![] };
    PatchManifest {
        schema: "test".into(),
        run_id: run.into(),
        stage_id: stage.into(),
        item_id: format!("{stage}-0"),
        baseline_commit: "base".into(),
        patch_path: PathBuf::from("p.patch"),
        declared_target_files: vec![FILE.into()],
        changed_files: only(pre.is_some() && post.is_some()),
        created_files: only(pre.is_none()),
        deleted_files: only(post.is_none()),
        pre_hashes: BTreeMap::from([(FILE.to_string(), state(pre, "absent"))]),
        post_hashes: BTreeMap::from([(FILE.to_string(), state(post, "deleted"))]),
        verify_command: None,
        agent_artifact_path: None,
        status: ManifestStatus::Applied,
        skipped_ignored: BTreeMap::new(),
        materialized: Default::default(),
        materializable: Default::default(),
    }
}

#[test]
fn an_untouched_landing_stands() {
    let r = repo();
    let one = r.land("fix-1", Some("x"), Some("a"));
    assert_eq!(r.holds(&one), Ok(()));
}

#[test]
fn a_later_landing_of_the_run_over_the_same_file_leaves_both_standing() {
    let r = repo();
    let one = r.land("fix-1", Some("x"), Some("a"));
    let two = r.land("fix-2", Some("a"), Some("b"));
    assert_eq!(r.holds(&one), Ok(()));
    assert_eq!(r.holds(&two), Ok(()));
}

/// R takes the file A to B and a later landing L reverts it to A: order is
/// the commits', so A is the tree the run left and B is not.
#[test]
fn a_reverting_later_landing_decides_by_commit_order_not_content() {
    let r = repo();
    r.land("setup", Some("x"), Some("A"));
    let one = r.land("fix-1", Some("A"), Some("B"));
    let two = r.land("fix-2", Some("B"), Some("A"));
    assert_eq!(r.holds(&one), Ok(()), "the correct tree");
    assert_eq!(r.holds(&two), Ok(()));
    // R re-applied outside the run: the tree is B again.
    r.write(Some("B"));
    assert!(r.holds(&one).is_err(), "the wrong tree");
    assert!(r.holds(&two).is_err(), "L must not stand on B");
    commit(&r.root, "operator", "re-apply fix-1 by hand", &[FILE]);
    assert!(r.holds(&one).is_err() && r.holds(&two).is_err());
}

#[test]
fn a_change_outside_the_runs_landings_refuses() {
    let r = repo();
    let one = r.land("fix-1", Some("x"), Some("a"));
    r.write(Some("edited by hand"));
    assert!(r.holds(&one).is_err());
    r.write(None);
    assert!(r.holds(&one).is_err(), "an outside deletion refuses too");
}

#[test]
fn a_landing_whose_commit_is_gone_or_another_runs_refuses() {
    let r = repo();
    let one = r.land("fix-1", Some("x"), Some("a"));
    git(&r.root, &["reset", "-q", "--hard", "HEAD~1"]);
    r.write(Some("a"));
    assert!(r.holds(&one).unwrap_err().contains("no landing commit"));
    let r = repo();
    let other = r.land_in("other-run", "fix-1", Some("x"), Some("a"));
    let mut mine = other.clone();
    mine.run_id = RUN.into();
    assert!(
        r.holds(&mine).is_err(),
        "another run's commit proves nothing"
    );
    // A commit of the right stage whose content is not the manifest's.
    let r = repo();
    let mut wrong = r.land("fix-1", Some("x"), Some("a"));
    wrong.post_hashes.insert(FILE.into(), hash("z"));
    assert!(r.holds(&wrong).is_err());
}

#[test]
fn a_deletion_then_a_recreation_by_the_run_stands() {
    let r = repo();
    let gone = r.land("fix-1", Some("x"), None);
    assert_eq!(r.holds(&gone), Ok(()));
    let back = r.land("fix-2", None, Some("new"));
    assert_eq!(r.holds(&gone), Ok(()));
    assert_eq!(r.holds(&back), Ok(()));
    r.write(None);
    assert!(r.holds(&back).is_err());
    assert!(
        r.holds(&gone).is_err(),
        "the run's last landing there recreated it"
    );
}

/// A gitignored path is in no commit: it must hold exactly what the
/// manifest recorded, with no order invented for it.
#[test]
fn an_ignored_path_keeps_the_strict_rule() {
    let r = repo();
    std::fs::create_dir_all(r.root.join("ignored")).unwrap();
    std::fs::write(r.root.join("ignored/out.json"), "one").unwrap();
    let mut landing = r.land("fix-1", Some("x"), Some("a"));
    landing.changed_files.push("ignored/out.json".into());
    landing
        .post_hashes
        .insert("ignored/out.json".into(), hash("one"));
    assert_eq!(r.holds(&landing), Ok(()));
    std::fs::write(r.root.join("ignored/out.json"), "two").unwrap();
    assert!(r.holds(&landing).is_err());
}

/// Issue-113: an ignored path the landing MATERIALIZED as a project artifact
/// is judged where it is verified, in the run's copy order
/// (`branch_cache_materialized`), so a later round's copy over it -- the
/// project root being the repository -- does not refuse this landing here.
#[test]
fn a_materialized_ignored_path_is_left_to_the_copy_order() {
    let r = repo();
    std::fs::create_dir_all(r.root.join("ignored")).unwrap();
    std::fs::write(r.root.join("ignored/out.json"), "one").unwrap();
    let mut landing = r.land("fix-1", Some("x"), Some("a"));
    landing
        .post_hashes
        .insert("ignored/out.json".into(), hash("one"));
    landing.materialized.insert(
        "ignored/out.json".into(),
        crate::write_coordinator::MaterializedDeliverable {
            destination: r.root.join("ignored/out.json").display().to_string(),
            pre_hash: "absent".into(),
            post_hash: hash("one"),
            sequence: 1,
        },
    );
    std::fs::write(r.root.join("ignored/out.json"), "two").unwrap();
    assert_eq!(r.holds(&landing), Ok(()));
}
