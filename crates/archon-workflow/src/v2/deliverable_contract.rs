use serde_json::Value;

pub fn verification_command(root: &str, contract: &Value) -> String {
    if let Some(reason) = template_binding_failure(contract) {
        return fail_closed_command(&reason);
    }
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

pub fn typed_verification_command(root: &str, contract: &Value) -> Option<String> {
    if template_binding_failure(contract).is_some() {
        return None;
    }
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

/// Why a declared deliverable contract cannot be verified as written, or `None`
/// when it can.
///
/// # D3: a template placeholder is not a filename
///
/// `<dataset-id>` never names a file. Joined to the project root verbatim it
/// produces a path that cannot exist, so the gate can never pass; expanded to a
/// glob with no declared floor it matches zero files and the gate can never
/// fail. Both readings are dishonest, and the second is the exact shape of
/// prior-run finding F4 (`wf-ee4a92fc`): an artifact reported present against a
/// wildcard path on the strength of "observed or contract-required" rather than
/// a file anyone looked at. Neither is used here — an unexpanded token that
/// nothing binds is a contract defect, reported as one, naming the token.
///
/// # What counts as bound
///
/// [`archon_workflow::task_universe::WorkflowV2DeliverableContract`] already
/// carries the one-contract-many-instances fields, so binding is a declaration
/// the author can already make. A templated `artifact_path` is checkable when
/// either:
///
/// - **a source collection is declared** — `instance_artifact_field` plus a
///   source path (`instance_source_path`, else `registry_path`) and its records
///   field (`instance_source_records_field`, else `registry_records_field`).
///   Each instance is then named by an entry, so a missing one is detectable; or
/// - **`min_instances >= 1`** — glob expansion with a floor. Weaker (it cannot
///   see an instance nobody wrote) but it is a claim that can fail, which zero
///   cannot.
///
/// Everything else fails closed:
///
/// - a templated `registry_path` or `instance_source_path`, because the verifier
///   opens those literally and no instance machinery applies to them;
/// - a templated `artifact_path` under `required_universe`, because a
///   required-universe contract is one enumerated file and the instance branch
///   is not reached;
/// - a templated `artifact_path` with a `typed_verifier_command`, because a
///   typed verifier is handed one concrete path and cannot expand it.
fn template_binding_failure(contract: &Value) -> Option<String> {
    if let Some(failure) = shell_template_failure(contract) {
        return Some(failure);
    }
    for key in ["registry_path", "instance_source_path"] {
        let Some(path) = contract_field(contract, key) else {
            continue;
        };
        let tokens = template_tokens(&path);
        if !tokens.is_empty() {
            return Some(format!(
                "deliverable contract {key} '{path}' carries an unexpanded {}; the verifier \
                 opens this path literally and no instance binding expands it, so declare a \
                 concrete path",
                rendered_tokens(&tokens)
            ));
        }
    }
    let artifact_path = contract_field(contract, "artifact_path").unwrap_or_default();
    let tokens = template_tokens(&artifact_path);
    if tokens.is_empty() {
        return None;
    }
    if contract.get("required_universe") == Some(&Value::Bool(true)) {
        return Some(format!(
            "deliverable contract declares required_universe against artifact_path \
             '{artifact_path}', which carries an unexpanded {}; a required-universe contract \
             names one enumerated artifact, so no instance binding applies to it",
            rendered_tokens(&tokens)
        ));
    }
    if contract_field(contract, "typed_verifier_command").is_some() {
        return Some(format!(
            "deliverable contract declares typed_verifier_command against artifact_path \
             '{artifact_path}', which carries an unexpanded {}; a typed verifier is handed one \
             concrete path and cannot expand it",
            rendered_tokens(&tokens)
        ));
    }
    if source_collection_is_bound(contract) || declared_instance_floor(contract) >= 1 {
        return None;
    }
    Some(format!(
        "deliverable contract artifact_path '{artifact_path}' carries an unexpanded {} with no \
         instance binding, so the gate can neither pass nor fail against it. Declare \
         instance_artifact_field together with instance_source_path (or registry_path) and \
         instance_source_records_field (or registry_records_field) so every instance is named by \
         a source entry, or declare min_instances >= 1 so the expansion is a claim that can fail.",
        rendered_tokens(&tokens)
    ))
}

fn source_collection_is_bound(contract: &Value) -> bool {
    contract_field(contract, "instance_artifact_field").is_some()
        && (contract_field(contract, "instance_source_path").is_some()
            || contract_field(contract, "registry_path").is_some())
        && (contract_field(contract, "instance_source_records_field").is_some()
            || contract_field(contract, "registry_records_field").is_some())
}

fn declared_instance_floor(contract: &Value) -> u64 {
    contract
        .get("min_instances")
        .and_then(Value::as_u64)
        .unwrap_or(0)
}

fn contract_field(contract: &Value, key: &str) -> Option<String> {
    let text = contract.get(key)?.as_str()?.trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// A shell-style `${...}` in any declared contract path, or `None`.
///
/// # Issue #168: `${PROJECT_ROOT}` is not a directory name
///
/// A run recorded an `artifact_path` of the shape
/// `${PROJECT_ROOT}/.archon/<area>/data/<set>/${DATASET_ID}/${VERSION}/out.json`
/// — the observed path itself is not reproduced here, because a fixture's
/// domain vocabulary in generic runtime source is what the D52/D75 genericity
/// gate exists to stop, and a comment fossilizes an assumption just as
/// effectively as code does.
///
/// Nothing in this engine expands `${...}`: the instance
/// machinery below rewrites `<...>` only, `glob_instances` wildcards `<...>`
/// only, and a source-bound contract names its instances from registry entries.
/// So a `${...}` path is unconditionally unbindable — unlike `<...>`, there is
/// no declaration an author can add that makes it checkable.
///
/// The alternative is worse than a refusal. Handed to a shell, an unset
/// `${PROJECT_ROOT}` expands to nothing and the "absolute" path silently
/// becomes relative to whatever directory the process is in; `${DATASET_ID}`
/// leaves a literal `${DATASET_ID}` segment in a path someone may then create.
/// Expand or refuse — and this engine cannot expand, so it refuses, naming the
/// token.
fn shell_template_failure(contract: &Value) -> Option<String> {
    for key in [
        "artifact_path",
        "registry_path",
        "instance_source_path",
        "payload_path",
    ] {
        let Some(path) = contract_field(contract, key) else {
            continue;
        };
        let tokens = shell_tokens(&path);
        if !tokens.is_empty() {
            return Some(format!(
                "deliverable contract {key} '{path}' carries an unexpanded shell {}; nothing in \
                 the verifier expands `${{...}}`, and an unset variable would expand to nothing \
                 and silently make the path relative, so declare a concrete path",
                rendered_tokens(&tokens)
            ));
        }
    }
    None
}

/// Every `${...}` span in a declared path, in the order written.
fn shell_tokens(value: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut rest = value;
    while let Some(open) = rest.find("${") {
        let after = &rest[open..];
        let Some(close) = after.find('}') else {
            break;
        };
        tokens.push(after[..=close].to_string());
        rest = &after[close + 1..];
    }
    tokens
}

/// Every `<...>` span in a declared path, in the order written.
fn template_tokens(value: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut rest = value;
    while let Some(open) = rest.find('<') {
        let after = &rest[open..];
        let Some(close) = after.find('>') else {
            break;
        };
        tokens.push(after[..=close].to_string());
        rest = &after[close + 1..];
    }
    tokens
}

fn rendered_tokens(tokens: &[String]) -> String {
    match tokens {
        [single] => format!("template token {single}"),
        many => format!("set of template tokens {}", many.join(" ")),
    }
}

/// A verifier that reports the contract defect and exits non-zero.
///
/// Emitted in place of the real verifier, so an unverifiable contract fails on
/// the gate rather than being skipped by it.
fn fail_closed_command(reason: &str) -> String {
    format!("printf '%s\\n' {} >&2\nexit 1", shell_quote(reason))
}

/// Resolve a contract path against the project root, `/`-separated.
///
/// These strings are interpolated into a shell command, so a native Windows
/// separator arrives at `sh` as an escape and the verifier probes a mangled
/// path. `is_absolute()` alone also misses `/repo/...` on Windows -- rooted but
/// driveless -- which would then be joined under the root a second time.
///
/// Template placeholders never reach here: [`template_binding_failure`] runs
/// before either caller, so a `<...>` path is rejected rather than joined.
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

const VERIFIER: &str = include_str!("deliverable_verifier.sh");
