fn reject_forbidden_text(text: &str) -> Result<(), WorkflowV2AgentError> {
    let lower = text.to_ascii_lowercase();
    if lower.contains("restored context")
        || lower.contains("context restored")
        || lower.contains("previous-session summary")
        || lower.contains("previous session summary")
    {
        return Err(WorkflowV2AgentError::RestoredContextSummary);
    }
    if (lower.contains("should i ")
        || lower.contains("do you want me")
        || lower.contains("would you like me")
        || lower.contains("can i proceed"))
        && text.contains('?')
    {
        return Err(WorkflowV2AgentError::ConfirmationQuestion);
    }
    Ok(())
}

fn reject_forbidden_result_text(result: &WorkflowV2Result) -> Result<(), WorkflowV2AgentError> {
    reject_forbidden_text(&result.summary)?;
    for evidence in &result.evidence {
        reject_forbidden_text(&evidence.summary)?;
    }
    for coverage in &result.task_coverage {
        reject_forbidden_text(&coverage.summary)?;
        for evidence in &coverage.evidence {
            reject_forbidden_text(&evidence.summary)?;
        }
    }
    Ok(())
}

fn plan_only_text(result: &WorkflowV2Result) -> bool {
    let mut fields = vec![result.summary.as_str()];
    fields.extend(
        result
            .evidence
            .iter()
            .map(|evidence| evidence.summary.as_str()),
    );
    fields.iter().any(|field| {
        let lower = field.to_ascii_lowercase();
        lower.contains("i will ")
            || lower.contains("we will ")
            || lower.contains("would implement")
            || lower.contains("next steps")
            || lower.contains("proposed changes")
            || lower.contains("implementation plan")
    })
}

pub(super) fn write_mode_label(write_mode: Option<WorkflowV2WriteMode>) -> &'static str {
    match write_mode {
        Some(WorkflowV2WriteMode::Serial) => "serial",
        Some(WorkflowV2WriteMode::Coordinated) => "coordinated",
        Some(WorkflowV2WriteMode::Worktree) => "worktree",
        None => "read_only",
    }
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for ch in value.chars().take(max_chars) {
        out.push(ch);
    }
    out
}

/// Bounded single-line head of a raw agent reply, embedded in parse errors so
/// persisted failure records and retry briefs show WHAT the agent wrote, not
/// just where serde gave up.
fn output_excerpt(output: &str) -> String {
    let collapsed: String = output.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_chars(&collapsed, 200)
}

/// Shared by read-only and implementation work: both run filtered test commands
/// as evidence, and a filter that matched nothing is not evidence. A macro
/// rather than a const because `concat!` accepts only literals.
macro_rules! test_filter_rule {
    () => {
        "- TEST COMMANDS: pass ONE filter per invocation. Most runners (cargo included) reject or silently ignore multiple positional filters, so a combined command can exit 0 while the filters you named matched nothing — that is not evidence and the host demotes it. Run each filter as its own command and check each reported a non-zero match count.\n"
    };
}

pub(super) const READ_ONLY_RULES: &str = concat!(
    "- This is read-only work: do not claim file edits and leave files_changed empty.\n",
    "- For project artifact checks, use project_artifact_paths absolute_path values when present; otherwise resolve .archon/... paths under project_artifact_root, not repository_root.\n",
    // Verification branches are read-only and are the ones running filtered test
    // commands to prove a task, so this rule matters more here than on the write
    // side where it originally lived alone.
    test_filter_rule!(),
    "- Run test and build commands from the repository root you were given; a runner invoked from the project artifact root will not find the source workspace."
);

pub(super) const IMPLEMENTATION_RULES: &str = concat!(
    "- This is implementation-capable work.\n",
    "- If edits are required and made, status must be accepted and files_changed must list each changed path.\n",
    "- The top-level status is your branch verdict; nested artifact/evidence content may describe fail-closed examples such as validation reports with status=failed.\n",
    "- commands_run.kind must be one of inspect, test, build, format, review, or other; use other for implementation notes.\n",
    test_filter_rule!(),
    "- Accepted/noop results must include concrete evidence: files/artifacts, commands_run, task_coverage, and residual_gaps when relevant.\n",
    "- Repository source edits must stay under repository_root and declared target_files; workflow/project artifacts must be written under project_artifact_root when provided and listed in artifacts.\n",
    "- FILE SIZE: a source file that would exceed the repository's per-file line cap is REJECTED and none of your patch is credited. Do not grow a file past the cap. You also own the module directory of each declared target (declaring `a/b.rs` owns `a/b/`), so when a target is at or near the cap, SPLIT it: move cohesive sections into new files under that module directory and re-export them from the original. Splitting is expected and in scope — silently growing the file until the gate rejects the whole patch is not.\n",
    "- If a write branch is genuinely already complete with no patch, return top-level \"status\":\"noop\", \"idempotent_noop\":true, commands_run evidence, and accepted/noop task_coverage evidence.\n",
    "- If no edits are required because the work is already complete, status must be noop and task_coverage must include typed evidence; declared project artifacts also require existing artifact evidence.\n",
    "- Status accepted with no files_changed is invalid unless concrete project artifact evidence was written under project_artifact_root."
);

