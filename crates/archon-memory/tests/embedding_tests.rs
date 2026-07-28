//! Integration tests for the embedding / vector search / hybrid search subsystem.
//!
//! These tests use CozoDB in-memory backend and synthetic embeddings so they
//! run without network access or model downloads.

use std::sync::Arc;

use archon_memory::embedding::{
    EmbeddingConfig, EmbeddingProvider, EmbeddingProviderKind, create_provider,
};
use archon_memory::graph::MemoryGraph;
use archon_memory::hybrid_search;
use archon_memory::types::{MemoryError, MemoryType};
use archon_memory::vector_search;

#[path = "embedding_tests/graph_tests.rs"]
mod graph_tests;
#[path = "embedding_tests/hybrid_tests.rs"]
mod hybrid_tests;
#[path = "embedding_tests/openai_tests.rs"]
mod openai_tests;
#[path = "embedding_tests/provider_tests.rs"]
mod provider_tests;
#[path = "embedding_tests/support.rs"]
mod support;
#[path = "embedding_tests/vector_tests.rs"]
mod vector_tests;
