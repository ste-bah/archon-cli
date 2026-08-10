//! CozoDB CRUD operations for document artefacts.
//!
//! Each function accepts a `&DbInstance` and the relevant model struct,
//! and performs insert or query operations against the canonical Cozo relations.

mod artifacts;
mod chunks;
mod common;
mod counts;
mod documents;
mod embeddings;
mod images;
mod locators;
mod pages;

pub use artifacts::*;
pub use chunks::*;
pub use counts::*;
pub use documents::*;
pub use embeddings::*;
pub use images::*;
pub use locators::*;
pub use pages::*;

#[cfg(test)]
mod hash_reservation_test_hooks;
#[cfg(test)]
mod hash_reservation_tests;
#[cfg(test)]
mod tests;
