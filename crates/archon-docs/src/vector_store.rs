use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result};
use hnsw_rs::prelude::{AnnT, DistCosine, Hnsw};
use rust_rocksdb::{DB, Options, WriteBatch};
use serde::{Deserialize, Serialize};

mod cache;
mod config;
mod generation;
mod hnsw_ids;
mod persisted_hnsw;
mod search;
#[cfg(test)]
mod test_hooks;

pub use config::default_store_dir;

use hnsw_ids::chunk_id_for_hnsw_id;
#[cfg(test)]
use hnsw_ids::chunk_ids_by_hnsw_id;
#[cfg(test)]
use hnsw_ids::{
    hit_resolution_probes, reset_hit_resolution_probes, reset_reverse_scan_probes,
    reverse_scan_probes,
};

const VECTOR_PREFIX: &str = "vec";
const CACHE_PREFIX: &str = "cache";
const ID_PREFIX: &str = "id";
const REVERSE_ID_PREFIX: &str = "rid";

#[cfg(test)]
fn persisted_hnsw_load_count() -> usize {
    persisted_hnsw::load_count()
}
#[derive(Clone, Debug)]
pub struct VectorWrite<'a> {
    pub chunk_id: &'a str,
    pub content_hash: &'a str,
    pub provider: &'a str,
    pub embedding: &'a [f32],
}

#[derive(Clone, Debug, Default)]
pub struct VectorStoreStats {
    pub raw_vectors: usize,
    pub cache_entries: usize,
}

