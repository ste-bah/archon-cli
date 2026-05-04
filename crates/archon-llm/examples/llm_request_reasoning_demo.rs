use archon_llm::provider::LlmRequest;
use sha2::{Digest, Sha256};

fn sha256_hex(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    hex::encode(hasher.finalize())
}

fn main() {
    let input_blob = "r".repeat(4096);
    let request = LlmRequest::default().with_reasoning_encrypted(Some(input_blob.clone()));
    let recovered_blob = request.reasoning_encrypted.as_deref().unwrap_or_default();

    let input_hash = sha256_hex(&input_blob);
    let recovered_hash = sha256_hex(recovered_blob);

    println!("input_sha256={input_hash}");
    println!("recovered_sha256={recovered_hash}");
    assert_eq!(input_hash, recovered_hash);
    println!("OK: blob preserved");
}
