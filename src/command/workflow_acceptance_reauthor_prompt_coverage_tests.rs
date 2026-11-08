use super::*;

#[test]
fn non_baseline_prompt_keeps_the_refused_check_verbatim() {
    let set = frozen_set(&[("AC-F-001", "run", false)]);
    let frozen = set.contract().acceptance[0].clone();
    let author_scope = scope(&set);
    let notes = &["judge rejection: failed to prove behavior".into()];
    let prompt = super::author::author_prompt(&author_scope, &frozen, notes, 1);
    let current = serde_json::json!({
        "id": frozen.id,
        "criterion": frozen.criterion,
        "check": frozen.check,
        "covers": frozen.covers,
        "covers_text": {},
        "droppable_covers": [],
        "gap_permitted": frozen.gap_permitted,
    });
    let expected = [
        format!(
            "Re-author exactly one acceptance entry of a frozen acceptance contract: {}. The findings below explain why the current entry needs revision. Author ONLY this entry, not the whole contract.",
            frozen.id
        ),
        format!("Read the PRD at {}.", author_scope.prd_path.display()),
        format!(
            "The code repository is {}. It is the ONLY place to verify source paths, test names, module layout and whether a file exists: a repository path you name must be one you observed there. Never descend into dependency, build-output or earlier-run directories.",
            author_scope.repository_root.display()
        ),
        format!(
            "{} is the project root. It holds the PRD and the task root; read source under the repository root alone.",
            author_scope.project_root.display()
        ),
        crate::command::workflow_task_set::live_root::check_path_rule(
            &author_scope.repository_root.display().to_string(),
            &author_scope.project_root.display().to_string(),
        ),
        "Return one JSON object with id, criterion, check, covers, gap_permitted, judgment. Keep covers or drop only ids in droppable_covers; keep the entry's own requirement if it has one. Do not add ids. Dropped ids are returned to the acceptance author as uncovered obligations. The examples below are ENTRIES showing the two check shapes; your reply is one such entry and nothing around it.".to_string(),
        super::ENTRY_SHAPES.to_string(),
        super::COMMAND_CWD_EXAMPLES.to_string(),
        "For a command check, choose EXACTLY ONE command form. Block form: the JSON check object holds kind, cwd and \"command_block\": true, with NO \"command\" field, followed by exactly one non-empty fence whose info string is exactly 'check'. Inline form: the JSON check object holds kind, cwd and \"command\" only; command is one short line without quotes or backslashes. Include no command_block and no fence.".to_string(),
        "A check must exercise the deliverable and fail when its criterion is false, not merely match usage text or assert that a file exists. The host judges the entry adversarially against the criterion, then runs it once against the current tree: a check whose own script crashes (a syntax error, an undefined name, a call that does not match a helper it defines) is returned to you. Repairing a crash never licenses weakening the check: keep every assertion, fix only the script defect.".to_string(),
        "The host also runs the check on the tree before any implementation, where it must fail. If it passes there, data the check writes itself is never evidence; movable inputs must exist under the check's selected root before the run. To guard behaviour implemented in code, select the root that contains the existing implementation or test dependency and name the deciding file by its literal path relative to the check's working directory. The check is a valid regression guard only when its pass or fail is decided by that test and it fails whenever the test fails. When the criterion is false, the check must still fail on its own assertion. The host moves aside only inputs it actually moves, never the check's own script, a program it runs, a directory it changes into or a build manifest; a failure of the deciding assertion after the move proves the guard; a build failure at or naming a moved input also proves it, while any other build failure leaves the check unproven; a move-aside failure is never a counterexample. The named file's assertions are known to the judge only when its content is supplied in named_files; supplied content is a redacted, UTF-8-lossy copy that can be read in place of opening the original file. A named path marked absent means the host found no file at that path; it may be a pattern or text argument, not a file, and absence alone is not evidence that behaviour is missing. Only when the path is established to name a deciding file does its absence mean behaviour not implemented yet, judged by the general rules; a named deciding input whose content is not supplied has unknown assertions, so the check cannot be accepted as a regression guard on assumed content. Read the deciding file by its named path only when its content was not supplied.".to_string(),
        format!(
            "Use the exact id {}. Criterion and judgment are host-owned placeholders; gap_permitted stays {}.",
            frozen.id, frozen.gap_permitted
        ),
        "Files already identified for this check are supplied here when available: []".to_string(),
        "Answer once you have read the supplied named_files content for the deciding file, or inspected that file at its named path yourself.".to_string(),
        format!("The entry being replaced: {current}"),
        format!(
            "Attempt 1; the host stops after {} consecutive attempts that repair no check. Findings to fix, oldest first:\n- {}",
            super::REAUTHOR_ATTEMPTS,
            notes.join("\n- ")
        ),
        "Return exactly the entry: for inline form, output only the JSON entry (raw or in one `json` fence); for block form, output the JSON entry (raw or in one `json` fence) followed by exactly one non-empty fence labelled `check`. Emit no prose, explanations, headings or other fences.".to_string(),
        "Do not run commands or write files.".to_string(),
    ]
    .join("\n");
    let compared = prompt
        .lines()
        .filter(|line| !line.starts_with("Checks run at the check site with PATH="))
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(compared, expected);
    let replaced = prompt
        .lines()
        .find(|line| line.starts_with("The entry being replaced"))
        .unwrap();
    assert!(replaced.contains("\"check\":"), "{replaced}");
    let entry: serde_json::Value = serde_json::from_str(
        replaced
            .strip_prefix("The entry being replaced: ")
            .expect("the replacement entry prefix"),
    )
    .expect("the replacement entry is JSON");
    assert_eq!(entry["check"], serde_json::to_value(&frozen.check).unwrap());
}