#[derive(Clone, Debug)]
pub struct RawVectorRecord {
    pub chunk_id: String,
    pub provider: String,
    pub vector: Vec<f32>,
    pub hnsw_id: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HnswManifest {
    pub provider: String,
    pub dimension: usize,
    pub vector_count: usize,
    pub dump_basename: String,
    pub created_at: String,
    #[serde(default)]
    pub provider_generation: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct HnswSearchHit {
    pub chunk_id: String,
    pub distance: f32,
}

pub struct DocVectorStore {
    db: DB,
    root: PathBuf,
    snapshot_fence: Mutex<()>,
}

impl DocVectorStore {
    pub fn acquire_default() -> Result<std::sync::Arc<Self>> {
        Self::acquire(default_store_dir())
    }

    pub fn acquire(path: impl AsRef<Path>) -> Result<std::sync::Arc<Self>> {
        cache::acquire(path.as_ref())
    }

    pub fn open_default() -> Result<Self> {
        Self::open(default_store_dir())
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let root = path.as_ref().to_path_buf();
        std::fs::create_dir_all(&root)
            .with_context(|| format!("create vector store dir {}", root.display()))?;
        let mut options = Options::default();
        options.create_if_missing(true);
        options.set_max_open_files(256);
        options.increase_parallelism(config::num_parallelism());
        let db = DB::open(&options, &root)
            .with_context(|| format!("open RocksDB vector store {}", root.display()))?;
        Ok(Self {
            db,
            root,
            snapshot_fence: Mutex::new(()),
        })
    }

    pub fn put_vectors(&self, rows: &[VectorWrite<'_>]) -> Result<usize> {
        for row in rows {
            validate_provider(row.provider)?;
        }
        if rows.is_empty() {
            return Ok(0);
        }
        let _snapshot_fence = self
            .snapshot_fence
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        for provider in rows.iter().map(|row| row.provider).collect::<HashSet<_>>() {
            if self
                .db
                .get_pinned(reverse_id_marker_key(provider))?
                .is_none()
            {
                self.migrate_reverse_hnsw_ids(provider)?;
            }
        }
        let mut batch = WriteBatch::default();
        let mut written_keys = HashSet::new();
        let mut mutated_providers = HashSet::new();
        for row in rows {
            if row.embedding.is_empty() {
                continue;
            }
            let bytes = encode_vector(row.embedding);
            let vector_key = vector_key(row.provider, row.chunk_id);
            batch.put(&vector_key, &bytes);
            let hnsw_id = hnsw_id(row.chunk_id);
            batch.put(id_key(row.provider, row.chunk_id), hnsw_id.to_be_bytes());
            batch.put(
                reverse_id_key(row.provider, hnsw_id),
                row.chunk_id.as_bytes(),
            );
            batch.put(reverse_id_marker_key(row.provider), []);
            if !row.content_hash.is_empty() {
                batch.put(cache_key(row.provider, row.content_hash), &bytes);
            }
            written_keys.insert(vector_key);
            mutated_providers.insert(row.provider);
        }
        if written_keys.is_empty() {
            return Ok(0);
        }
        for provider in mutated_providers {
            let generation = generation::next(&self.db, provider)?;
            batch.put(generation::key(provider), generation::encode(generation));
        }
        self.db
            .write(batch)
            .context("write raw vectors to RocksDB vector store")?;
        Ok(written_keys.len())
    }

    pub fn has_vector(&self, provider: &str, chunk_id: &str) -> Result<bool> {
        validate_provider(provider)?;
        self.db
            .get_pinned(vector_key(provider, chunk_id))
            .context("read vector presence from RocksDB")
            .map(|value| value.is_some())
    }

    pub fn cached_embedding(&self, provider: &str, content_hash: &str) -> Result<Option<Vec<f32>>> {
        validate_provider(provider)?;
        if content_hash.is_empty() {
            return Ok(None);
        }
        self.db
            .get(cache_key(provider, content_hash))
            .context("read cached vector from RocksDB")?
            .map(|bytes| decode_vector(&bytes))
            .transpose()
    }

    pub fn count_vectors(&self, provider: Option<&str>) -> Result<usize> {
        validate_optional_provider(provider)?;
        self.count_prefix(&vector_prefix(provider))
    }

    pub fn stats(&self, provider: Option<&str>) -> Result<VectorStoreStats> {
        validate_optional_provider(provider)?;
        Ok(VectorStoreStats {
            raw_vectors: self.count_prefix(&vector_prefix(provider))?,
            cache_entries: self.count_prefix(&cache_prefix(provider))?,
        })
    }

    pub fn build_hnsw(
        &self,
        provider: &str,
        dimension: usize,
        limit: Option<usize>,
    ) -> Result<HnswManifest> {
        validate_provider(provider)?;
        let _snapshot_fence = self
            .snapshot_fence
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let records = self.iter_records(Some(provider), limit)?;
        let provider_generation = generation::current(&self.db, provider)?;
        anyhow::ensure!(
            !records.is_empty(),
            "no raw vectors found for provider {provider}"
        );
        let mut hnsw = build_hnsw_index(&records, dimension)?;
        hnsw.set_searching_mode(true);
        let hnsw_dir = self.hnsw_dir(provider);
        std::fs::create_dir_all(&hnsw_dir)
            .with_context(|| format!("create HNSW dir {}", hnsw_dir.display()))?;
        let basename = format!("doc-text-{}", chrono::Utc::now().format("%Y%m%dT%H%M%SZ"));
        let dump_basename = hnsw
            .file_dump(&hnsw_dir, &basename)
            .context("dump Rust HNSW index")?;
        let manifest = HnswManifest {
            provider: provider.into(),
            dimension,
            vector_count: records.len(),
            dump_basename,
            created_at: chrono::Utc::now().to_rfc3339(),
            provider_generation: Some(provider_generation),
        };
        self.write_hnsw_manifest(provider, &manifest)?;
        persisted_hnsw::clear_dir(&hnsw_dir);
        self.remove_superseded_hnsw_dumps(provider, &manifest.dump_basename)?;
        Ok(manifest)
    }

    pub fn latest_hnsw_manifest(&self, provider: &str) -> Result<Option<HnswManifest>> {
        validate_provider(provider)?;
        let path = self.hnsw_manifest_path(provider);
        if !path.exists() {
            return Ok(None);
        }
        let bytes = std::fs::read(&path)
            .with_context(|| format!("read HNSW manifest {}", path.display()))?;
        let manifest: HnswManifest =
            serde_json::from_slice(&bytes).context("parse HNSW manifest")?;
        anyhow::ensure!(
            manifest.provider == provider,
            "HNSW manifest provider mismatch: expected {provider}, got {}",
            manifest.provider
        );
        Ok(Some(manifest))
    }

    fn iter_records(
        &self,
        provider: Option<&str>,
        limit: Option<usize>,
    ) -> Result<Vec<RawVectorRecord>> {
        let prefix = vector_prefix(provider);
        let mut records = Vec::new();
        for item in self.db.prefix_iterator(prefix.as_bytes()) {
            let (key, value) = item.context("iterate RocksDB vector records")?;
            if !key.starts_with(prefix.as_bytes()) {
                break;
            }
            let Some((provider, chunk_id)) = parse_vector_key(&key) else {
                continue;
            };
            records.push(RawVectorRecord {
                hnsw_id: hnsw_id(&chunk_id),
                chunk_id,
                provider,
                vector: decode_vector(&value)?,
            });
            if limit.is_some_and(|limit| records.len() >= limit) {
                break;
            }
        }
        Ok(records)
    }

    fn count_prefix(&self, prefix: &str) -> Result<usize> {
        let mut count = 0;
        for item in self.db.prefix_iterator(prefix.as_bytes()) {
            let (key, _) = item.context("iterate RocksDB prefix")?;
            if !key.starts_with(prefix.as_bytes()) {
                break;
            }
            count += 1;
        }
        Ok(count)
    }

    fn hnsw_dir(&self, provider: &str) -> PathBuf {
        self.root.join("hnsw").join(safe_provider(provider))
    }

    fn hnsw_manifest_path(&self, provider: &str) -> PathBuf {
        self.hnsw_dir(provider).join("manifest.json")
    }

    fn remove_superseded_hnsw_dumps(&self, provider: &str, current_basename: &str) -> Result<()> {
        let hnsw_dir = self.hnsw_dir(provider);
        for entry in std::fs::read_dir(&hnsw_dir)
            .with_context(|| format!("read HNSW dir {}", hnsw_dir.display()))?
        {
            let path = entry
                .with_context(|| format!("read HNSW dir entry {}", hnsw_dir.display()))?
                .path();
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            let is_dump = name.ends_with(".hnsw.graph") || name.ends_with(".hnsw.data");
            if is_dump && !name.starts_with(&format!("{current_basename}.")) {
                std::fs::remove_file(&path)
                    .with_context(|| format!("remove superseded HNSW dump {}", path.display()))?;
            }
        }
        Ok(())
    }

    fn write_hnsw_manifest(&self, provider: &str, manifest: &HnswManifest) -> Result<()> {
        let path = self.hnsw_manifest_path(provider);
        let bytes = serde_json::to_vec_pretty(manifest)?;
        std::fs::write(&path, bytes)
            .with_context(|| format!("write HNSW manifest {}", path.display()))
    }
}

fn build_hnsw_index(
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

fn encode_vector(vector: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(4 + vector.len() * 4);
    bytes.extend_from_slice(&(vector.len() as u32).to_le_bytes());
    for value in vector {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

fn decode_vector(bytes: &[u8]) -> Result<Vec<f32>> {
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

fn validate_provider(provider: &str) -> Result<()> {
    anyhow::ensure!(!provider.is_empty(), "provider must not be empty");
    anyhow::ensure!(
        !provider.contains('/'),
        "provider must not contain the '/' separator"
    );
    Ok(())
}

fn validate_optional_provider(provider: Option<&str>) -> Result<()> {
    if let Some(provider) = provider {
        validate_provider(provider)?;
    }
    Ok(())
}

fn vector_key(provider: &str, chunk_id: &str) -> Vec<u8> {
    key3(VECTOR_PREFIX, provider, chunk_id)
}

fn cache_key(provider: &str, content_hash: &str) -> Vec<u8> {
    key3(CACHE_PREFIX, provider, content_hash)
}

fn id_key(provider: &str, chunk_id: &str) -> Vec<u8> {
    key3(ID_PREFIX, provider, chunk_id)
}

fn reverse_id_key(provider: &str, hnsw_id: usize) -> Vec<u8> {
    key3(REVERSE_ID_PREFIX, provider, &hnsw_id.to_string())
}

fn reverse_id_marker_key(provider: &str) -> Vec<u8> {
    key3(REVERSE_ID_PREFIX, provider, "ready")
}

fn vector_prefix(provider: Option<&str>) -> String {
    match provider {
        Some(provider) => format!("{VECTOR_PREFIX}/{provider}/"),
        None => format!("{VECTOR_PREFIX}/"),
    }
}

fn cache_prefix(provider: Option<&str>) -> String {
    match provider {
        Some(provider) => format!("{CACHE_PREFIX}/{provider}/"),
        None => format!("{CACHE_PREFIX}/"),
    }
}

fn parse_vector_key(key: &[u8]) -> Option<(String, String)> {
    let text = std::str::from_utf8(key).ok()?;
    let mut parts = text.splitn(3, '/');
    (parts.next()? == VECTOR_PREFIX).then_some(())?;
    Some((parts.next()?.to_string(), parts.next()?.to_string()))
}

fn key3(prefix: &str, provider: &str, value: &str) -> Vec<u8> {
    format!("{prefix}/{provider}/{value}").into_bytes()
}

fn hnsw_id(chunk_id: &str) -> usize {
    let digest = blake3::hash(chunk_id.as_bytes());
    let mut bytes = [0_u8; 8];
    bytes.copy_from_slice(&digest.as_bytes()[..8]);
    u64::from_le_bytes(bytes) as usize
}

fn safe_provider(provider: &str) -> String {
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

#[cfg(test)]
mod persisted_hnsw_tests;

#[cfg(test)]
mod tests;
