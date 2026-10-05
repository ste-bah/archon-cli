//! Issue 331 through a real freeze probe at a configured scratch site: a
//! check that runs `cargo nextest` on a toolchain path with `cargo` (and a
//! `cargo-*` program) but no `cargo-nextest`, which the host has, is
//! unproven, then its author's finding -- never a proof.

use super::super::super::probe_tests::{Trees, scratch_left, trees};
use super::super::super::verdict_probe_tests::REPO;
use super::super::super::{ExecutabilityProbe, HostProbe};
use super::HOST_PATH;
use super::tests::{Bin, NEXTEST, SIBLING, host_cargo};

/// Write `trees`' scratch policy, with the toolchain path `toolchain`.
fn configure(trees: &Trees, toolchain: &str) -> std::path::PathBuf {
    let scratch = trees.outside.path().join("scratch");
    std::fs::create_dir_all(trees.set.project.path().join(".archon")).unwrap();
    std::fs::write(
        trees.set.project.path().join(".archon/config.toml"),
        format!(
            "[workflow.acceptance_execution]\nrepository={:?}\nscratch_parent={:?}\nproject_inputs=[\"data\"]\nproject_repository_view=\"separate\"\ntoolchain_path={:?}\ntimeout_secs=60\noutput_bytes=8192\nscratch_bytes=16777216\n",
            trees.repo.display().to_string(),
            scratch.display().to_string(),
            toolchain,
        ),
    )
    .unwrap();
    scratch
}

#[tokio::test]
async fn a_scratch_freeze_never_counts_a_missing_cargo_subcommand_as_a_proof() {
    let Some(cargo) = host_cargo() else {
        eprintln!("skipped: no cargo on the host's PATH");
        return;
    };
    let bin = Bin::with_host(&cargo, &[SIBLING], &[NEXTEST]);
    HOST_PATH.with(|path| *path.borrow_mut() = Some(bin.host_path()));
    let trees = trees(&[("AC-331-001", "cargo nextest run --workspace", REPO)]);
    let scratch = configure(&trees, &bin.path());
    let copies = tempfile::tempdir().unwrap();
    let freeze = || {
        HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks)
            .with_copy_parent(copies.path().to_path_buf())
    };
    let first = freeze();
    assert!(matches!(first.site, super::super::super::Site::Scratch(_)));
    let findings = first.script_defects(&trees.contract(), &trees.ids()).await;
    let unproven = first.take_unproven();
    let runs = first.take_baseline_runs().expect("a baseline");
    assert!(
        findings.is_empty(),
        "never the author's at first: {findings:?}"
    );
    assert!(runs.failures.is_empty(), "no proof: {runs:?}");
    let why = unproven.get("AC-331-001").unwrap_or_else(|| {
        panic!(
            "a missing subcommand is no proof: {unproven:?} {:?}",
            first.take_diagnostics()
        )
    });
    assert!(
        why.contains("`cargo nextest`") && why.contains(&bin.path()),
        "{why}"
    );
    let second = freeze();
    let findings = second.script_defects(&trees.contract(), &trees.ids()).await;
    assert!(second.take_unproven().is_empty(), "no longer the host's");
    let finding = findings.get("AC-331-001").expect("the author's finding");
    assert!(
        finding.contains("cannot be proven on the base commit")
            && finding.contains("`cargo nextest`"),
        "{finding}"
    );
    assert!(
        scratch_left(&scratch).is_empty(),
        "{:?}",
        scratch_left(&scratch)
    );
    trees.assert_live_untouched(copies.path());
}

#[tokio::test]
async fn a_scratch_freeze_counts_a_subcommand_the_deliverable_adds_as_a_proof() {
    let Some(cargo) = host_cargo() else {
        eprintln!("skipped: no cargo on the host's PATH");
        return;
    };
    // The host has no `cargo-archon333alias`: an alias the tree may add.
    let bin = Bin::with_host(&cargo, &[SIBLING], &[NEXTEST]);
    HOST_PATH.with(|path| *path.borrow_mut() = Some(bin.host_path()));
    let trees = trees(&[("AC-333-001", "cargo archon333alias --check", REPO)]);
    let scratch = configure(&trees, &bin.path());
    let copies = tempfile::tempdir().unwrap();
    let probe = HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks)
        .with_copy_parent(copies.path().to_path_buf());
    let findings = probe.script_defects(&trees.contract(), &trees.ids()).await;
    let unproven = probe.take_unproven();
    let runs = probe.take_baseline_runs().expect("a baseline");
    assert!(findings.is_empty(), "{findings:?}");
    assert!(unproven.is_empty(), "never the host's: {unproven:?}");
    let failed = runs.failures.get("AC-333-001").expect("a proof");
    assert!(
        String::from_utf8_lossy(&failed.stderr).contains("no such command: `archon333alias`"),
        "{failed:?}"
    );
    assert!(scratch_left(&scratch).is_empty());
    trees.assert_live_untouched(copies.path());
}
