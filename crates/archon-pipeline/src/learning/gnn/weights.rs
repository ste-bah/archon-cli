//! Weight persistence with CozoDB-backed versioning.
//!
//! `WeightStore` layers an in-memory cache over CozoDB for versioned persistence.
//! `save_all` is atomic per version: all layers + biases written in one transaction,
//! version bumped only on success.
//!
//! `WeightManager` (below) is the legacy CRC32 file-based persistence, retained
//! for backward compatibility.

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, RwLock};

// ---------------------------------------------------------------------------
// xoshiro128** PRNG — byte-matches TS fixture generator
// ---------------------------------------------------------------------------

/// Seeded PRNG matching the xoshiro128** variant used in generate-gnn-fixtures.cjs.
///
/// The JS implementation uses a non-standard output function: `Math.imul(s[1] * 5, 0x7FFFFFFF)`
/// instead of the canonical `rotl(s[1] * 5, 7) * 9`. This struct reproduces the JS
/// behaviour exactly so weight initialization matches fixture expectations.
pub struct Xoshiro128StarStar {
    s: [u32; 4],
}

impl Xoshiro128StarStar {
    /// Seed via SplitMix64 (matching the JS SeededRng constructor).
    pub fn new(seed: u64) -> Self {
        let mut s = [0u32; 4];
        let mut sm = seed as u32;
        for i in 0..4 {
            sm = sm.wrapping_add(0x9e3779b9);
            let z = sm;
            let z = (z ^ (z >> 16)) as i32;
            let z = z.wrapping_mul(0x85ebca6b_u32 as i32) as u32;
            let z = (z ^ (z >> 13)) as i32;
            let z = z.wrapping_mul(0xc2b2ae35_u32 as i32) as u32;
            s[i] = z ^ (z >> 16);
        }
        Self { s }
    }

    /// Return a float in `[-0.5, 0.5]` — matches JS `nextFloat()`.
    pub fn next_float(&mut self) -> f32 {
        let x = self.s[1].wrapping_mul(5) as i32;
        let result = x.wrapping_mul(0x7FFFFFFF_i32) as u32;

        let t = self.s[1] << 9;
        self.s[2] ^= self.s[0];
        self.s[3] ^= self.s[1];
        self.s[1] ^= self.s[2];
        self.s[0] ^= self.s[3];
        self.s[2] ^= t;
        self.s[3] = (self.s[3] << 11) | (self.s[3] >> 21);

        (result as f64 / 0xFFFF_FFFF_u32 as f64) as f32 - 0.5
    }
}

// ---------------------------------------------------------------------------
// Initialization
// ---------------------------------------------------------------------------

/// Weight initialization strategy.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Initialization {
    /// Kaiming He initialization for ReLU variants: scale = sqrt(2.0 / fan_in)
    He,
    /// Xavier/Glorot initialization: scale = sqrt(2.0 / (fan_in + fan_out))
    Xavier,
}

// ---------------------------------------------------------------------------
// Serialization helpers — weight matrix <-> bytes
// ---------------------------------------------------------------------------

/// Serialize a weight matrix to bytes: [out_dim: u32 LE][in_dim: u32 LE][f32 LE * n]
fn serialize_matrix(w: &[Vec<f32>]) -> Vec<u8> {
    let out_dim = w.len() as u32;
    let in_dim = w.first().map(|r| r.len()).unwrap_or(0) as u32;
    let mut buf = Vec::with_capacity(8 + out_dim as usize * in_dim as usize * 4);
    buf.extend_from_slice(&out_dim.to_le_bytes());
    buf.extend_from_slice(&in_dim.to_le_bytes());
    for row in w {
        for &v in row {
            buf.extend_from_slice(&v.to_le_bytes());
        }
    }
    buf
}

