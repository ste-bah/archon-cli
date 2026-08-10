//! The standalone verification entry points.
//!
//! These are the only associated functions on [`TradingDataLake`] that take no
//! `&self`: they are handed a path that already exists on disk and asked
//! whether what is there holds up. The rest of the parent module is the
//! read/write path of a lake you own; this is the audit path over one someone
//! else produced, so it lives on its own.

use super::*;

impl TradingDataLake {
    pub fn verify_artifact_dir(dataset_dir: &Path) -> Result<StoredDatasetRecord, DataStoreError> {
        let root = project_root_for_artifact(dataset_dir)?;
        let record: StoredDatasetRecord = read_json(&dataset_dir.join("manifest.json"))?;
        verify_artifacts(&root, &record)?;
        Ok(record)
    }

    /// Verify a registry file and every dataset it registers.
    ///
    /// Structural validity of `registry.json` alone would be a weak claim: the
    /// registry's job is to name datasets that hold up, so a registry listing a
    /// broken dataset must fail. This walks each entry through the same
    /// artifact verification a dataset directory gets.
    pub fn verify_registry_file(
        registry_path: &Path,
    ) -> Result<PersistentDatasetRegistry, DataStoreError> {
        let root = project_root_for_artifact(registry_path)?;
        let registry: PersistentDatasetRegistry = read_json(registry_path)?;
        for record in registry.datasets.values() {
            verify_artifacts(&root, record)?;
        }
        Ok(registry)
    }

    pub fn verify_coverage_files(
        coverage_path: &Path,
        registry_path: &Path,
    ) -> Result<CoverageMatrix, DataStoreError> {
        let root = project_root_for_artifact(registry_path)?;
        let matrix: CoverageMatrix = read_json(coverage_path)?;
        let registry: PersistentDatasetRegistry = read_json(registry_path)?;
        validate_coverage_matrix_complete(&matrix)?;
        for cell in matrix.cells.iter().filter(|cell| cell.available) {
            let dataset_id = cell.dataset_id.as_deref().ok_or_else(|| {
                DataStoreError::IncompleteArtifactContract("coverage dataset id".into())
            })?;
            let version = cell.version.as_deref().ok_or_else(|| {
                DataStoreError::IncompleteArtifactContract("coverage dataset version".into())
            })?;
            let record = registry
                .datasets
                .get(&registry_key(dataset_id, version))
                .ok_or_else(|| {
                    DataStoreError::IncompleteArtifactContract(format!(
                        "coverage registry link {dataset_id}:{version}"
                    ))
                })?;
            if cell.dataset_checksum.as_deref() != Some(record.checksum.as_str()) {
                return Err(DataStoreError::IncompleteArtifactContract(format!(
                    "coverage checksum link {dataset_id}:{version}"
                )));
            }
            let lake = Self::new(root.clone());
            coverage_record_issues(&lake, record, &cell.canonical_instrument, &cell.timeframe)
                .map_err(|issues| {
                    DataStoreError::IncompleteArtifactContract(format!(
                        "coverage dataset {dataset_id}:{version} rejected: {}",
                        issues.join("; ")
                    ))
                })?;
        }
        Ok(matrix)
    }
}
