use super::*;
#[tokio::test]
async fn judged_commands_mutate_only_the_committed_copy() {
    let _clock = tests::freeze_runner_clock();
    let temp = tempfile::tempdir().unwrap();
    let (root, _, host, _) = tests::world(temp.path());
    let commit = tests::head(&root);
    let store = WorkflowV2ResultStore::new(temp.path().join("run/v2"));
    std::fs::write(root.join("untracked"), "private").unwrap();
    std::fs::write(root.join("src/old_red"), "live edit").unwrap();
    for action in [
        "test ! -f untracked && test \"$(cat src/old_red)\" = '// x' && rm src/old_red",
        "printf changed > src/shared_tests.rs",
        "test -d .git && test ! -f .git/objects/info/alternates",
    ] {
        let command = format!(
            "({action}) && printf 'running 0 tests\\ntest result: ok. 0 passed; 0 failed\\n'"
        );
        let commands = vec![command.clone()];
        let runs = host_verdicts(
            &store,
            &host,
            &root,
            Tree::Judged,
            &commit,
            &commands,
            &FrozenCommands::from_commands(&commands),
        )
        .await;
        assert_eq!(runs[&command].exit_code, Some(0));
        assert_eq!(
            std::fs::read_to_string(root.join("src/old_red")).unwrap(),
            "live edit"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("src/shared_tests.rs")).unwrap(),
            "// x\n"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("untracked")).unwrap(),
            "private"
        );
        assert_eq!(tests::head(&root), commit);
    }
}
