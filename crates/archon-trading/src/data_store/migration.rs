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
    let original_schema = registry.schema_version.clone();
    if original_schema == REGISTRY_SCHEMA_V2 {
        let migrated = registry.datasets.len();
        let degraded = count_degraded(&registry);
        let mut report = RegistryMigrationReport {
            schema_version: REGISTRY_SCHEMA_V2.into(),
            migrated,
            skipped: migrated,
            degraded,
            failed: 0,
            backup_path: latest_registry_backup(data_root)?,
            ..RegistryMigrationReport::default()
        };
        if write_reports || !restore_migration_report_artifacts(root, data_root, &mut report)? {
            report.report_path = Some(write_registry_migration_report(root, data_root, &report)?);
        }
        return Ok(RegistryMigration { registry, report });
    }
    if original_schema != REGISTRY_SCHEMA_V1 {
        return Err(DataStoreError::InvalidRegistrySchema(original_schema));
    }
    registry.schema_version = REGISTRY_SCHEMA_V2.into();
    let mut report = RegistryMigrationReport {
        schema_version: REGISTRY_SCHEMA_V2.into(),
        migrated: 0,
        skipped: 0,
        degraded: 0,
        failed: 0,
        backup_path: latest_registry_backup(data_root)?,
        ..RegistryMigrationReport::default()
    };
    for record in registry.datasets.values_mut() {
        match migrate_record(root, record, write_reports) {
            Ok(validation_path) => {
                report.migrated += 1;
                report.degraded += 1;
                report.validation_report_paths.push(validation_path);
            }
            Err(_) => {
                report.failed += 1;
                record.status = DatasetStatus::Degraded;
            }
        }
    }
    if write_reports {
        report.report_path = Some(write_registry_migration_report(root, data_root, &report)?);
    }
    Ok(RegistryMigration { registry, report })
}

fn count_degraded(registry: &PersistentDatasetRegistry) -> usize {
    registry
        .datasets
        .values()
        .filter(|record| record.status == DatasetStatus::Degraded)
        .count()
}

fn migrate_record(
    root: &Path,
    record: &mut StoredDatasetRecord,
    write_reports: bool,
) -> Result<String, DataStoreError> {
    let metadata_path = root.join(&record.metadata_path);
    let mut metadata = read_migration_metadata(&metadata_path, record)?;
    fail_closed_metadata(&mut metadata);
    record.status = DatasetStatus::Degraded;
    fill_migration_paths(record);
    fill_migration_registry_contract(root, record)?;
    fill_migration_metadata_fields(record, &metadata);
    if write_reports {
        write_migration_raw_contract(root, record)?;
        persist_migrated_metadata(&metadata_path, &metadata)?;
        write_migration_validation(root, record, &metadata)?;
    }
    Ok(record.validation_path.clone())
}

fn read_migration_metadata(
    path: &Path,
    record: &StoredDatasetRecord,
) -> Result<DatasetMetadata, DataStoreError> {
    if path.exists() {
        read_json(path)
    } else {
        Err(DataStoreError::IncompleteArtifactContract(
            record.metadata_path.clone(),
        ))
    }
}

fn fail_closed_metadata(metadata: &mut DatasetMetadata) {
    metadata.native_interval = false;
    metadata.production_eligible = false;
    metadata.quality_status = "degraded".into();
    if metadata.provider_symbol.trim().is_empty() {
        metadata.provider_symbol = metadata
            .symbol_map
            .values()
            .next()
            .cloned()
            .unwrap_or_else(|| metadata.canonical_instrument.clone());
    }
}

fn fill_migration_paths(record: &mut StoredDatasetRecord) {
    let Some((dir, _)) = record.metadata_path.rsplit_once('/') else {
        return;
    };
    if record.validation_path.trim().is_empty() {
        record.validation_path = format!("{dir}/validation.json");
    }
    if record.manifest_path.trim().is_empty() {
        record.manifest_path = format!("{dir}/manifest.json");
    }
}

fn fill_migration_registry_contract(
    root: &Path,
    record: &mut StoredDatasetRecord,
) -> Result<(), DataStoreError> {
    record.schema_version = REGISTRY_SCHEMA_V2.into();
    if record.dataset_path.trim().is_empty() {
        let Some((dir, _)) = record.metadata_path.rsplit_once('/') else {
            return Err(DataStoreError::InvalidPath);
        };
        record.dataset_path = dir.into();
    }
    let legacy_raw_path = record.raw_path.clone();
    record.raw_response_path = dataset_raw_path(record, "response.json");
    record.raw_request_path = dataset_raw_path(record, "request.json");
    record.redacted_headers_path = dataset_raw_path(record, "headers.redacted.json");
    record.provider_notes_path = dataset_raw_path(record, "provider-notes.md");
    record.metadata_checksum = checksum_file(root, &record.metadata_path)?;
    record.raw_checksum = checksum_file(root, &legacy_raw_path)?;
    record.raw_path = record.raw_response_path.clone();
    Ok(())
}

