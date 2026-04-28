//! Weight persistence and in-memory weight store.
//!
//! `WeightStore` is the primary API for GnnEnhancer — in-memory with He/Xavier
//! initialization. PR 2 replaces this with CozoDB-backed versioning.
//!
//! `WeightManager` (below) is the legacy CRC32 file-based persistence, retained
//! for backward compatibility.

use std::collections::HashMap;
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
        // JS: seed = (seed + 0x9e3779b9) | 0  — truncates to 32-bit signed on each iteration
        let mut sm = seed as u32;
        for i in 0..4 {
            sm = sm.wrapping_add(0x9e3779b9);
            let z = sm;
            // Math.imul(z ^ (z >>> 16), 0x85ebca6b) — signed 32-bit multiply, low 32 bits
            let z = (z ^ (z >> 16)) as i32;
            let z = z.wrapping_mul(0x85ebca6b_u32 as i32) as u32;
            // Math.imul(z ^ (z >>> 13), 0xc2b2ae35)
            let z = (z ^ (z >> 13)) as i32;
            let z = z.wrapping_mul(0xc2b2ae35_u32 as i32) as u32;
            // z ^ (z >>> 16) >>> 0
            s[i] = z ^ (z >> 16);
        }
        Self { s }
    }

    /// Return a float in `[-0.5, 0.5]` — matches JS `nextFloat()`.
    pub fn next_float(&mut self) -> f32 {
        // JS: const result = Math.imul(s[1] * 5, 0x7FFFFFFF);
        // s[1] * 5 in JS is f64 multiply, then Math.imul truncates to signed 32-bit
        let x = self.s[1].wrapping_mul(5) as i32;
        let result = x.wrapping_mul(0x7FFFFFFF_i32) as u32;

        // State transition (standard xoshiro128** scramble)
        let t = self.s[1] << 9;
        self.s[2] ^= self.s[0];
        self.s[3] ^= self.s[1];
        self.s[1] ^= self.s[2];
        self.s[0] ^= self.s[3];
        self.s[2] ^= t;
        self.s[3] = (self.s[3] << 11) | (self.s[3] >> 21);

        // JS: (result >>> 0) / 0xFFFFFFFF - 0.5
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
// WeightStore
// ---------------------------------------------------------------------------

/// In-memory weight store for GNN layers.
///
/// Thread-safe: wraps weights in `Arc<RwLock<...>>` for concurrent access.
pub struct WeightStore {
    weights: RwLock<HashMap<String, Arc<Vec<Vec<f32>>>>>,
    biases: RwLock<HashMap<String, Arc<Vec<f32>>>>,
}

impl WeightStore {
    /// Create an empty weight store.
    pub fn new() -> Self {
        Self {
            weights: RwLock::new(HashMap::new()),
            biases: RwLock::new(HashMap::new()),
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
        // next_float() returns [-0.5, 0.5]; * 2.0 * scale gives [-scale, scale]
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

    /// Set weights and bias for a layer (used by trainer in PR 2, tests in PR 1).
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
}

impl Default for WeightStore {
    fn default() -> Self {
        Self::new()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_weight_store_initialize_and_get() {
        let store = WeightStore::new();
        store.initialize("test_layer", 4, 2, Initialization::He, 42);
        let w = store.get_weights("test_layer");
        assert_eq!(w.len(), 2);
        assert_eq!(w[0].len(), 4);
        // Weights should be non-zero
        assert!(w.iter().any(|row| row.iter().any(|&x| x != 0.0)));
    }

    #[test]
    fn test_weight_store_different_seeds() {
        let s1 = WeightStore::new();
        s1.initialize("l", 4, 2, Initialization::He, 1);
        let w1 = s1.get_weights("l");

        let s2 = WeightStore::new();
        s2.initialize("l", 4, 2, Initialization::He, 2);
        let w2 = s2.get_weights("l");

        // Different seeds should produce different weights
        let diff = w1
            .iter()
            .flatten()
            .zip(w2.iter().flatten())
            .any(|(a, b)| (a - b).abs() > 1e-6);
        assert!(diff);
    }

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
