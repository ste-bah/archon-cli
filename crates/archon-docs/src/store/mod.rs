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
mod pages;

pub use artifacts::*;
pub use chunks::*;
pub use counts::*;
pub use documents::*;
pub use embeddings::*;
pub use images::*;
pub use pages::*;

#[cfg(test)]
mod tests;
