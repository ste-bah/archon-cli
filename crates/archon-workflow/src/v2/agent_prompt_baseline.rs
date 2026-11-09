//! The `## Baseline Tests` section of a verifier's prompt (Obs-31).
//!
//! The host record and the identity conditions for a pre_existing claim.
//! Empty for an item without a stamp.

use serde_json::Value;

use crate::v2::verification::baseline_rule::stamped;

pub(crate) fn baseline_tests_prompt_section(input: &Value) -> String {
    let Some(stamp) = stamped(input) else {
        return String::new();
    };
    let sha: String = stamp.base_commit.chars().take(12).collect();
    // Issue-70: a stamp established at the verification base names the
    // commit the verifier's own run sees, not the task's implementation base.
    let when = if stamp.verification_base {
        format!(
            "on the verification base commit {sha}, the current head of the checkout you run \
             in, after every earlier task's work landed"
        )
    } else {
        format!("on the base commit {sha} before any task changed the tree")
    };
    let mut text = format!(
        "## Baseline Tests\nThe host ran this task's declared focused test commands {when}.\n"
    );
    text.push_str("- Make every listed Must pass test pass.\n");
    text.push_str("- A pre_existing claim requires a stored host tip run.\n");
    text.push_str("- Every red identity in that run must occur in the base set.\n");
    text.push_str("- Base and tip exit codes must match.\n");
    text.push_str("- A red command without a usable identity blocks.\n");
    text.push_str("- The host refuses an accepted verdict that leaves any other test red.\n");
    text.push_str("- Keep every base-passing test visible at tip.\n");
    text.push_str("- Do not disable or delete tests.\n");
    text.push_str("- Do not search previous run directories.\n");
    if stamp.must_pass.is_empty() {
        text.push_str("- Must pass (red on the base commit, this task's to fix): none recorded.\n");
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
            "- Owned by another task (may stay red with host identity evidence): {}\n",
            listed.join(", ")
        ));
    }
    if !stamp.ignored.is_empty() {
        text.push_str(&format!(
            "- To leave alone (may stay red with host identity evidence): {}\n",
            stamp.ignored.join(", ")
        ));
    }
    for pre in &stamp.pre_existing_diagnostics {
        text.push_str(&format!(
            "- `{}`: base failure recorded; files: {}.\n",
            pre.command,
            pre.files.join(", ")
        ));
    }
    if !stamp.pre_existing_diagnostics.is_empty() {
        text.push_str("- A diagnostic in any other file is this task's failure.\n");
    }
    if !stamp.unbaselined_commands.is_empty() {
        text.push_str(&format!(
            "- Declared commands with no proof in this baseline record: {}\n",
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
        assert!(
            text.contains("- A pre_existing claim requires a stored host tip run.\n"),
            "{text}"
        );
        assert!(
            text.contains("- Every red identity in that run must occur in the base set.\n"),
            "{text}"
        );
        assert!(
            text.contains("- Base and tip exit codes must match.\n"),
            "{text}"
        );
        assert!(!text.contains("your commands_run output"), "{text}");
        assert!(
            text.contains(
                "- Must pass (red on the base commit, this task's to fix): grant::tests::mine\n"
            ),
            "{text}"
        );
        assert!(text.contains("- Owned by another task (may stay red with host identity evidence): plan::tests::theirs (owned by TASK-B)\n"), "{text}");
        assert!(
            text.contains(
                "- To leave alone (may stay red with host identity evidence): gate::frozen\n"
            ),
            "{text}"
        );
        assert!(text.contains("- Declared commands with no proof in this baseline record: `cargo test -p engine slow`\n"), "{text}");
        assert!(text.contains("- `cargo clippy -p engine -- -D warnings`: base failure recorded; files: crates/engine/src/gate.rs, crates/other/src/lib.rs."), "{text}");
    }

    #[test]
    fn a_verification_base_stamp_names_the_verification_base_and_the_tests_to_ignore() {
        let input = json!({
            "item": {},
            BASELINE_TESTS_INPUT_KEY: {
                "base_commit": "fedcba9876543210",
                "verification_base": true,
                "declared_commands": ["cargo test -p engine grant"],
                "other_owner": [{"test_id": "plan::tests::theirs", "owner_task": "TASK-B"}],
            }
        });
        let text = baseline_tests_prompt_section(&input);
        assert!(
            text.starts_with(
                "## Baseline Tests\nThe host ran this task's declared focused test commands on \
                 the verification base commit fedcba987654, the current head of the checkout \
                 you run in, after every earlier task's work landed.\n"
            ),
            "{text}"
        );
        assert!(!text.contains("before any task changed the tree"), "{text}");
        assert!(text.contains("- Owned by another task (may stay red with host identity evidence): plan::tests::theirs (owned by TASK-B)\n"), "{text}");
    }
}

#[cfg(test)]
#[path = "agent_prompt_baseline_agnostic_proof_tests.rs"]
mod agnostic_proof_tests;
