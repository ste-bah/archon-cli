use std::path::Path;

use archon_permissions::checker::PermissionChecker;
use archon_permissions::classifier::{CommandClass, classify_command};
use archon_permissions::mode::{PermissionDecision, PermissionMode};
use archon_permissions::rules::{PathDecision, RuleSet, check_write_path};

fn no_overrides() -> (Vec<String>, Vec<String>, Vec<String>) {
    (vec![], vec![], vec![])
}

// -----------------------------------------------------------------------
// Command classification -- safe
// -----------------------------------------------------------------------

#[test]
fn ls_is_safe() {
    let (s, r, d) = no_overrides();
    assert_eq!(classify_command("ls", &s, &r, &d), CommandClass::Safe);
    assert_eq!(
        classify_command("ls -la /tmp", &s, &r, &d),
        CommandClass::Safe
    );
}

#[test]
fn git_status_is_safe() {
    let (s, r, d) = no_overrides();
    assert_eq!(
        classify_command("git status", &s, &r, &d),
        CommandClass::Safe
    );
}

#[test]
fn git_log_is_safe() {
    let (s, r, d) = no_overrides();
    assert_eq!(
        classify_command("git log --oneline", &s, &r, &d),
        CommandClass::Safe
    );
}

#[test]
fn cargo_test_is_safe() {
    let (s, r, d) = no_overrides();
    assert_eq!(
        classify_command("cargo test", &s, &r, &d),
        CommandClass::Safe
    );
}

#[test]
fn echo_is_safe() {
    let (s, r, d) = no_overrides();
    assert_eq!(
        classify_command("echo hello", &s, &r, &d),
        CommandClass::Safe
    );
}

#[test]
fn grep_is_safe() {
    let (s, r, d) = no_overrides();
    assert_eq!(
        classify_command("grep -r pattern .", &s, &r, &d),
        CommandClass::Safe
    );
}

// -----------------------------------------------------------------------
// Command classification -- risky
// -----------------------------------------------------------------------

#[test]
fn git_commit_is_risky() {
    let (s, r, d) = no_overrides();
    assert_eq!(
        classify_command("git commit -m 'msg'", &s, &r, &d),
        CommandClass::Risky
    );
}

#[test]
fn npm_install_is_risky() {
    let (s, r, d) = no_overrides();
    assert_eq!(
        classify_command("npm install express", &s, &r, &d),
        CommandClass::Risky
    );
}

#[test]
fn cargo_build_is_risky() {
    let (s, r, d) = no_overrides();
    assert_eq!(
        classify_command("cargo build --release", &s, &r, &d),
        CommandClass::Risky
    );
}

#[test]
fn unknown_command_is_risky() {
    let (s, r, d) = no_overrides();
    assert_eq!(
        classify_command("mycustomtool --flag", &s, &r, &d),
        CommandClass::Risky
    );
}

// -----------------------------------------------------------------------
// Command classification -- dangerous
// -----------------------------------------------------------------------

#[test]
fn rm_rf_is_dangerous() {
    let (s, r, d) = no_overrides();
    assert_eq!(
        classify_command("rm -rf /", &s, &r, &d),
        CommandClass::Dangerous
    );
}

#[test]
fn rm_r_is_dangerous() {
    let (s, r, d) = no_overrides();
    assert_eq!(
        classify_command("rm -r /tmp/stuff", &s, &r, &d),
        CommandClass::Dangerous
    );
}

#[test]
fn sudo_is_dangerous() {
    let (s, r, d) = no_overrides();
    assert_eq!(
        classify_command("sudo apt install pkg", &s, &r, &d),
        CommandClass::Dangerous
    );
}

#[test]
fn git_push_is_dangerous() {
    let (s, r, d) = no_overrides();
    assert_eq!(
        classify_command("git push origin main", &s, &r, &d),
        CommandClass::Dangerous
    );
}

#[test]
fn git_push_force_is_dangerous() {
    let (s, r, d) = no_overrides();
    assert_eq!(
        classify_command("git push --force origin main", &s, &r, &d),
        CommandClass::Dangerous
    );
}

#[test]
fn git_reset_hard_is_dangerous() {
    let (s, r, d) = no_overrides();
    assert_eq!(
        classify_command("git reset --hard HEAD~1", &s, &r, &d),
        CommandClass::Dangerous
    );
}

// -----------------------------------------------------------------------
// Pipe chain classification
// -----------------------------------------------------------------------

