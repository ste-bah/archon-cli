use sha2::{Digest, Sha256};

/// Compute the Evidence Engine chain hash.
///
/// Rule: SHA256(parent_hashes | operation | input_hashes | output_hash | tool
/// | model | parameters_json). Null separators keep adjacent fields from
/// accidentally colliding.
pub fn chain_hash(
    parent_hashes: &[String],
    operation: &str,
    input_hashes: &[String],
    output_hash: &str,
    tool: Option<&str>,
    model: Option<&str>,
    parameters_json: &serde_json::Value,
) -> String {
    chain_hash_from_str(
        parent_hashes,
        operation,
        input_hashes,
        output_hash,
        tool,
        model,
        &parameters_json.to_string(),
    )
}

pub fn chain_hash_from_str(
    parent_hashes: &[String],
    operation: &str,
    input_hashes: &[String],
    output_hash: &str,
    tool: Option<&str>,
    model: Option<&str>,
    parameters_json: &str,
) -> String {
    let mut hasher = Sha256::new();
    update_many(&mut hasher, parent_hashes);
    update_one(&mut hasher, operation);
    update_many(&mut hasher, input_hashes);
    update_one(&mut hasher, output_hash);
    update_one(&mut hasher, tool.unwrap_or(""));
    update_one(&mut hasher, model.unwrap_or(""));
    update_one(&mut hasher, parameters_json);
    hex::encode(hasher.finalize())
}

fn update_many(hasher: &mut Sha256, values: &[String]) {
    for value in values {
        update_one(hasher, value);
    }
}

fn update_one(hasher: &mut Sha256, value: &str) {
    hasher.update(value.as_bytes());
    hasher.update([0]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chain_hash_is_deterministic() {
        let first = chain_hash_from_str(
            &["parent".into()],
            "ocr",
            &["input".into()],
            "output",
            Some("tesseract"),
            Some("local"),
            "{}",
        );
        let second = chain_hash_from_str(
            &["parent".into()],
            "ocr",
            &["input".into()],
            "output",
            Some("tesseract"),
            Some("local"),
            "{}",
        );
        assert_eq!(first, second);
    }

    #[test]
    fn chain_hash_changes_when_parent_changes() {
        let first = chain_hash_from_str(&["a".into()], "op", &[], "out", None, None, "{}");
        let second = chain_hash_from_str(&["b".into()], "op", &[], "out", None, None, "{}");
        assert_ne!(first, second);
    }

    #[test]
    fn chain_hash_uses_separators() {
        let joined = chain_hash_from_str(&[], "ab", &[], "c", None, None, "{}");
        let split = chain_hash_from_str(&[], "a", &[], "bc", None, None, "{}");
        assert_ne!(joined, split);
    }
}