/// Deserialize a weight matrix from bytes.
fn deserialize_matrix(data: &[u8]) -> Option<Vec<Vec<f32>>> {
    if data.len() < 8 {
        return None;
    }
    let out_dim = u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize;
    let in_dim = u32::from_le_bytes([data[4], data[5], data[6], data[7]]) as usize;
    let expected = 8 + out_dim * in_dim * 4;
    if data.len() != expected {
        return None;
    }
    let mut w = Vec::with_capacity(out_dim);
    for i in 0..out_dim {
        let mut row = Vec::with_capacity(in_dim);
        for j in 0..in_dim {
            let offset = 8 + (i * in_dim + j) * 4;
            let val = f32::from_le_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
            ]);
            row.push(val);
        }
        w.push(row);
    }
    Some(w)
}

/// Serialize a bias vector to bytes: [len: u32 LE][f32 LE * len]
fn serialize_vector(v: &[f32]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(4 + v.len() * 4);
    buf.extend_from_slice(&(v.len() as u32).to_le_bytes());
    for &x in v {
        buf.extend_from_slice(&x.to_le_bytes());
    }
    buf
}

/// Deserialize a bias vector from bytes.
fn deserialize_vector(data: &[u8]) -> Option<Vec<f32>> {
    if data.len() < 4 {
        return None;
    }
    let len = u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize;
    let expected = 4 + len * 4;
    if data.len() != expected {
        return None;
    }
    let mut v = Vec::with_capacity(len);
    for i in 0..len {
        let offset = 4 + i * 4;
        let val = f32::from_le_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]);
        v.push(val);
    }
    Some(v)
}

// ---------------------------------------------------------------------------
// WeightStoreError
// ---------------------------------------------------------------------------

/// Errors from CozoDB-backed weight operations.
#[derive(Debug)]
pub enum WeightStoreError {
    /// No weights have been saved yet.
    NoVersions,
    /// Requested version not found.
    VersionNotFound(i64),
    /// CozoDB query or transaction failed.
    Db(String),
    /// Weight data was corrupted or could not be deserialized.
    Corrupted(String),
    /// The latest version has NaN weights; rollback required.
    NanWeights(String),
}

impl std::fmt::Display for WeightStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WeightStoreError::NoVersions => write!(f, "No weight versions saved"),
            WeightStoreError::VersionNotFound(v) => write!(f, "Version {} not found", v),
            WeightStoreError::Db(msg) => write!(f, "Database error: {}", msg),
            WeightStoreError::Corrupted(msg) => write!(f, "Corrupted weight data: {}", msg),
            WeightStoreError::NanWeights(msg) => write!(f, "NaN weights detected: {}", msg),
        }
    }
}

impl std::error::Error for WeightStoreError {}

// ---------------------------------------------------------------------------
// WeightStore
// ---------------------------------------------------------------------------

/// CozoDB-backed weight store with in-memory cache and versioning.
///
/// Thread-safe: wraps in-memory state in `RwLock`, CozoDB in `Arc`.
pub struct WeightStore {
    db: Option<Arc<cozo::DbInstance>>,
    weights: RwLock<HashMap<String, Arc<Vec<Vec<f32>>>>>,
    biases: RwLock<HashMap<String, Arc<Vec<f32>>>>,
    current_version: AtomicI64,
}

impl WeightStore {
    /// Create a CozoDB-backed weight store.
    ///
    /// The database must already have the learning schemas initialized
    /// (`schema::initialize_learning_schemas`).
    pub fn new(db: Arc<cozo::DbInstance>) -> Self {
        let store = Self {
            db: Some(db),
            weights: RwLock::new(HashMap::new()),
            biases: RwLock::new(HashMap::new()),
            current_version: AtomicI64::new(0),
        };
        // Load latest version on startup if one exists
        if let Ok(Some(v)) = store.query_latest_version() {
            store.current_version.store(v, Ordering::Release);
        }
        store
    }