fn fill_migration_metadata_fields(record: &mut StoredDatasetRecord, metadata: &DatasetMetadata) {
    record.symbol = metadata.canonical_instrument.clone();
    record.timeframe = metadata.timeframe.clone();
    record.native_interval = false;
    record.production_eligible = false;
}

fn write_migration_raw_contract(
    root: &Path,
    record: &StoredDatasetRecord,
) -> Result<(), DataStoreError> {
    if !root.join(&record.raw_response_path).exists() {
        if root.join(&record.raw_path).exists() {
            std::fs::copy(
                root.join(&record.raw_path),
                root.join(&record.raw_response_path),
            )
            .map_err(io_error)?;
        } else {
            let legacy = legacy_raw_response_path(root, record)?;
            std::fs::copy(legacy, root.join(&record.raw_response_path)).map_err(io_error)?;
        }
    }
    if !root.join(&record.raw_request_path).exists() {
        write_json(&root.join(&record.raw_request_path), &serde_json::json!({}))?;
    }
    if !root.join(&record.redacted_headers_path).exists() {
        write_json(
            &root.join(&record.redacted_headers_path),
            &serde_json::json!({}),
        )?;
    }
    if !root.join(&record.provider_notes_path).exists() {
        write_text(
            &root.join(&record.provider_notes_path),
            "v1 migration; provider notes unavailable",
        )?;
    }
    Ok(())
}

fn legacy_raw_response_path(
    root: &Path,
    record: &StoredDatasetRecord,
) -> Result<PathBuf, DataStoreError> {
    let raw_dir = root
        .join(&record.raw_response_path)
        .parent()
        .ok_or(DataStoreError::InvalidPath)?
        .to_path_buf();
    for file_name in [
        "raw-response.json",
        "response.csv",
        "response.json",
        "response.txt",
        "response.zip",
    ] {
        let path = raw_dir.join(file_name);
        if path.exists() {
            return Ok(path);
        }
    }
    Err(DataStoreError::IncompleteArtifactContract(
        record.raw_response_path.clone(),
    ))
}

fn persist_migrated_metadata(
    path: &Path,
    metadata: &DatasetMetadata,
) -> Result<(), DataStoreError> {
    write_json(path, metadata)
}

fn write_migration_validation(
    root: &Path,
    record: &StoredDatasetRecord,
    metadata: &DatasetMetadata,
) -> Result<(), DataStoreError> {
    let report = migration_validation_report(metadata, record);
    write_json(&root.join(&record.validation_path), &report)?;
    write_json(&root.join(&record.manifest_path), record)
}

fn migration_validation_report(
    metadata: &DatasetMetadata,
    record: &StoredDatasetRecord,
) -> ValidationReport {
    ValidationReport {
        schema_version: "archon-trading-validation-v1".into(),
        dataset_id: record.dataset_id.clone(),
        version: record.version.clone(),
        status: ValidationStatus::Degraded,
        native_interval: false,
        production_eligible: false,
        checks: migration_checks(),
        summary: ValidationSummary {
            row_count: record.bars as u64,
            duplicate_timestamp_count: 0,
            gap_count: metadata.gaps.missing_bars,
            bad_ohlc_count: 0,
            missing_volume_count: 0,
        },
        validated_at: record.created_at.clone(),
    }
}

fn migration_checks() -> Vec<ValidationCheck> {
    vec![ValidationCheck {
        id: "migration.v1_conservative_upgrade".into(),
        status: ValidationStatus::Failed,
        severity: ValidationSeverity::Warning,
        message: "v1 registry migration preserved dataset and failed closed: native_interval=false, production_eligible=false".into(),
    }]
}

fn write_registry_migration_report(
    root: &Path,
    data_root: &Path,
    report: &RegistryMigrationReport,
) -> Result<String, DataStoreError> {
    let path = data_root.join("registry-migration-report.json");
    write_json(&path, report)?;
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

fn checksum_file(root: &Path, relative_path: &str) -> Result<String, DataStoreError> {
    std::fs::read(root.join(relative_path))
        .map(|bytes| bytes_checksum(&bytes))
        .map_err(io_error)
}
