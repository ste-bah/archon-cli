//! Free helpers for the document vector store.
//!
//! Split out of `vector_store.rs` to keep that file under the 500-line
//! ceiling after `docs delete` was added.

use super::*;

pub(super) fn hnsw_dump_basename(timestamp: chrono::DateTime<chrono::Utc>) -> String {
    format!(
        "doc-text-{}-{}",
        timestamp.format("%Y%m%dT%H%M%SZ"),
        uuid::Uuid::new_v4().simple()
    )
}

pub(super) fn build_hnsw_index(
    records: &[RawVectorRecord],
    dimension: usize,
) -> Result<Hnsw<'static, f32, DistCosine>> {
    let max_nb_connection = 32;
    let max_layer = 16;
    let ef_construction = 200;
    let hnsw = Hnsw::new(
        max_nb_connection,
        records.len().max(1),
        max_layer,
        ef_construction,
        DistCosine {},
    );
    for record in records {
        anyhow::ensure!(
            record.vector.len() == dimension,
            "vector dimension mismatch for {}: expected {}, got {}",
            record.chunk_id,
            dimension,
            record.vector.len()
        );
        hnsw.insert((&record.vector, record.hnsw_id));
    }
    Ok(hnsw)
}

pub(super) fn encode_vector(vector: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(4 + vector.len() * 4);
    bytes.extend_from_slice(&(vector.len() as u32).to_le_bytes());
    for value in vector {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

pub(super) fn decode_vector(bytes: &[u8]) -> Result<Vec<f32>> {
    anyhow::ensure!(bytes.len() >= 4, "vector payload is too short");
    let dim = u32::from_le_bytes(bytes[0..4].try_into()?) as usize;
    anyhow::ensure!(
        bytes.len() == 4 + dim * 4,
        "vector payload has invalid length"
    );
    let mut vector = Vec::with_capacity(dim);
    for chunk in bytes[4..].chunks_exact(4) {
        vector.push(f32::from_le_bytes(chunk.try_into()?));
    }
    Ok(vector)
}

pub(super) fn validate_provider(provider: &str) -> Result<()> {
    anyhow::ensure!(!provider.is_empty(), "provider must not be empty");
    anyhow::ensure!(
        !provider.contains('/'),
        "provider must not contain the '/' separator"
    );
    Ok(())
}

pub(super) fn validate_optional_provider(provider: Option<&str>) -> Result<()> {
    if let Some(provider) = provider {
        validate_provider(provider)?;
    }
    Ok(())
}

pub(super) fn vector_key(provider: &str, chunk_id: &str) -> Vec<u8> {
    key3(VECTOR_PREFIX, provider, chunk_id)
}

pub(super) fn cache_key(provider: &str, content_hash: &str) -> Vec<u8> {
    key3(CACHE_PREFIX, provider, content_hash)
}

pub(super) fn id_key(provider: &str, chunk_id: &str) -> Vec<u8> {
    key3(ID_PREFIX, provider, chunk_id)
}

pub(super) fn reverse_id_key(provider: &str, hnsw_id: usize) -> Vec<u8> {
    key3(REVERSE_ID_PREFIX, provider, &hnsw_id.to_string())
}

pub(super) fn reverse_id_marker_key(provider: &str) -> Vec<u8> {
    key3(REVERSE_ID_PREFIX, provider, "ready")
}

pub(super) fn vector_prefix(provider: Option<&str>) -> String {
    match provider {
        Some(provider) => format!("{VECTOR_PREFIX}/{provider}/"),
        None => format!("{VECTOR_PREFIX}/"),
    }
}

pub(super) fn cache_prefix(provider: Option<&str>) -> String {
    match provider {
        Some(provider) => format!("{CACHE_PREFIX}/{provider}/"),
        None => format!("{CACHE_PREFIX}/"),
    }
}

pub(super) fn parse_vector_key(key: &[u8]) -> Option<(String, String)> {
    let text = std::str::from_utf8(key).ok()?;
    let mut parts = text.splitn(3, '/');
    (parts.next()? == VECTOR_PREFIX).then_some(())?;
    Some((parts.next()?.to_string(), parts.next()?.to_string()))
}

pub(super) fn key3(prefix: &str, provider: &str, value: &str) -> Vec<u8> {
    format!("{prefix}/{provider}/{value}").into_bytes()
}

pub(super) fn hnsw_id(chunk_id: &str) -> usize {
    let digest = blake3::hash(chunk_id.as_bytes());
    let mut bytes = [0_u8; 8];
    bytes.copy_from_slice(&digest.as_bytes()[..8]);
    u64::from_le_bytes(bytes) as usize
}

pub(super) fn safe_provider(provider: &str) -> String {
    provider
        .bytes()
        .fold(String::with_capacity(provider.len()), |mut safe, byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_') {
                safe.push(byte as char);
            } else if byte == b'~' {
                safe.push_str("~~");
            } else {
                use std::fmt::Write;
                write!(safe, "~{byte:02x}").expect("write to String cannot fail");
            }
            safe
        })
}
