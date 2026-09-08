use archon_core::config::load_config_from;
use serde_json::json;

fn load(text: &str) -> Result<serde_json::Value, String> {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, text).unwrap();
    load_config_from(path)
        .map(|config| serde_json::to_value(config).unwrap())
        .map_err(|error| error.to_string())
}

#[test]
fn audit_limits_survive_real_configuration_loading() {
    let config = load("[workflow.repository_audit]\nattempt_timeout_secs = 7200\ntotal_time_secs = \"unlimited\"\nunexpected_change_refreshes = 12\n").unwrap();
    assert_eq!(config["workflow"]["repository_audit"]["attempt_timeout_secs"], 7200);
    assert_eq!(config["workflow"]["repository_audit"]["total_time_secs"], "unlimited");
    assert_eq!(config["workflow"]["repository_audit"]["unexpected_change_refreshes"], 12);
}

#[test]
fn audit_limits_reject_invalid_values_and_unknown_fields() {
    for field in ["attempt_timeout_secs", "total_time_secs", "unexpected_change_refreshes"] {
        for value in ["0", "-1", "1.5", "\"infinite\"", "true", "9223372036854775807"] {
            let error = load(&format!("[workflow.repository_audit]\n{field} = {value}\n"))
                .expect_err("invalid limits must not silently become defaults");
            assert!(error.contains(field), "{field}: {error}");
        }
    }
    assert!(load("[workflow.repository_audit]\ntotal_time_sec = 3\n").is_err());
}

#[test]
fn every_audit_dimension_accepts_explicit_unlimited() {
    let config = load("[workflow.repository_audit]\nattempt_timeout_secs = \"unlimited\"\ntotal_time_secs = \"unlimited\"\nunexpected_change_refreshes = \"unlimited\"\n").unwrap();
    assert_eq!(config["workflow"]["repository_audit"], json!({
        "attempt_timeout_secs":"unlimited", "total_time_secs":"unlimited",
        "unexpected_change_refreshes":"unlimited"
    }));
}

#[test]
fn layered_unlimited_overrides_finite_without_disabling_inheritance() {
    use archon_core::{config::AuditLimit, config_layers::load_layered_config};
    let dir = tempfile::tempdir().unwrap();
    let user = dir.path().join("user.toml");
    let project = dir.path().join("project");
    std::fs::create_dir_all(project.join(".archon")).unwrap();
    std::fs::write(&user, "[workflow.generated]\nhost_call_timeout_secs=7200\n[workflow.repository_audit]\ntotal_time_secs=600\nunexpected_change_refreshes=3\n").unwrap();
    std::fs::write(project.join(".archon/config.toml"), "[workflow.repository_audit]\ntotal_time_secs=\"unlimited\"\nunexpected_change_refreshes=12\n").unwrap();
    let config = load_layered_config(Some(&user), &project, None, None).unwrap();
    let policy = config.workflow.repository_audit.resolve(config.workflow.generated.host_call_timeout_secs);
    assert_eq!(policy.attempt_timeout_secs, AuditLimit::Finite(7200));
    assert_eq!(policy.total_time_secs, AuditLimit::Unlimited);
    assert_eq!(policy.unexpected_change_refreshes, AuditLimit::Finite(12));
    assert_eq!(policy.attempt_timeout_source, "workflow.generated.host_call_timeout_secs");
}

#[test]
fn finite_time_above_one_hour_is_not_clamped() {
    let config = load("[workflow.repository_audit]\ntotal_time_secs=28800\n").unwrap();
    assert_eq!(config["workflow"]["repository_audit"]["total_time_secs"], 28800);
}

#[test]
fn malformed_audit_layer_is_not_skipped() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join(".archon")).unwrap();
    let path=dir.path().join(".archon/config.toml");
    std::fs::write(&path,"[workflow.repository_audit]\ntotal_time_secs = [\n").unwrap();
    assert!(archon_core::config_layers::load_layered_config(None,dir.path(),None,None).is_err(),"invalid audit policy was silently skipped");
}

#[test]
fn invalid_layered_audit_value_names_its_source_file_and_key() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join(".archon")).unwrap();
    let path = dir.path().join(".archon/config.toml");
    std::fs::write(&path,"[workflow.repository_audit]\ntotal_time_secs = 0\n").unwrap();
    let error = archon_core::config_layers::load_layered_config(None,dir.path(),None,None).unwrap_err().to_string();
    assert!(error.contains(path.to_str().unwrap()), "invalid policy lost source attribution: {error}");
    assert!(error.contains("total_time_secs"), "invalid policy lost key attribution: {error}");
}

#[test]
fn layered_policy_retains_effective_source_and_inheritance() {
    let dir = tempfile::tempdir().unwrap();
    let user = dir.path().join("user.toml");
    let project = dir.path().join("project");
    std::fs::create_dir_all(project.join(".archon")).unwrap();
    std::fs::write(&user,"[workflow.generated]\nhost_call_timeout_secs=7200\n[workflow.repository_audit]\ntotal_time_secs=600\n").unwrap();
    let local = project.join(".archon/config.toml");
    std::fs::write(&local,"[workflow.repository_audit]\ntotal_time_secs=\"unlimited\"\n").unwrap();
    let config = archon_core::config_layers::load_layered_config(Some(&user), &project, None, None).unwrap();
    let resolved = serde_json::to_value(config.workflow.repository_audit.resolve(config.workflow.generated.host_call_timeout_secs)).unwrap();
    assert_eq!(resolved["sources"]["attempt_timeout_secs"]["path"],user.to_str().unwrap());
    assert_eq!(resolved["sources"]["total_time_secs"]["path"],local.to_str().unwrap());
    assert_eq!(resolved["sources"]["attempt_timeout_secs"]["key"],"workflow.generated.host_call_timeout_secs");
}
