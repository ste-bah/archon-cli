use super::*;
use crate::v2::write::test_baseline_run_base::tests::{freeze_runner_clock, world};
use crate::v2::write::test_baseline_run_base::{FrozenCommands, cached_per_test};

#[tokio::test]
async fn generic_exit_verdicts_run_frozen_text_without_inventing_per_test_evidence() {
    let _clock = freeze_runner_clock();
    let temp = tempfile::tempdir().unwrap();
    let (root, base, host, _) = world(temp.path());
    let store = WorkflowV2ResultStore::new(temp.path().join("run/v2"));
    for (command, expected) in [
        ("word='one two'; test ${#word} -eq 7", 0),
        ("word='one two'; test ${#word} -ne 7", 1),
        ("word='one two'; [ ${#word} -eq 7 ]", 0),
    ] {
        let frozen = FrozenCommands::from_commands(&[command.into()]);
        let reported = command.replace(' ', "  ");
        let runs = generic_verdicts(
            &store,
            &host,
            &root,
            &base,
            std::slice::from_ref(&reported),
            &frozen,
        )
        .await;
        assert_eq!(runs[&reported].command, command);
        assert_eq!(runs[&reported].exit_code, Some(expected));
        assert!(!runs[&reported].per_test_verdict);
        assert!(cached_per_test(&store, Tree::RunBase, &base, &reported, &frozen).is_none());
    }
}

#[tokio::test]
async fn a_format_lost_after_the_base_is_not_a_tip_per_test_verdict() {
    use crate::v2::write::test_baseline_run_base::tests::{bind_run, commit_files, head};
    let _clock = freeze_runner_clock();
    let temp = tempfile::tempdir().unwrap();
    let (root, base, host, _) = world(temp.path());
    let store = WorkflowV2ResultStore::new(temp.path().join("run/v2"));
    bind_run(&store, &base);
    commit_files(&root, &["src/no_summary"], "unreadable output at tip");
    let tip = head(&root);
    for command in [
        "cargo test -p app --lib",
        "cargo test -q -p app --lib",
        "cargo test -p app --test selected",
    ] {
        let commands = vec![command.into()];
        let at = super::super::regression_slot::verdicts_at;
        let before = at(&store, &host, &root, &base, &commands).await;
        assert!(before[command].per_test_verdict);
        let after = at(&store, &host, &root, &tip, &commands).await;
        assert!(!after.contains_key(command));
        assert_eq!(
            cached(&store, Tree::RunBase, &tip, command)
                .unwrap()
                .exit_code,
            Some(101)
        );
    }
}
