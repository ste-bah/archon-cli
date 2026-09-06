pub mod openbb_polygon;
#[cfg(test)]
pub(crate) mod openbb_polygon_tests;
pub mod yfinance;
#[cfg(test)]
pub(crate) mod yfinance_tests;
// Re-export stooq from its canonical location under data_lake
// for consumers expecting it under providers::stooq (integration tests, stooq_ingest).
pub use crate::data_lake::stooq;