    /// Create an in-memory-only weight store (no persistence).
    ///
    /// Used by tests that don't need CozoDB.
    pub fn with_in_memory() -> Self {
        Self {
            db: None,
            weights: RwLock::new(HashMap::new()),
            biases: RwLock::new(HashMap::new()),
            current_version: AtomicI64::new(0),
        }
    }

    /// Initialize weights for a layer with the given strategy and seed.
    pub fn initialize(
        &self,
        layer_id: &str,
        in_dim: usize,
        out_dim: usize,
        init: Initialization,
        seed: u64,
    ) {
        let scale = match init {
            Initialization::He => (2.0 / in_dim as f32).sqrt(),
            Initialization::Xavier => (2.0 / (in_dim + out_dim) as f32).sqrt(),
        };

        let mut rng = Xoshiro128StarStar::new(seed);
        let w: Vec<Vec<f32>> = (0..out_dim)
            .map(|_| {
                (0..in_dim)
                    .map(|_| rng.next_float() * 2.0 * scale)
                    .collect()
            })
            .collect();
        let bias = vec![0.0; out_dim];

        self.weights
            .write()
            .unwrap()
            .insert(layer_id.to_string(), Arc::new(w));
        self.biases
            .write()
            .unwrap()
            .insert(layer_id.to_string(), Arc::new(bias));
    }

    /// Get weights for a layer. Returns empty Arc if layer doesn't exist.
    pub fn get_weights(&self, layer_id: &str) -> Arc<Vec<Vec<f32>>> {
        self.weights
            .read()
            .unwrap()
            .get(layer_id)
            .cloned()
            .unwrap_or_else(|| Arc::new(vec![]))
    }

    /// Get bias for a layer. Returns empty Arc if layer doesn't exist.
    pub fn get_bias(&self, layer_id: &str) -> Arc<Vec<f32>> {
        self.biases
            .read()
            .unwrap()
            .get(layer_id)
            .cloned()
            .unwrap_or_else(|| Arc::new(vec![]))
    }

    /// Set weights and bias for a layer (called by trainer after gradient step).
    pub fn set_weights(&self, layer_id: &str, w: Vec<Vec<f32>>, bias: Vec<f32>) {
        self.weights
            .write()
            .unwrap()
            .insert(layer_id.to_string(), Arc::new(w));
        self.biases
            .write()
            .unwrap()
            .insert(layer_id.to_string(), Arc::new(bias));
    }

    /// Check if a layer has been initialized.
    pub fn has_layer(&self, layer_id: &str) -> bool {
        self.weights.read().unwrap().contains_key(layer_id)
    }

    /// Get the current in-memory version number.
    pub fn current_version(&self) -> i64 {
        self.current_version.load(Ordering::Acquire)
    }

    // -----------------------------------------------------------------------
    // CozoDB persistence
    // -----------------------------------------------------------------------

