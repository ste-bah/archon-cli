use serde_json::Value;

pub(crate) fn verification_command(root: &str, contract: &Value) -> String {
    let root_literal = serde_json::to_string(root).expect("project root JSON");
    let contract_json = serde_json::to_string(contract).expect("deliverable contract JSON");
    let contract_literal =
        serde_json::to_string(&contract_json).expect("deliverable contract JSON literal");
    let verifier = VERIFIER
        .replace("__PROJECT_ROOT__", &root_literal)
        .replace("__CONTRACT_JSON__", &contract_literal);
    let Some(command) = typed_verification_command(root, contract) else {
        return verifier;
    };
    format!("{command}\n{verifier}")
}

pub(super) fn typed_verification_command(root: &str, contract: &Value) -> Option<String> {
    let command = contract.get("typed_verifier_command")?.as_str()?.trim();
    if command.is_empty() {
        return None;
    }
    let artifact = resolve_contract_path(root, contract.get("artifact_path"));
    let registry = resolve_contract_path(root, contract.get("registry_path"));
    Some(
        command
            .replace("{artifact_path}", &shell_quote(&artifact))
            .replace("{registry_path}", &shell_quote(&registry)),
    )
}

fn resolve_contract_path(root: &str, value: Option<&Value>) -> String {
    let value = value.and_then(Value::as_str).unwrap_or_default();
    let path = std::path::Path::new(value);
    if path.is_absolute() {
        value.to_string()
    } else {
        std::path::Path::new(root).join(path).display().to_string()
    }
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

const VERIFIER: &str = include_str!("workflow_live_v2_deliverable_verifier.sh");
