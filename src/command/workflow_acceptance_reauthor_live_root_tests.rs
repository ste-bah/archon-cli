//! Issue 366: the host re-author is told the same check-path rule as the
//! decomposition author, and a reply naming a live root by its absolute path
//! is refused before the judge, in the freeze's own words.

use super::test_client::{ScriptedAuthorJudge, command_entry};
use super::*;
use crate::command::workflow_task_set::live_root::live_root_finding;
use crate::command::workflow_task_set::republish::test_fixture::frozen_set_proven as frozen_set;

const RULE: &str = "name every path relative to the check's working directory";

fn ids(list: &[&str]) -> BTreeSet<String> {
    list.iter().map(|id| id.to_string()).collect()
}

#[tokio::test]
async fn the_reauthor_prompt_states_where_a_check_runs_and_the_relative_path_rule() {
    let set = frozen_set(&[("AC-F-001", "jq -e '.a == true' out.json", false)]);
    let scope = AuthorScope::for_task_set(set.project.path(), &set.tasks, &set.prd);
    let client = ScriptedAuthorJudge::new(
        |entry, _| command_entry(entry, "jq -e '.a == 1' out.json"),
        |_, _| true,
    );
    let _ = reauthor(
        &client,
        &set.contract(),
        &ids(&["AC-F-001"]),
        &scope,
        "sonnet",
        &set.gate(),
    )
    .await;
    let prompts = client.prompts.lock().unwrap();
    let prompt = prompts.first().expect("one author call");
    let rule = (prompt.lines())
        .find(|line| line.contains(RULE))
        .unwrap_or_else(|| panic!("the re-author prompt has no relative-path rule:\n{prompt}"));
    for root in [&scope.repository_root, &scope.project_root] {
        assert!(rule.contains(&root.display().to_string()), "{rule}");
    }
    assert!(
        prompt
            .contains("must not change, delete or reset anything outside its own temporary files"),
        "{prompt}"
    );
    assert!(
        !prompt.contains("never runs a check in the live"),
        "{prompt}"
    );
}

#[tokio::test]
async fn a_reply_naming_a_live_root_is_refused_before_the_judge_in_the_freeze_words() {
    let set = frozen_set(&[("AC-F-001", "jq -e '.a == true' out.json", false)]);
    let scope = AuthorScope::for_task_set(set.project.path(), &set.tasks, &set.prd);
    let root = set.project.path().canonicalize().unwrap();
    let absolute = format!("jq -e '.a == 1' {}/out.json", root.display());
    let client = ScriptedAuthorJudge::new(
        move |entry, attempt| match attempt {
            1 => command_entry(entry, &absolute),
            _ => command_entry(entry, "jq -e '.a == 1' out.json"),
        },
        |_, _| true,
    );
    let _ = reauthor(
        &client,
        &set.contract(),
        &ids(&["AC-F-001"]),
        &scope,
        "sonnet",
        &set.gate(),
    )
    .await;
    let finding = live_root_finding("AC-F-001", &root);
    let prompts = client.prompts.lock().unwrap();
    assert!(prompts.len() >= 2, "the refused reply is re-authored");
    assert!(
        prompts[1].contains(&finding),
        "the next attempt shows the freeze's own refusal:\n{}",
        prompts[1]
    );
    // The judge saw the original (to seed its repair) and the relative
    // repair, never the live-root reply.
    let judged = client.judged_ids.lock().unwrap().len();
    assert!(
        judged <= 2,
        "the live-root reply reached the judge ({judged} judged)"
    );
}
