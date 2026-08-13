use super::*;

#[derive(Debug, Clone)]
pub(super) struct RegistryMigration {
    pub registry: PersistentDatasetRegistry,
    pub report: RegistryMigrationReport,
}

pub(super) fn migrate_registry(
    root: &Path,
    data_root: &Path,
    registry: PersistentDatasetRegistry,
    write_reports: bool,
) -> Result<RegistryMigration, DataStoreError> {
    // v1 is the sole governed contract; "migration" now means validation and report repair.
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
    if !restore_migration_report_artifacts(root, data_root, &mut report)? && write_reports {
        report.report_path = Some(write_registry_migration_report(root, data_root, &report)?);
    }
    Ok(RegistryMigration { registry, report })
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
