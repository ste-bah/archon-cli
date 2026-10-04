use super::*;

#[test]
fn round2_typed_errors_preserve_diagnostic_words() {
    let _registry = archon_observability::secret_values::scoped_registry_for_tests();
    let secret = "typed-errors-own-value";
    let secrets = SecretValues::new([secret]);
    let words = "unexpected token at line 3; check secret password authorization credentials api_key and retry";
    let text = format!("{words}; {secret}");
    let errors = [
        McpError::ConfigParse(text.clone()),
        McpError::ConfigIo(std::io::Error::other(text.clone())),
        McpError::Transport(text.clone()),
        McpError::InitFailed {
            server: text.clone(),
            reason: text.clone(),
        },
        McpError::ToolCallFailed(text.clone()),
        McpError::ServerNotFound(text.clone()),
        McpError::ServerNotReady(text.clone(), crate::types::ServerState::Crashed),
        McpError::Shutdown(text.clone()),
        McpError::Json(<serde_json::Error as serde::de::Error>::custom(
            text.clone(),
        )),
        McpError::MaxRestartsExceeded(text),
    ];
    for error in errors {
        let kind = std::mem::discriminant(&error);
        let clean = error.redacted(&secrets);
        assert_eq!(std::mem::discriminant(&clean), kind);
        let clean = clean.to_string();
        assert!(clean.contains(words), "diagnostic damaged: {clean}");
        assert!(!clean.contains(secret), "credential leaked: {clean}");
    }
}