pub(super) const FINAL_OUTPUT_RULE: &str = r#"## Final Output Rule
Your final message must be exactly one JSON WorkflowV2Result object, even for a no-op. Example: {"status":"noop","idempotent_noop":true,"summary":"already satisfied","evidence":[{"kind":"inspection","summary":"verified existing implementation"}],"commands_run":[{"kind":"inspect","command":"exact check","status":"succeeded","exit_code":0,"output_summary":"passed"}],"files_changed":[],"task_coverage":[{"task_id":"canonical task id","status":"noop","summary":"already satisfied","evidence":[{"kind":"implementation","summary":"concrete proof"}]}],"residual_gaps":[]}. Never return prose such as Status: noop."#;

pub(super) const RESULT_SCHEMA: &str = r#"{
  "status": "accepted | noop | failed | blocked | needs_review | cancelled",
  "idempotent_noop": "optional boolean; true only for a top-level noop with concrete evidence and no patch",
  "summary": "concise factual summary",
  "evidence": [{"kind": "inspection | implementation | test | review | remediation | blocker | artifact | other", "summary": "specific evidence", "source": "optional path or command"}],
  "artifacts": [{"id": "stable-id", "path": "artifact/path", "description": "optional"}],
  "commands_run": [{"kind": "inspect | test | build | format | review | other", "command": "exact command", "status": "succeeded | failed | skipped", "exit_code": 0, "output_summary": "short output"}],
  "files_read": [{"path": "path", "purpose": "optional"}],
  "files_changed": [{"path": "path", "purpose": "optional"}],
  "task_coverage": [{"task_id": "canonical id", "status": "accepted | noop | partial | missing | blocked | unknown", "summary": "coverage summary", "evidence": [{"kind": "implementation", "summary": "evidence"}]}],
  "residual_gaps": [{"id": "gap-id", "description": "remaining gap", "severity": "optional"}],
  "data": {"items": "optional typed payload for downstream fanout/reduce"}
}"#;

#[cfg(test)]
#[path = "agent_adapter_artifact_context_tests.rs"]
mod artifact_context_tests;
#[cfg(test)]
#[path = "agent_adapter_envelope_tests.rs"]
mod envelope_tests;
#[cfg(test)]
#[path = "agent_adapter_project_artifact_completion_tests.rs"]
mod project_artifact_completion_tests;
#[cfg(test)]
#[path = "agent_prompt_digest_tests.rs"]
mod prompt_digest_tests;
#[cfg(test)]
#[path = "agent_prompt_growth_tests.rs"]
mod prompt_growth_tests;
#[cfg(test)]
#[path = "agent_prompt_tests.rs"]
mod prompt_tests;
#[cfg(test)]
#[path = "agent_adapter_required_tools_tests.rs"]
mod required_tools_tests;
#[cfg(test)]
#[path = "agent_adapter_tests.rs"]
mod tests;

#[cfg(test)]
mod shared_rule_tests {
    /// The one-filter rule lived only in IMPLEMENTATION_RULES, but VERIFICATION
    /// branches are read-only and are precisely the ones running filtered test
    /// commands as proof. In one observed run agents issued 63 invalid
    /// multi-filter cargo commands and self-corrected, burning cycles against
    /// guidance they were never shown.
    ///
    /// Worse than the wasted cycles: a runner that silently ignores extra
    /// filters exits 0 having matched nothing, which is the zero-match evidence
    /// the host already demotes. Both paths must carry the rule.
    #[test]
    fn both_rule_sets_carry_the_test_filter_rule() {
        assert!(
            super::READ_ONLY_RULES.contains("ONE filter per invocation"),
            "read-only work runs filtered test commands too: {}",
            super::READ_ONLY_RULES
        );
        assert!(
            super::IMPLEMENTATION_RULES.contains("ONE filter per invocation"),
            "{}",
            super::IMPLEMENTATION_RULES
        );
    }
}

#[cfg(test)]
mod test_module_gating_tests {
    /// Every `mod *_tests` declaration must carry `#[cfg(test)]`.
    ///
    /// A union merge resolution left one of six bare, because the attribute and
    /// the item it modifies fell on opposite sides of a conflict marker. The
    /// consequence was test code compiling into the PRODUCTION binary, and
    /// nothing caught it: cargo check was clean, every test passed, the module
    /// compiled. The only signal was a dead_code lint on two helpers that have
    /// no callers outside tests.
    ///
    /// Third appearance of this shape in one session — an attribute binding to
    /// the wrong item, so tests silently stop being tests. Cheap to assert,
    /// expensive to find.
    #[test]
    fn every_test_module_is_gated_behind_cfg_test() {
        // Read the assembled module, not just the parent. After the file-size
        // split the parent holds only `include!` lines, so scanning it alone
        // finds no `mod` at all and this guard passes vacuously — precisely
        // the failure mode described above.
        let source = [
            include_str!("agent_adapter.rs"),
            include_str!("agent_adapter_a.rs"),
            include_str!("agent_adapter_b.rs"),
        ]
        .concat();
        let lines: Vec<&str> = source.lines().collect();
        let mut bare = Vec::new();
        for (index, line) in lines.iter().enumerate() {
            let trimmed = line.trim_start();
            let is_test_mod = trimmed.starts_with("mod ")
                && trimmed
                    .trim_start_matches("mod ")
                    .split(|c: char| c == ';' || c == '{' || c.is_whitespace())
                    .next()
                    .is_some_and(|name| name.contains("test"));
            if !is_test_mod {
                continue;
            }
            let start = index.saturating_sub(3);
            if !lines[start..index]
                .iter()
                .any(|l| l.contains("#[cfg(test)]"))
            {
                bare.push(format!("line {}: {}", index + 1, trimmed));
            }
        }
        assert!(
            bare.is_empty(),
            "test modules compiling into the production binary: {bare:#?}"
        );
    }
}