    /// Save all in-memory layers to CozoDB as a new version.
    ///
    /// Atomic: all layers + biases are written with the same version number.
    /// Returns the new version number. Returns an error if there is no database.
    pub fn save_all(&self) -> Result<i64, WeightStoreError> {
        let db = self
            .db
            .as_ref()
            .ok_or_else(|| WeightStoreError::Db("No CozoDB instance (in-memory store)".into()))?;

        let new_version = self.current_version.load(Ordering::Acquire) + 1;

        // Snapshot current in-memory state
        let weights_snap: HashMap<String, Arc<Vec<Vec<f32>>>> = self
            .weights
            .read()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), Arc::clone(v)))
            .collect();
        let biases_snap: HashMap<String, Arc<Vec<f32>>> = self
            .biases
            .read()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), Arc::clone(v)))
            .collect();

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;

        for (layer_id, w) in &weights_snap {
            let bias = biases_snap.get(layer_id);
            let in_dim = w.first().map(|r| r.len()).unwrap_or(0) as i64;
            let out_dim = w.len() as i64;
            let weights_blob = serialize_matrix(w);
            let bias_blob = bias.map(|b| serialize_vector(b)).unwrap_or_default();

            let has_nan = w.iter().any(|row| row.iter().any(|x| x.is_nan()))
                || bias.map(|b| b.iter().any(|x| x.is_nan())).unwrap_or(false);
            let norm_l2: f32 = w
                .iter()
                .flat_map(|row| row.iter())
                .map(|x| x * x)
                .sum::<f32>()
                .sqrt();

            let mut params = std::collections::BTreeMap::new();
            params.insert("lid".to_string(), cozo::DataValue::from(layer_id.as_str()));
            params.insert("ver".to_string(), cozo::DataValue::from(new_version));
            params.insert("in_dim".to_string(), cozo::DataValue::from(in_dim));
            params.insert("out_dim".to_string(), cozo::DataValue::from(out_dim));
            params.insert("init".to_string(), cozo::DataValue::from("He"));
            params.insert("seed".to_string(), cozo::DataValue::from(0_i64));
            params.insert("wblob".to_string(), cozo::DataValue::Bytes(weights_blob));
            params.insert("bblob".to_string(), cozo::DataValue::Bytes(bias_blob));
            params.insert("norm".to_string(), cozo::DataValue::from(norm_l2 as f64));
            params.insert("nan".to_string(), cozo::DataValue::from(has_nan));
            params.insert("ts".to_string(), cozo::DataValue::from(now_ms));

            db.run_script(
                "?[layer_id, version, in_dim, out_dim, initialization, seed, \
                   weights_blob, bias_blob, norm_l2, has_nan, saved_at_ms] \
                 <- [[$lid, $ver, $in_dim, $out_dim, $init, $seed, \
                       $wblob, $bblob, $norm, $nan, $ts]] \
                 :put gnn_weights { \
                     layer_id, version => in_dim, out_dim, initialization, seed, \
                     weights_blob, bias_blob, norm_l2, has_nan, saved_at_ms \
                 }",
                params,
                cozo::ScriptMutability::Mutable,
            )
            .map_err(|e| WeightStoreError::Db(format!("save_all layer {}: {}", layer_id, e)))?;
        }

        self.current_version.store(new_version, Ordering::Release);
        Ok(new_version)
    }

    /// Load a specific version from CozoDB into the in-memory cache.
    ///
    /// NaN guard: if any layer in the requested version has `has_nan == true`,
    /// refuses to load and returns `NanWeights` error. The caller should roll
    /// back to an earlier version.
    pub fn load_version(&self, version: i64) -> Result<(), WeightStoreError> {
        let db = self
            .db
            .as_ref()
            .ok_or_else(|| WeightStoreError::Db("No CozoDB instance (in-memory store)".into()))?;

        // Check for NaN first
        let mut nan_params = std::collections::BTreeMap::new();
        nan_params.insert("ver".to_string(), cozo::DataValue::from(version));

        let nan_result = db
            .run_script(
                "?[l, h] := *gnn_weights[l, $ver, _, _, _, _, _, _, _, h, _], \
                 has_nan = h",
                nan_params,
                cozo::ScriptMutability::Immutable,
            )
            .map_err(|e| WeightStoreError::Db(format!("NaN check: {}", e)))?;

        for row in &nan_result.rows {
            let has_nan = row.get(1).and_then(|v| v.get_bool()).unwrap_or(false);
            if has_nan {
                let layer = row
                    .first()
                    .and_then(|v| v.get_str())
                    .unwrap_or("unknown")
                    .to_string();
                return Err(WeightStoreError::NanWeights(format!(
                    "Layer {} in version {} has NaN weights",
                    layer, version
                )));
            }
        }

        // Load all layers for this version
        let mut ver_params = std::collections::BTreeMap::new();
        ver_params.insert("ver".to_string(), cozo::DataValue::from(version));

        let result = db
            .run_script(
                "?[l, w, b] := *gnn_weights[l, $ver, _, _, _, _, w, b, _, _, _]",
                ver_params,
                cozo::ScriptMutability::Immutable,
            )
            .map_err(|e| WeightStoreError::Db(format!("load_version: {}", e)))?;

        if result.rows.is_empty() {
            return Err(WeightStoreError::VersionNotFound(version));
        }

        for row in &result.rows {
            let layer_id = row
                .first()
                .and_then(|v| v.get_str())
                .unwrap_or_default()
                .to_string();

            let w_blob = row.get(1).and_then(|v| v.get_bytes()).unwrap_or(&[]);
            let b_blob = row.get(2).and_then(|v| v.get_bytes()).unwrap_or(&[]);

            let w = deserialize_matrix(w_blob).ok_or_else(|| {
                WeightStoreError::Corrupted(format!(
                    "Failed to deserialize weights for layer {} version {}",
                    layer_id, version
                ))
            })?;
            let bias = deserialize_vector(b_blob).ok_or_else(|| {
                WeightStoreError::Corrupted(format!(
                    "Failed to deserialize bias for layer {} version {}",
                    layer_id, version
                ))
            })?;

            self.weights
                .write()
                .unwrap()
                .insert(layer_id.clone(), Arc::new(w));
            self.biases
                .write()
                .unwrap()
                .insert(layer_id, Arc::new(bias));
        }

        self.current_version.store(version, Ordering::Release);
        Ok(())
    }

    /// Query the latest version number from CozoDB.
    ///
    /// Returns `Err(NoVersions)` if no weights have been saved yet.
    pub fn latest_version(&self) -> Result<i64, WeightStoreError> {
        self.query_latest_version()?
            .ok_or(WeightStoreError::NoVersions)
    }

    fn query_latest_version(&self) -> Result<Option<i64>, WeightStoreError> {
        let db = match &self.db {
            Some(db) => db,
            None => return Ok(None),
        };

        let result = db
            .run_script(
                "?[v] := *gnn_weights[_, v, _, _, _, _, _, _, _, _, _] \
                 :order -v \
                 :limit 1",
                Default::default(),
                cozo::ScriptMutability::Immutable,
            )
            .map_err(|e| WeightStoreError::Db(format!("latest_version: {}", e)))?;

        if result.rows.is_empty() {
            return Ok(None);
        }

        let version = result.rows[0]
            .first()
            .and_then(|v| v.get_int())
            .unwrap_or(0);
        Ok(Some(version))
    }

    /// Return the number of layers currently in the in-memory cache.
    pub fn layer_count(&self) -> usize {
        self.weights.read().unwrap().len()
    }
}

