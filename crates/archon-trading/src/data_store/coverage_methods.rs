use super::*;

impl TradingDataLake {
    pub fn coverage_matrix(
        &self,
        universe: &str,
        generated_at: String,
    ) -> Result<CoverageMatrix, DataStoreError> {
        if universe != "trading-core-v1" {
            return Err(DataStoreError::InvalidMetadata(format!(
                "unsupported coverage universe: {universe}"
            )));
        }
        let registry = self.load_registry_migration(false)?.registry;
        let instruments = trading_core_instruments();
        let timeframes = trading_core_timeframes();
        let mut cells = Vec::new();
        let mut gaps = Vec::new();
        for instrument in &instruments {
            for timeframe in &timeframes {
                let cell = coverage_cell(self, &registry, instrument, timeframe, &generated_at);
                if !cell.available {
                    gaps.push(CoverageGap {
                        canonical_instrument: instrument.clone(),
                        timeframe: timeframe.clone(),
                        reason: cell
                            .fallback_reason
                            .clone()
                            .unwrap_or_else(|| "no production-eligible native dataset".into()),
                    });
                }
                cells.push(cell);
            }
        }
        Ok(CoverageMatrix {
            schema_version: "archon-trading-coverage-v1".into(),
            generated_at,
            instruments,
            timeframes,
            cells,
            gaps,
        })
    }

    pub fn write_coverage_matrix(
        &self,
        universe: &str,
        generated_at: String,
    ) -> Result<CoverageMatrix, DataStoreError> {
        let matrix = self.coverage_matrix(universe, generated_at)?;
        let coverage_dir = self.coverage_dir();
        write_json(&coverage_dir.join("latest.json"), &matrix)?;
        write_text(&coverage_dir.join("latest.md"), &coverage_markdown(&matrix))?;
        write_json(
            &coverage_dir
                .join("history")
                .join(format!("{}.json", safe_path(&matrix.generated_at))),
            &matrix,
        )?;
        Ok(matrix)
    }
}
