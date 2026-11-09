use super::*;

#[test]
fn declared_commands_are_preserved_for_different_runners() {
    for command in [
        "go test ./pkg -run TestOne",
        "python3 -m unittest tests.One",
        "mvn -pl module -Dtest=One test",
    ] {
        let request = serde_json::from_value(serde_json::json!({
            "run_id":"proof", "stage_id":"focused-tests", "stage_kind":"agent",
            "agent":null, "task":"Run declared focused tests", "attempt":1,
            "provider_tier":"coder", "depends_on":[],
            "input":{"verify_command":command,"stage_extra":{"commands":true}}
        }))
        .unwrap();
        let prompt = workflow_prompt(&request);
        assert!(prompt.contains(command), "{prompt}");
        assert!(prompt.contains("commands exactly as declared"));
        assert!(prompt.contains("never widen a declared command"));
        assert!(prompt.contains("When no command is declared, choose the narrowest available test file, target, module, package, class, or test id; report the reason if the exact target cannot be determined"));
        assert_eq!(prompt.matches("Run declared `verify_command`").count(), 1);
        assert!(!prompt.contains("run that exact command"));
        assert!(!prompt.contains("adapt the commands"));
    }
}

#[test]
fn prompt_constants_do_not_inject_a_toolchain() {
    let forbidden = regex::Regex::new(
        r"(?i)\b(cargo(?:_[a-z_]+)?|rustc|rustdoc|libtest|nextest|clippy|rustfmt|pytest|npm|npx|node|go|javac|mvn|gradle|tsc|python|rust|node_modules|__pycache__|panicked|AssertionError|SyntaxError)\b|REQ-|AC-|\.(rs|py|go|java|ts)\b",
    )
    .unwrap();
    for (name, source) in [
        ("stage", include_str!("stage_prompt.rs")),
        ("adapter", include_str!("v2/agent_adapter_b.rs")),
        ("baseline", include_str!("v2/agent_prompt_baseline.rs")),
        ("lifecycle", include_str!("v2/lifecycle_prompts.rs")),
        ("repair", include_str!("stage_item_output.rs")),
        (
            "acceptance",
            include_str!("../../../src/command/workflow_live_v3_acceptance_output.rs"),
        ),
        (
            "body",
            include_str!("../../../src/command/workflow_decompose_v1_body.js"),
        ),
    ] {
        for (line, text) in source
            .lines()
            .take_while(|line| !line.starts_with("#[cfg(test)]"))
            .enumerate()
        {
            let text = text.trim_start();
            if !text.starts_with("//") {
                assert!(
                    !forbidden.is_match(text),
                    "{name}:{} injects a toolchain: {text}",
                    line + 1
                );
            }
        }
    }
}

#[test]
fn verification_prompts_keep_host_limits_and_count_intended_targets() {
    use crate::v2::lifecycle_prompts::*;
    assert!(
        VERIFICATION_PLAN_TASK
            .contains("Verify every requested test symbol exists in repository source.")
    );
    for prompt in [VERIFICATION_PLAN_TASK, VERIFICATION_WAVE_TASK] {
        assert!(prompt.contains(
            "Never override environment variables the host sets for build output or job limits"
        ));
    }
    for prompt in [
        VERIFICATION_PLAN_TASK,
        VERIFICATION_WAVE_TASK,
        REVIEW_VERIFICATION_PLAN_TASK,
        REVIEW_VERIFICATION_WAVE_TASK,
    ] {
        assert!(prompt.to_lowercase().contains(
            "count one pass per intended target when a runner prints the same target twice"
        ));
        assert!(prompt.contains("no intended target fails"));
    }
}

#[test]
fn baseline_and_ownership_prompts_keep_enforced_consequences() {
    let adapter = include_str!("v2/agent_adapter_b.rs");
    assert!(adapter.contains("You own the directories in target_ownership_scopes; when a target nears the cap, split it into new files there; splitting is expected."));
    let baseline = include_str!("v2/agent_prompt_baseline.rs");
    assert!(
        baseline.contains("The host refuses an accepted verdict that leaves any other test red.")
    );
    assert!(baseline.contains("A diagnostic in any other file is this task's failure."));
}