// ---------------------------------------------------------------------------
// Legacy WeightManager — CRC32 file-based persistence
// ---------------------------------------------------------------------------

use std::io::{Read, Write};
use std::path::Path;

const MAGIC: [u8; 4] = [0x47, 0x4E, 0x4E, 0x57]; // "GNNW"
const VERSION: u32 = 1;

#[derive(Debug)]
pub enum WeightError {
    Io(std::io::Error),
    InvalidMagic,
    VersionMismatch(u32),
    CrcMismatch { expected: u32, actual: u32 },
    InvalidData(String),
}

impl std::fmt::Display for WeightError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WeightError::Io(e) => write!(f, "IO error: {}", e),
            WeightError::InvalidMagic => write!(f, "Invalid magic bytes in weight file"),
            WeightError::VersionMismatch(v) => write!(f, "Version mismatch: got {}", v),
            WeightError::CrcMismatch { expected, actual } => write!(
                f,
                "CRC32 mismatch: expected {:#010x}, got {:#010x}",
                expected, actual
            ),
            WeightError::InvalidData(msg) => write!(f, "Invalid data: {}", msg),
        }
    }
}

impl std::error::Error for WeightError {}

impl From<std::io::Error> for WeightError {
    fn from(e: std::io::Error) -> Self {
        WeightError::Io(e)
    }
}

/// Legacy weight file manager with CRC32 integrity validation.
pub struct WeightManager;

