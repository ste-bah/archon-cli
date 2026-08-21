use super::*;

pub(super) const REGISTRY_SCHEMA_V1: &str = "archon-trading-data-registry-v1";
/// Retained only so existing v2 files fail with an explicit unsupported-schema value.
pub(super) const REGISTRY_SCHEMA_V2: &str = "archon-trading-data-registry-v2";

impl TradingDataLake {
    pub fn load_registry(&self) -> Result<PersistentDatasetRegistry, DataStoreError> {
        self.load_verified_registry()
    }

    pub(super) fn load_verified_registry(
        &self,
    ) -> Result<PersistentDatasetRegistry, DataStoreError> {
        let mut registry = self.load_registry_migration(false)?.registry;
        for record in registry.datasets.values_mut() {
            verify_artifacts(&self.root, record)?;
            let validation =
                read_json::<ValidationReport>(&self.root.join(&record.validation_path));
            let quarantined = dataset_is_quarantined(&self.root, record);
            let production_eligible =
                !quarantined && registry_record_allows_production(record, validation.as_ref());
            let status = if production_eligible {
                DatasetStatus::Healthy
            } else {
                DatasetStatus::Degraded
            };
            record.production_eligible = production_eligible;
            record.status = status;
        }
        Ok(registry)
    }

    pub(super) fn load_registry_migration(
        &self,
        write_reports: bool,
    ) -> Result<RegistryMigration, DataStoreError> {
        let path = self.registry_path();
        if !path.exists() {
            let registry = PersistentDatasetRegistry::default();
            if write_reports {
                write_schema_json(&path, &registry)?;
            }
            return Ok(RegistryMigration {
                registry,
                report: RegistryMigrationReport {
                    schema_version: REGISTRY_SCHEMA_V1.into(),
                    ..RegistryMigrationReport::default()
                },
            });
        }
        let registry: PersistentDatasetRegistry = read_json(&path)?;
        migrate_registry(&self.root, &self.data_root(), registry, write_reports)
    }
}

pub(super) fn registry_record_allows_production(
    record: &StoredDatasetRecord,
    validation: Result<&ValidationReport, &DataStoreError>,
) -> bool {
    record.native_interval
        && record.production_eligible
        && !record.provider.trim().eq_ignore_ascii_case("yfinance")
        && validation.is_ok_and(ValidationReport::allows_production)
}
