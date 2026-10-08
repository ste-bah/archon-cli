const SOURCE: &str = include_str!("../src/hooks/executor_tests.rs");
#[test]
fn unix_probe_does_not_spawn_outside_helper() {
    assert!(!SOURCE.contains("std::process::Command::new(\"kill\")"));
}
#[test]
fn child_test_binary_uses_spawn_helper() {
    assert!(!SOURCE.contains("std::process::Command::new(std::env::current_exe()"));
}
#[test]
fn windows_probe_uses_spawn_helper() {
    assert!(!SOURCE.contains("std::process::Command::new(powershell_exe()"));
}