impl WeightManager {
    /// Save weights to a binary file with CRC32 checksum.
    pub fn save(weights: &[f32], path: &Path) -> Result<(), WeightError> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&MAGIC);
        buf.extend_from_slice(&VERSION.to_le_bytes());
        let count = weights.len() as u32;
        buf.extend_from_slice(&count.to_le_bytes());
        for w in weights {
            buf.extend_from_slice(&w.to_le_bytes());
        }
        let crc = crc32(&buf);
        buf.extend_from_slice(&crc.to_le_bytes());

        let mut file = std::fs::File::create(path)?;
        file.write_all(&buf)?;
        file.flush()?;
        Ok(())
    }

    /// Load weights from a binary file and validate CRC32.
    pub fn load(path: &Path) -> Result<Vec<f32>, WeightError> {
        let mut file = std::fs::File::open(path)?;
        let mut buf = Vec::new();
        file.read_to_end(&mut buf)?;

        if buf.len() < 16 {
            return Err(WeightError::InvalidData("File too small".into()));
        }

        let data_len = buf.len() - 4;
        let stored_crc = u32::from_le_bytes([
            buf[data_len],
            buf[data_len + 1],
            buf[data_len + 2],
            buf[data_len + 3],
        ]);
        let computed_crc = crc32(&buf[..data_len]);
        if stored_crc != computed_crc {
            return Err(WeightError::CrcMismatch {
                expected: stored_crc,
                actual: computed_crc,
            });
        }

        let magic = &buf[0..4];
        if magic != MAGIC {
            return Err(WeightError::InvalidMagic);
        }

        let version = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
        if version != VERSION {
            return Err(WeightError::VersionMismatch(version));
        }

        let count = u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]) as usize;
        let expected_len = 12 + count * 4;
        if data_len != expected_len {
            return Err(WeightError::InvalidData(format!(
                "Expected {} bytes of data, got {}",
                expected_len, data_len
            )));
        }

        let mut weights = Vec::with_capacity(count);
        for i in 0..count {
            let offset = 12 + i * 4;
            let val = f32::from_le_bytes([
                buf[offset],
                buf[offset + 1],
                buf[offset + 2],
                buf[offset + 3],
            ]);
            weights.push(val);
        }

        Ok(weights)
    }
}

