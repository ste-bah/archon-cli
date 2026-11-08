use super::*;
use crate::command::workflow_task_set::republish::test_fixture::frozen_set_proven as frozen_set;

#[test]
fn reauthor_prompt_omits_path_when_site_policy_capture_fails() {
    let set = frozen_set(&[("AC-X-001", "run", false)]);
    let scope =
        AuthorScope::for_task_set_without_cache(set.project.path(), &set.tasks, &set.prd).unwrap();
    std::fs::create_dir_all(scope.project_root.join(".archon")).unwrap();
    std::fs::write(scope.project_root.join(".archon/config.toml"), "[invalid").unwrap();
    let prompt = super::author::author_prompt_with_prefetch(
        &scope,
        &set.contract().acceptance[0],
        &["repair".into()],
        1,
        &[],
        &[],
    )
    .expect("read requirement texts");
    assert!(!prompt.contains("Checks run at the check site with PATH="));
}
