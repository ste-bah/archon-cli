//! The `## Baseline Tests` section of a verifier's prompt (Obs-31).
//!
//! Rendered from the stamp `verification::baseline_rule` put on the item, in
//! the same words the coder was given, with the rule the host will enforce
//! on the verifier's report: a red test in the task's declared filter refuses
//! acceptance unless it is on the other-owner or leave-alone list, and
//! "pre-existing" is not a reason. Empty for an item without a stamp.

use serde_json::Value;

use crate::v2::verification::baseline_rule::stamped;

pub(super) fn baseline_tests_prompt_section(input: &Value) -> String {
    let Some(stamp) = stamped(input) else {
        return String::new();
    };
    let sha: String = stamp.base_commit.chars().take(12).collect();
    let mut text = format!(
        "## Baseline Tests\n\
         The host ran this task's declared focused test commands on the base commit {sha} \
         before any task changed the tree. Rule: the task is NOT accepted while any test in its \
         declared filter fails, unless that test is listed below as owned by another task or as \
         one to leave alone. \"Pre-existing\" is not an acceptable reason to accept a red test, \
         and a `pre_existing: true` command record is honoured only when every test its output \
         names is on those lists; the host re-reads your commands_run output for `test <name> \
         ... FAILED` lines and refuses an accepted verdict that leaves any other test red.\n"
    );
    if stamp.must_pass.is_empty() {
        text.push_str("- Must pass (red on the base commit, this task's to fix): none recorded; every red test in the filter is this task's.\n");
    } else {
        text.push_str(&format!(
            "- Must pass (red on the base commit, this task's to fix): {}\n",
            stamp.must_pass.join(", ")
        ));
    }
    if !stamp.other_owner.is_empty() {
        let listed: Vec<String> = stamp
            .other_owner
            .iter()
            .map(|t| format!("{} (owned by {})", t.test_id, t.owner_task))
            .collect();
        text.push_str(&format!(
            "- Owned by another task (may stay red; record as pre_existing with this list as the evidence): {}\n",
            listed.join(", ")
        ));
    }
    if !stamp.ignored.is_empty() {
        text.push_str(&format!(
            "- To leave alone (may stay red): {}\n",
            stamp.ignored.join(", ")
        ));
    }
    for pre in &stamp.pre_existing_diagnostics {
        text.push_str(&format!(
            "- `{}` already fails on the base commit for error diagnostics in {} file(s) outside \
             this task's target_files: {}. Record it as pre_existing with those locations as \
             the evidence; the host re-reads your output for `--> path` locations and honours \
             the claim only while every location is in this list — a diagnostic in any other \
             file is this task's failure.\n",
            pre.command,
            pre.files.len(),
            pre.files.join(", ")
        ));
    }
    if !stamp.unbaselined_commands.is_empty() {
        text.push_str(&format!(
            "- Declared commands the host could not baseline (no exemption applies to their failures): {}\n",
            stamp
                .unbaselined_commands
                .iter()
                .map(|c| format!("`{c}`"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    text.push('\n');
    text
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::baseline_tests_prompt_section;
    use crate::v2::verification::baseline_rule::BASELINE_TESTS_INPUT_KEY;

    #[test]
    fn the_section_carries_the_lists_and_the_rule_and_is_absent_without_a_stamp() {
        assert_eq!(baseline_tests_prompt_section(&json!({"item": {}})), "");
        let input = json!({
            "item": {},
            BASELINE_TESTS_INPUT_KEY: {
                "base_commit": "abcdef0123456789",
                "declared_commands": ["cargo test -p engine grant"],
                "must_pass": ["grant::tests::mine"],
                "other_owner": [{"test_id": "plan::tests::theirs", "owner_task": "TASK-B"}],
                "ignored": ["gate::frozen"],
                "unbaselined_commands": ["cargo test -p engine slow"],
                "pre_existing_diagnostics": [{
                    "command": "cargo clippy -p engine -- -D warnings",
                    "files": ["crates/engine/src/gate.rs", "crates/other/src/lib.rs"],
                }],
            }
        });
        let text = baseline_tests_prompt_section(&input);
        assert!(text.starts_with("## Baseline Tests\nThe host ran this task's declared focused test commands on the base commit abcdef012345 "), "{text}");
        assert!(text.contains("the task is NOT accepted while any test in its declared filter fails, unless that test is listed below as owned by another task or as one to leave alone. \"Pre-existing\" is not an acceptable reason"), "{text}");
        assert!(
            text.contains(
                "- Must pass (red on the base commit, this task's to fix): grant::tests::mine\n"
            ),
            "{text}"
        );
        assert!(text.contains("- Owned by another task (may stay red; record as pre_existing with this list as the evidence): plan::tests::theirs (owned by TASK-B)\n"), "{text}");
        assert!(
            text.contains("- To leave alone (may stay red): gate::frozen\n"),
            "{text}"
        );
        assert!(text.contains("- Declared commands the host could not baseline (no exemption applies to their failures): `cargo test -p engine slow`\n"), "{text}");
        assert!(text.contains("- `cargo clippy -p engine -- -D warnings` already fails on the base commit for error diagnostics in 2 file(s) outside this task's target_files: crates/engine/src/gate.rs, crates/other/src/lib.rs. Record it as pre_existing with those locations as the evidence;"), "{text}");
    }
}