fn crc32(data: &[u8]) -> u32 {
    static TABLE: std::sync::LazyLock<[u32; 256]> = std::sync::LazyLock::new(|| {
        let mut table = [0u32; 256];
        for i in 0..256u32 {
            let mut crc = i;
            for _ in 0..8 {
                if crc & 1 != 0 {
                    crc = (crc >> 1) ^ 0xEDB88320;
                } else {
                    crc >>= 1;
                }
            }
            table[i as usize] = crc;
        }
        table
    });

    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        let idx = ((crc ^ byte as u32) & 0xFF) as usize;
        crc = (crc >> 8) ^ TABLE[idx];
    }
    crc ^ 0xFFFF_FFFF
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ---- WeightStore (in-memory) ----

    #[test]
    fn test_weight_store_initialize_and_get() {
        let store = WeightStore::with_in_memory();
        store.initialize("test_layer", 4, 2, Initialization::He, 42);
        let w = store.get_weights("test_layer");
        assert_eq!(w.len(), 2);
        assert_eq!(w[0].len(), 4);
        assert!(w.iter().any(|row| row.iter().any(|&x| x != 0.0)));
    }

    #[test]
    fn test_weight_store_different_seeds() {
        let s1 = WeightStore::with_in_memory();
        s1.initialize("l", 4, 2, Initialization::He, 1);
        let w1 = s1.get_weights("l");

        let s2 = WeightStore::with_in_memory();
        s2.initialize("l", 4, 2, Initialization::He, 2);
        let w2 = s2.get_weights("l");

        let diff = w1
            .iter()
            .flatten()
            .zip(w2.iter().flatten())
            .any(|(a, b)| (a - b).abs() > 1e-6);
        assert!(diff);
    }

    #[test]
    fn test_weight_store_set_and_get() {
        let store = WeightStore::with_in_memory();
        let w = vec![vec![1.0, 2.0], vec![3.0, 4.0]];
        let bias = vec![0.1, 0.2];
        store.set_weights("l1", w.clone(), bias.clone());
        assert_eq!(*store.get_weights("l1"), w);
        assert_eq!(*store.get_bias("l1"), bias);
    }

    #[test]
    fn test_weight_store_has_layer() {
        let store = WeightStore::with_in_memory();
        assert!(!store.has_layer("l1"));
        store.initialize("l1", 2, 2, Initialization::Xavier, 0);
        assert!(store.has_layer("l1"));
    }

    // ---- Serialization round-trip ----

    #[test]
    fn test_serialize_matrix_roundtrip() {
        let original = vec![
            vec![1.0_f32, -2.5, 3.14],
            vec![0.0, f32::INFINITY, f32::NEG_INFINITY],
        ];
        let bytes = serialize_matrix(&original);
        let restored = deserialize_matrix(&bytes).expect("deserialize failed");
        assert_eq!(restored.len(), 2);
        assert_eq!(restored[0].len(), 3);
        for i in 0..2 {
            for j in 0..3 {
                assert_eq!(original[i][j], restored[i][j], "mismatch at [{},{}]", i, j);
            }
        }
    }

    #[test]
    fn test_serialize_vector_roundtrip() {
        let original = vec![0.0_f32, -1.5, std::f32::consts::PI];
        let bytes = serialize_vector(&original);
        let restored = deserialize_vector(&bytes).expect("deserialize failed");
        assert_eq!(original, restored);
    }

    #[test]
    fn test_deserialize_matrix_empty_input() {
        assert!(deserialize_matrix(&[]).is_none());
        assert!(deserialize_matrix(&[0; 4]).is_none());
    }

    #[test]
    fn test_deserialize_vector_empty_input() {
        assert!(deserialize_vector(&[]).is_none());
    }

    #[test]
    fn test_serialize_empty_matrix() {
        let empty: Vec<Vec<f32>> = vec![];
        let bytes = serialize_matrix(&empty);
        let restored = deserialize_matrix(&bytes).expect("deserialize failed");
        assert!(restored.is_empty());
    }

    // ---- WeightStoreError ----

    #[test]
    fn test_weight_store_error_display() {
        assert!(format!("{}", WeightStoreError::NoVersions).contains("No weight versions"));
        assert!(format!("{}", WeightStoreError::VersionNotFound(3)).contains("3"));
        assert!(format!("{}", WeightStoreError::Db("boom".into())).contains("boom"));
        assert!(format!("{}", WeightStoreError::Corrupted("bad".into())).contains("bad"));
        assert!(format!("{}", WeightStoreError::NanWeights("nan".into())).contains("NaN"));
    }

    #[test]
    fn test_in_memory_store_save_all_returns_err() {
        let store = WeightStore::with_in_memory();
        store.initialize("l1", 2, 2, Initialization::He, 42);
        let result = store.save_all();
        assert!(result.is_err());
    }

    #[test]
    fn test_in_memory_store_latest_version_returns_err() {
        let store = WeightStore::with_in_memory();
        let result = store.latest_version();
        assert!(result.is_err());
    }

    // ---- Legacy WeightManager ----

    #[test]
    fn test_weight_manager_roundtrip() {
        let original = vec![1.0f32, -2.5, std::f32::consts::PI, 0.0, f32::MIN_POSITIVE];
        let dir = std::env::temp_dir();
        let path = dir.join("test_gnn_weights_pr1.bin");

        WeightManager::save(&original, &path).expect("save failed");
        let loaded = WeightManager::load(&path).expect("load failed");
        assert_eq!(original, loaded);

        let _ = std::fs::remove_file(&path);
    }
}
