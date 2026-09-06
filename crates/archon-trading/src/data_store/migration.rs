use super::*;

#[derive(Debug, Clone)]
pub(super) struct RegistryMigration {
    pub registry: PersistentDatasetRegistry,
    pub report: RegistryMigrationReport,
}

pub(super) fn migrate_registry(
    root: &Path,
    data_root: &Path,
    mut registry: PersistentDatasetRegistry,
    write_reports: bool,
) -> Result<RegistryMigration, DataStoreError> {
    if registry.schema_version != REGISTRY_SCHEMA_V1 {
        return Err(DataStoreError::InvalidRegistrySchema(
            registry.schema_version.clone(),
        ));
    }
    let mut report = RegistryMigrationReport {
        schema_version: REGISTRY_SCHEMA_V1.into(),
        backup_path: latest_registry_backup(data_root)?,
        ..RegistryMigrationReport::default()
    };

    // Repair missing validation.json and manifest.json for every v1 record.
    // A v1 record created before the current codebase wrote these as separate
    // artifacts, or one that was stripped in a test, must have them regenerated
    // from the remaining on-disk data so that verify_artifacts does not reject
    // the registry.
    let keys: Vec<String> = registry.datasets.keys().cloned().collect();
    for key in &keys {
        let record = registry.datasets.get(key).unwrap();
        let validation_path = root.join(&record.validation_path);
        let manifest_path = root.join(&record.manifest_path);

        if !validation_path.exists() || !manifest_path.exists() {
            repair_missing_artifacts(root, &mut registry, key, &mut report)?;
        }
    }

    if !restore_migration_report_artifacts(root, data_root, &mut report)? && write_reports {
        report.report_path = Some(write_registry_migration_report(root, data_root, &report)?);
    }
    Ok(RegistryMigration { registry, report })
}

/// Regenerate validation.json and manifest.json for a single dataset record
/// whose on-disk artifacts are missing.  The record's metadata, normalized
/// bars, and raw response are assumed to exist; if they do not the error
/// propagates naturally through `verify_artifacts` later.
fn repair_missing_artifacts(
    root: &Path,
    registry: &mut PersistentDatasetRegistry,
    key: &str,
    report: &mut RegistryMigrationReport,
) -> Result<(), DataStoreError> {
    let record = registry.datasets.get(key).unwrap();
    let metadata_path = root.join(&record.metadata_path);
    let validation_path = root.join(&record.validation_path);
    let manifest_path = root.join(&record.manifest_path);
    let normalized_path = root.join(&record.normalized_path);

    // If even the metadata is gone there is nothing we can repair.
    if !metadata_path.exists() || !normalized_path.exists() {
        report.failed += 1;
        return Ok(());
    }

    // Read what we can from disk.
    let metadata: DatasetMetadata = read_json(&metadata_path)?;
    let bars: Vec<OhlcvBar> = match read_jsonl_bars(&normalized_path) {
        Ok(b) => b,
        Err(_) => {
            report.failed += 1;
            return Ok(());
        }
    };

    // Regenerate validation report.
    let validated_at = record.created_at.clone();
    let validation = crate::data_store::validation::validation_report_at_root(
        root,
        &metadata,
        &bars,
        validated_at,
    );
    write_schema_json(&validation_path, &validation)?;

    // Rebuild manifest (StoredDatasetRecord) from the record we have.
    // The regenerated manifest uses the registry record's data as-is;
    // we cannot change checksums or paths because verify_artifacts will
    // compare them.
    write_schema_json(&manifest_path, record)?;

    report.degraded += 1;
    Ok(())
}

fn write_registry_migration_report(
    root: &Path,
    data_root: &Path,
    report: &RegistryMigrationReport,
) -> Result<String, DataStoreError> {
    let path = data_root.join("registry-migration-report.json");
    write_schema_json(&path, report)?;
    relative(root, &path)
}

fn restore_migration_report_artifacts(
    root: &Path,
    data_root: &Path,
    report: &mut RegistryMigrationReport,
) -> Result<bool, DataStoreError> {
    let path = data_root.join("registry-migration-report.json");
    if path.exists() {
        report.report_path = Some(relative(root, &path)?);
        return Ok(true);
    }
    Ok(false)
}
