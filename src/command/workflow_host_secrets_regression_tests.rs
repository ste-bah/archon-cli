use super::tests::context;
use super::*;

fn secrets(names: &[&str], values: &[&str]) -> HostSecrets {
    let environment = names
        .iter()
        .zip(values)
        .map(|(n, v)| (n.to_string(), OsString::from(v)))
        .collect();
    HostSecrets::of(&context(names, &[]), &environment)
}

fn typed_string(field: &str) {
    let secret = "credential-canary";
    let mut value = serde_json::json!({"schema_version":1, "report":{},
            "policy_findings":[{"subject":"task", "text":"failure", "remediation_scope":"body"}],
            "operational_error":{"kind":"probe", "text":"failure"}});
    if field == "kind" {
        value["operational_error"][field] = secret.into();
    } else {
        value["policy_findings"][0][field] = secret.into();
    }
    let clean =
        secrets(&["SERVICE_KEY"], &[secret]).envelope(serde_json::from_value(value).unwrap());
    let bytes = serde_json::to_vec(&clean).unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains(secret), "{field}");
    assert_eq!(clean.policy_findings.len(), 1);
    assert!(clean.operational_error.is_some());
}
#[test]
fn typed_subject_is_redacted() {
    typed_string("subject");
}
#[test]
fn typed_source_path_is_redacted() {
    typed_string("source_path");
}
#[test]
fn typed_error_kind_is_redacted() {
    typed_string("kind");
}

fn collision(value: serde_json::Value) {
    let mut report = serde_json::json!({"credential-canary":value, "[REDACTED]":"other", "[REDACTED]#1":"reserved"});
    secrets(&["SERVICE_KEY"], &["credential-canary"]).strings(&mut report);
    assert_eq!(report.as_object().unwrap().len(), 3, "{report}");
    assert!(!report.to_string().contains("credential-canary"));
    assert!(report.as_object().unwrap().values().any(|v| v == "other"));
    assert!(
        report
            .as_object()
            .unwrap()
            .values()
            .any(|v| v == "reserved")
    );
}
#[test]
fn number_collision_is_retained() {
    collision(serde_json::json!(1));
}
#[test]
fn array_collision_is_retained() {
    collision(serde_json::json!(["credential-canary"]));
}
#[test]
fn nested_collision_is_retained() {
    collision(serde_json::json!({"nested":"failure"}));
}

fn password_address(name: &str, value: &str, password: &str) {
    let secret = secrets(&[name], &[value]);
    assert_eq!(secret.text(value), REDACTED, "{name}");
    assert_eq!(secret.text(password), REDACTED, "bare password {name}");
}
#[test]
fn short_database_password_is_redacted() {
    password_address("DATABASE_URL", "postgres://user:small@host/db", "small");
}
#[test]
fn encoded_uri_password_is_redacted() {
    password_address(
        "SERVICE_URI",
        "https://user:escaped%40password@host/api",
        "escaped%40password",
    );
}
#[test]
fn proxy_password_is_redacted() {
    password_address(
        "HTTP_PROXY",
        "http://user:proxy-password@proxy:8080",
        "proxy-password",
    );
}
