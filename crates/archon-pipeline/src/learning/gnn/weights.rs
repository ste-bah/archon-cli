//! Weight persistence with CozoDB-backed versioning.
//!
//! `WeightStore` layers an in-memory cache over CozoDB for versioned persistence.
//! `save_all` is atomic per version: all layers + biases written in one transaction,
//! version bumped only on success.
//!
//! `WeightManager` is the legacy CRC32 file-based persistence, retained for backward
//! compatibility.

mod errors;
mod legacy;
mod prng;
mod serialization;
mod store;
#[cfg(test)]
mod tests;
mod types;

pub use errors::{WeightError, WeightStoreError};
pub use legacy::WeightManager;
pub use prng::Xoshiro128StarStar;
pub use store::WeightStore;
pub use types::Initialization;
