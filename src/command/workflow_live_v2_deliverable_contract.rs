use serde_json::Value;

pub(crate) fn verification_command(root: &str, contract: &Value) -> String {
    // The generated verifier is executed by `sh`, so the project root has to be
    // written the way a POSIX shell reads it. A native Windows root
    // (`C:\Users\...`) reaches the shell with its separators consumed as escape
    // characters, and the verifier then probes a path that does not exist —
    // reporting a launch-ish failure instead of the "missing or empty" verdict
    // the caller relies on. Git's sh accepts `C:/Users/...` unchanged.
    let root = &root.replace('\\', "/");
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

/// Resolve a contract path against the project root, `/`-separated.
///
/// These strings are interpolated into a shell command, so a native Windows
/// separator arrives at `sh` as an escape and the verifier probes a mangled
/// path. `is_absolute()` alone also misses `/repo/...` on Windows -- rooted but
/// driveless -- which would then be joined under the root a second time.
fn resolve_contract_path(root: &str, value: Option<&Value>) -> String {
    let value = value.and_then(Value::as_str).unwrap_or_default();
    let path = std::path::Path::new(value);
    if path.is_absolute() || path.has_root() {
        value.replace('\\', "/")
    } else {
        let root = root.trim_end_matches(['/', '\\']).replace('\\', "/");
        let relative = value.trim_start_matches(['/', '\\']).replace('\\', "/");
        format!("{root}/{relative}")
    }
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

const VERIFIER: &str = include_str!("workflow_live_v2_deliverable_verifier.sh");