#[test]
fn pipe_to_rm_is_dangerous() {
    let (s, r, d) = no_overrides();
    assert_eq!(
        classify_command("echo foo | rm -rf /", &s, &r, &d),
        CommandClass::Dangerous
    );
}

#[test]
fn pipe_to_sudo_is_dangerous() {
    let (s, r, d) = no_overrides();
    assert_eq!(
        classify_command("echo foo | sudo rm /etc/passwd", &s, &r, &d),
        CommandClass::Dangerous
    );
}

#[test]
fn safe_pipe_is_safe() {
    let (s, r, d) = no_overrides();
    assert_eq!(
        classify_command("cat file.txt | grep pattern | wc -l", &s, &r, &d),
        CommandClass::Safe
    );
}

// -----------------------------------------------------------------------
// bash -c quoted commands
// -----------------------------------------------------------------------

#[test]
fn bash_c_rm_rf_is_dangerous() {
    let (s, r, d) = no_overrides();
    assert_eq!(
        classify_command(r#"bash -c "rm -rf /""#, &s, &r, &d),
        CommandClass::Dangerous
    );
}

// -----------------------------------------------------------------------
// User config overrides
// -----------------------------------------------------------------------

#[test]
fn user_override_makes_custom_command_safe() {
    let safe = vec!["mycustomtool".to_string()];
    let (_, r, d) = no_overrides();
    assert_eq!(
        classify_command("mycustomtool --flag", &safe, &r, &d),
        CommandClass::Safe
    );
}

#[test]
fn user_override_makes_safe_command_dangerous() {
    let dangerous = vec!["ls".to_string()];
    let (s, r, _) = no_overrides();
    assert_eq!(
        classify_command("ls -la", &s, &r, &dangerous),
        CommandClass::Dangerous
    );
}

// -----------------------------------------------------------------------
// Path rules
// -----------------------------------------------------------------------

#[test]
fn write_inside_project_allowed() {
    let result = check_write_path(
        Path::new("/home/user/project/src/main.rs"),
        Path::new("/home/user/project"),
        &[],
        &[],
    );
    assert_eq!(result, PathDecision::Allow);
}

#[test]
fn write_outside_project_needs_permission() {
    let result = check_write_path(
        Path::new("/etc/config.toml"),
        Path::new("/home/user/project"),
        &[],
        &[],
    );
    assert_eq!(result, PathDecision::NeedsPermission);
}

#[test]
fn write_to_deny_path_is_denied() {
    let result = check_write_path(
        Path::new("/etc/passwd"),
        Path::new("/home/user/project"),
        &[],
        &["/etc/*".to_string()],
    );
    assert_eq!(result, PathDecision::Deny);
}

#[test]
fn write_to_allow_path_outside_project() {
    let result = check_write_path(
        Path::new("/tmp/output.txt"),
        Path::new("/home/user/project"),
        &["/tmp/*".to_string()],
        &[],
    );
    assert_eq!(result, PathDecision::Allow);
}

// -----------------------------------------------------------------------
// PermissionChecker modes (updated for 6-mode API)
// -----------------------------------------------------------------------

#[test]
fn default_mode_needs_permission_for_write() {
    let checker = PermissionChecker::new(PermissionMode::Default, RuleSet::empty());
    let result = checker.check("Write", "write a file", "/tmp/foo");
    assert!(
        matches!(result, PermissionDecision::NeedsPermission(_)),
        "Default mode should need permission for Write"
    );
}

#[test]
fn bypass_mode_always_allows() {
    let checker = PermissionChecker::new(PermissionMode::BypassPermissions, RuleSet::empty());
    let result = checker.check("Bash", "run rm -rf /", "rm -rf /");
    assert_eq!(result, PermissionDecision::Allow);
}

#[test]
fn auto_mode_needs_permission_for_write() {
    let checker = PermissionChecker::new(PermissionMode::Auto, RuleSet::empty());
    let result = checker.check("Write", "write a file", "/tmp/foo");
    assert!(
        matches!(result, PermissionDecision::NeedsPermission(_)),
        "auto mode should need permission for Write"
    );
}

// -----------------------------------------------------------------------
// Edge cases
// -----------------------------------------------------------------------

#[test]
fn empty_command_is_risky() {
    let (s, r, d) = no_overrides();
    assert_eq!(classify_command("", &s, &r, &d), CommandClass::Risky);
}

#[test]
fn whitespace_only_command_is_risky() {
    let (s, r, d) = no_overrides();
    assert_eq!(classify_command("   ", &s, &r, &d), CommandClass::Risky);
}
