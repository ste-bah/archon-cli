use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use hnsw_rs::prelude::{AnnT, DistCosine, Hnsw};
use rust_rocksdb::{DB, Options, WriteBatch};
use serde::{Deserialize, Serialize};

const VECTOR_PREFIX: &str = "vec";
const CACHE_PREFIX: &str = "cache";
const ID_PREFIX: &str = "id";
const DEFAULT_STORE_DIR: &str = "doc-vector-store";

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
}

#[derive(Clone, Debug)]
pub struct HnswSearchHit {
    pub chunk_id: String,
    pub distance: f32,
}

pub struct DocVectorStore {
    db: DB,
    root: PathBuf,
}

impl DocVectorStore {
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
        options.increase_parallelism(num_parallelism());
        let db = DB::open(&options, &root)
            .with_context(|| format!("open RocksDB vector store {}", root.display()))?;
        Ok(Self { db, root })
    }

    pub fn put_vectors(&self, rows: &[VectorWrite<'_>]) -> Result<usize> {
        if rows.is_empty() {
            return Ok(0);
        }
        let mut batch = WriteBatch::default();
        for row in rows {
            if row.embedding.is_empty() {
                continue;
            }
            let bytes = encode_vector(row.embedding);
            batch.put(vector_key(row.provider, row.chunk_id), &bytes);
            batch.put(
                id_key(row.provider, row.chunk_id),
                hnsw_id(row.chunk_id).to_be_bytes(),
            );
            if !row.content_hash.is_empty() {
                batch.put(cache_key(row.provider, row.content_hash), &bytes);
            }
        }
        self.db
            .write(batch)
            .context("write raw vectors to RocksDB vector store")?;
        Ok(rows.len())
    }

    pub fn has_vector(&self, provider: &str, chunk_id: &str) -> Result<bool> {
        self.db
            .get_pinned(vector_key(provider, chunk_id))
            .context("read vector presence from RocksDB")
            .map(|value| value.is_some())
    }

    pub fn cached_embedding(&self, provider: &str, content_hash: &str) -> Result<Option<Vec<f32>>> {
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
        Ok(self.iter_records(provider, None)?.len())
    }

    pub fn stats(&self, provider: Option<&str>) -> Result<VectorStoreStats> {
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
        let records = self.iter_records(Some(provider), limit)?;
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
        };
        self.write_hnsw_manifest(provider, &manifest)?;
        Ok(manifest)
    }

    pub fn search_in_memory(
        &self,
        provider: &str,
        query: &[f32],
        top_k: usize,
        ef: usize,
        limit: Option<usize>,
    ) -> Result<Vec<HnswSearchHit>> {
        let records = self.iter_records(Some(provider), limit)?;
        if records.is_empty() || top_k == 0 {
            return Ok(Vec::new());
        }
        let mut hnsw = build_hnsw_index(&records, query.len())?;
        hnsw.set_searching_mode(true);
        let hits = hnsw.search(query, top_k, ef.max(top_k));
        Ok(hits
            .into_iter()
            .filter_map(|hit| {
                records
                    .iter()
                    .find(|record| record.hnsw_id == hit.get_origin_id())
                    .map(|record| HnswSearchHit {
                        chunk_id: record.chunk_id.clone(),
                        distance: hit.get_distance(),
                    })
            })
            .collect())
    }

    pub fn latest_hnsw_manifest(&self, provider: &str) -> Result<Option<HnswManifest>> {
        let path = self.hnsw_manifest_path(provider);
        if !path.exists() {
            return Ok(None);
        }
        let bytes = std::fs::read(&path)
            .with_context(|| format!("read HNSW manifest {}", path.display()))?;
        serde_json::from_slice(&bytes).context("parse HNSW manifest")
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

    fn write_hnsw_manifest(&self, provider: &str, manifest: &HnswManifest) -> Result<()> {
        let path = self.hnsw_manifest_path(provider);
        let bytes = serde_json::to_vec_pretty(manifest)?;
        std::fs::write(&path, bytes)
            .with_context(|| format!("write HNSW manifest {}", path.display()))
    }
}

pub fn default_store_dir() -> PathBuf {
    if let Some(path) = std::env::var_os("ARCHON_DOC_VECTOR_STORE_DIR") {
        return PathBuf::from(path);
    }
    #[cfg(test)]
    {
        std::env::temp_dir()
            .join(format!("archon-{DEFAULT_STORE_DIR}-tests"))
            .join(format!(
                "test-{}-{}",
                std::process::id(),
                test_thread_suffix()
            ))
    }
    #[cfg(not(test))]
    {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(".archon")
            .join(DEFAULT_STORE_DIR)
    }
}

#[cfg(test)]
fn test_thread_suffix() -> String {
    format!("{:?}", std::thread::current().id())
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect()
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

fn vector_key(provider: &str, chunk_id: &str) -> Vec<u8> {
    key3(VECTOR_PREFIX, provider, chunk_id)
}

fn cache_key(provider: &str, content_hash: &str) -> Vec<u8> {
    key3(CACHE_PREFIX, provider, content_hash)
}

fn id_key(provider: &str, chunk_id: &str) -> Vec<u8> {
    key3(ID_PREFIX, provider, chunk_id)
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
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn num_parallelism() -> i32 {
    std::thread::available_parallelism()
        .map(|count| count.get().min(8) as i32)
        .unwrap_or(2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rocksdb_store_round_trips_vectors_and_cache() {
        let temp = tempfile::tempdir().unwrap();
        let store = DocVectorStore::open(temp.path()).unwrap();
        let rows = [VectorWrite {
            chunk_id: "chunk-a",
            content_hash: "hash-a",
            provider: "test",
            embedding: &[0.25, 0.75],
        }];
        assert_eq!(store.put_vectors(&rows).unwrap(), 1);
        assert_eq!(store.stats(Some("test")).unwrap().raw_vectors, 1);
        assert_eq!(
            store.cached_embedding("test", "hash-a").unwrap().unwrap(),
            vec![0.25, 0.75]
        );
    }

    #[test]
    fn rust_hnsw_search_returns_nearest_chunk() {
        let temp = tempfile::tempdir().unwrap();
        let store = DocVectorStore::open(temp.path()).unwrap();
        let rows = [
            VectorWrite {
                chunk_id: "chunk-a",
                content_hash: "hash-a",
                provider: "test",
                embedding: &[1.0, 0.0],
            },
            VectorWrite {
                chunk_id: "chunk-b",
                content_hash: "hash-b",
                provider: "test",
                embedding: &[0.0, 1.0],
            },
        ];
        store.put_vectors(&rows).unwrap();
        let hits = store
            .search_in_memory("test", &[0.99, 0.01], 1, 16, None)
            .unwrap();
        assert_eq!(hits[0].chunk_id, "chunk-a");
    }
}
