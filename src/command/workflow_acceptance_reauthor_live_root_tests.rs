//! Issue 366: the host re-author is told the same check-path rule as the
//! decomposition author, and a reply naming a live root by its absolute path
//! is refused before the judge, in the freeze's own words.

use super::test_client::{ScriptedAuthorJudge, command_entry};
use super::*;
use crate::command::workflow_task_set::live_root::live_root_finding;
use crate::command::workflow_task_set::republish::test_fixture::frozen_set_proven as frozen_set;
use std::sync::{Arc, Mutex};

const RULE: &str = "name every path relative to the check's working directory";

fn ids(list: &[&str]) -> BTreeSet<String> {
    list.iter().map(|id| id.to_string()).collect()
}

#[tokio::test]
async fn the_reauthor_prompt_states_where_a_check_runs_and_the_relative_path_rule() {
    let set = frozen_set(&[("AC-F-001", "jq -e '.a == true' out.json", false)]);
    let scope = AuthorScope::for_task_set(set.project.path(), &set.tasks, &set.prd)
        .expect("the task set's repository record is believed");
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

/// Issue 366 N3: the live-root reply never reaches the judge. Every check
/// the judge is sent is recorded; none names the root, in any spelling, and
/// the relative repair is judged (so the judge was reached at all).
#[tokio::test]
async fn a_reply_naming_a_live_root_is_refused_before_the_judge_in_the_freeze_words() {
    const REPAIR: &str = "jq -e '.a == 1' out.json";
    let set = frozen_set(&[("AC-F-001", "jq -e '.a == true' out.json", false)]);
    let scope = AuthorScope::for_task_set(set.project.path(), &set.tasks, &set.prd)
        .expect("the task set's repository record is believed");
    let root = set.project.path().canonicalize().unwrap();
    for live in [
        format!("jq -e '.a == 1' {}/out.json", root.display()),
        format!("jq -e '.a == 1' \"{}\"/out.json", root.display()),
        format!("cd {}//. && jq -e '.a == 1' out.json", root.display()),
    ] {
        let judged = Arc::new(Mutex::new(Vec::<String>::new()));
        let seen = judged.clone();
        let reply = live.clone();
        let client = ScriptedAuthorJudge::new(
            move |entry, attempt| match attempt {
                1 => command_entry(entry, &reply),
                _ => command_entry(entry, REPAIR),
            },
            move |id, check| {
                seen.lock().unwrap().push(format!("{id} {check}"));
                true
            },
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
        assert!(
            prompts.len() >= 2,
            "{live}: the refused reply is re-authored"
        );
        assert!(
            prompts[1].contains(&finding),
            "{live}: the next attempt shows the freeze's own refusal:\n{}",
            prompts[1]
        );
        let judged = judged.lock().unwrap();
        let root_text = root.display().to_string();
        assert!(
            judged.iter().all(|check| !check.contains(&root_text)),
            "{live}: a check naming the live root reached the judge: {judged:?}"
        );
        assert!(
            judged.iter().any(|check| check.contains(REPAIR)),
            "{live}: the relative repair is judged: {judged:?}"
        );
    }
}

/// Issue 366 N1: the host re-author is never told, nor confined to, a
/// guessed repository root: a lock it cannot believe is refused by name.
#[test]
fn a_repository_lock_that_cannot_be_believed_gives_no_author_scope() {
    for (case, lock) in crate::command::workflow_task_set::live_root::tests::UNBELIEVABLE_LOCKS {
        let set = frozen_set(&[("AC-F-001", "jq -e '.a == true' out.json", false)]);
        let path = archon_workflow::repository_record::repository_record_path(&set.tasks);
        std::fs::write(&path, lock).unwrap();
        let why = AuthorScope::for_task_set(set.project.path(), &set.tasks, &set.prd)
            .err()
            .unwrap_or_else(|| panic!("{case}: a scope was built from a guessed root"));
        assert!(why.contains(&path.display().to_string()), "{case}: {why}");
    }
}
