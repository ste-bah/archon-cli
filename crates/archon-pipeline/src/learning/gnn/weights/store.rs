use super::errors::WeightStoreError;
use super::prng::Xoshiro128StarStar;
use super::serialization::{
    deserialize_matrix, deserialize_vector, serialize_matrix, serialize_vector,
};
use super::types::Initialization;
use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, RwLock};

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
