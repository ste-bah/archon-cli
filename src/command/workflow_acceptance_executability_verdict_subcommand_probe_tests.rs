//! Issue 331 through a real freeze probe at a configured scratch site: a
//! check that runs `cargo nextest` on a toolchain path with `cargo` (and a
//! `cargo-*` program) but no `cargo-nextest` is unproven, then its
//! author's finding -- never a proof.

use super::super::super::probe_tests::{scratch_left, trees};
use super::super::super::verdict_probe_tests::REPO;
use super::super::super::{ExecutabilityProbe, HostProbe};
use super::tests::{Bin, SIBLING, host_cargo};

#[tokio::test]
async fn a_scratch_freeze_never_counts_a_missing_cargo_subcommand_as_a_proof() {
    let Some(cargo) = host_cargo() else {
        eprintln!("skipped: no cargo on the host's PATH");
        return;
    };
    let bin = Bin::new(&cargo, &[SIBLING]);
    let trees = trees(&[("AC-331-001", "cargo nextest run --workspace", REPO)]);
    let scratch = trees.outside.path().join("scratch");
    std::fs::create_dir_all(trees.set.project.path().join(".archon")).unwrap();
    std::fs::write(
        trees.set.project.path().join(".archon/config.toml"),
        format!(
            "[workflow.acceptance_execution]\nrepository={:?}\nscratch_parent={:?}\nproject_inputs=[\"data\"]\nproject_repository_view=\"separate\"\ntoolchain_path={:?}\ntimeout_secs=60\noutput_bytes=8192\nscratch_bytes=16777216\n",
            trees.repo.display().to_string(),
            scratch.display().to_string(),
            bin.path(),
        ),
    )
    .unwrap();
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
