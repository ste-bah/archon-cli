//! Tests for Pre/PostToolUse Hook Policies (TASK-PIPE-E08).
//!
//! Validates: phase-based Write/Edit/Bash denial, dangerous command blocking,
//! affected_files scope, orphan warning, periodic cargo check, forbidden patterns.

use archon_pipeline::coding::hooks::{HookDecision, PreToolUseHook, PostToolUseHook};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn pre_hook(phase: u8, affected: &[&str]) -> PreToolUseHook {
    PreToolUseHook::new(phase, affected.iter().map(|s| s.to_string()).collect())
}

fn tool_input_file(path: &str) -> serde_json::Value {
    serde_json::json!({ "file_path": path })
}

fn tool_input_bash(command: &str) -> serde_json::Value {
    serde_json::json!({ "command": command })
}

fn tool_input_edit(path: &str, old: &str, new: &str) -> serde_json::Value {
    serde_json::json!({ "file_path": path, "old_string": old, "new_string": new })
}

// ---------------------------------------------------------------------------
// PreToolUse Tests
// ---------------------------------------------------------------------------

mod pre_hook_tests {
    use super::*;

    #[test]
    fn phase_1_blocks_write() {
        let hook = pre_hook(1, &["src/main.rs"]);
        let decision = hook.evaluate("Write", &tool_input_file("src/main.rs"));
        assert!(matches!(decision, HookDecision::Block { .. }), "Phase 1 should block Write");
    }

    #[test]
    fn phase_2_blocks_edit() {
        let hook = pre_hook(2, &["src/main.rs"]);
        let decision = hook.evaluate("Edit", &tool_input_edit("src/main.rs", "old", "new"));
        assert!(matches!(decision, HookDecision::Block { .. }), "Phase 2 should block Edit");
    }

    #[test]
    fn phase_3_blocks_bash() {
        let hook = pre_hook(3, &[]);
        let decision = hook.evaluate("Bash", &tool_input_bash("echo hello"));
        assert!(matches!(decision, HookDecision::Block { .. }), "Phase 3 should block Bash");
    }

    #[test]
    fn phase_4_allows_write() {
        let hook = pre_hook(4, &["src/new.rs"]);
        let decision = hook.evaluate("Write", &tool_input_file("src/new.rs"));
        assert!(matches!(decision, HookDecision::Allow), "Phase 4 should allow Write to affected file");
    }

    #[test]
    fn phase_4_warns_write_outside_scope() {
        let hook = pre_hook(4, &["src/main.rs"]);
        let decision = hook.evaluate("Write", &tool_input_file("src/other.rs"));
        assert!(matches!(decision, HookDecision::Warn { .. }), "Phase 4 should warn on out-of-scope Write");
    }

    #[test]
    fn phase_1_blocks_write_outside_scope() {
        let hook = pre_hook(1, &["src/main.rs"]);
        let decision = hook.evaluate("Write", &tool_input_file("src/other.rs"));
        assert!(matches!(decision, HookDecision::Block { .. }), "Phase 1 should block all writes");
    }

    #[test]
    fn dangerous_command_blocked_any_phase() {
        let hook = pre_hook(5, &[]);
        let patterns = vec![
            "rm -rf /tmp/stuff",
            "git push origin main",
            "git merge feature",
            "DROP TABLE users",
            "git reset --hard HEAD",
            "git checkout -- .",
        ];
        for cmd in patterns {
            let decision = hook.evaluate("Bash", &tool_input_bash(cmd));
            assert!(
                matches!(decision, HookDecision::Block { .. }),
                "dangerous command should be blocked: {}",
                cmd
            );
        }
    }

    #[test]
    fn safe_command_allowed_in_phase_4() {
        let hook = pre_hook(4, &[]);
        let decision = hook.evaluate("Bash", &tool_input_bash("cargo test"));
        assert!(matches!(decision, HookDecision::Allow), "safe command should be allowed in Phase 4+");
    }

    #[test]
    fn read_only_tools_always_allowed() {
        let hook = pre_hook(1, &[]);
        for tool in &["Read", "Glob", "Grep", "WebSearch", "WebFetch"] {
            let decision = hook.evaluate(tool, &serde_json::json!({}));
            assert!(matches!(decision, HookDecision::Allow), "{} should always be allowed", tool);
        }
    }

    #[test]
    fn empty_affected_files_allows_any_file_in_phase_4() {
        let hook = pre_hook(4, &[]);
        let decision = hook.evaluate("Write", &tool_input_file("src/anything.rs"));
        // When affected_files is empty, allow all (no contract constraint)
        assert!(matches!(decision, HookDecision::Allow | HookDecision::Warn { .. }));
    }
}

// ---------------------------------------------------------------------------
// PostToolUse Tests
// ---------------------------------------------------------------------------

mod post_hook_tests {
    use super::*;

    #[test]
    fn forbidden_pattern_detected_in_write_content() {
        let hook = PostToolUseHook::new(std::path::PathBuf::from("/tmp"));
        let content = "pub fn handler() { todo!() }";
        let result = hook.check_forbidden_patterns(content);
        assert!(result.is_some(), "should detect todo!() as forbidden pattern");
    }

    #[test]
    fn clean_content_no_warning() {
        let hook = PostToolUseHook::new(std::path::PathBuf::from("/tmp"));
        let content = "pub fn handler() -> String { \"hello\".to_string() }";
        let result = hook.check_forbidden_patterns(content);
        assert!(result.is_none(), "clean content should produce no warning");
    }

    #[test]
    fn edit_counter_increments() {
        let hook = PostToolUseHook::new(std::path::PathBuf::from("/tmp"));
        assert_eq!(hook.increment_edit_counter(), 1);
        assert_eq!(hook.increment_edit_counter(), 2);
        assert_eq!(hook.increment_edit_counter(), 3);
    }

    #[test]
    fn should_run_cargo_check_every_5th_edit() {
        let hook = PostToolUseHook::new(std::path::PathBuf::from("/tmp"));
        for _ in 0..5 {
            hook.increment_edit_counter();
        }
        assert!(!hook.should_run_compilation_check(3)); // phase 3 = no
        assert!(hook.should_run_compilation_check(4)); // phase 4, 5th edit = yes
    }

    #[test]
    fn hook_decision_debug_format() {
        let allow = HookDecision::Allow;
        let block = HookDecision::Block { reason: "test".into() };
        let warn = HookDecision::Warn { message: "test".into() };
        // Should not panic
        let _ = format!("{:?}", allow);
        let _ = format!("{:?}", block);
        let _ = format!("{:?}", warn);
    }
}
