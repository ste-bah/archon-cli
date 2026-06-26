//! Archon ingestion extensions (Port #2 — surgical, GPU-free bits).
//!
//! Currently: the token-aware, bbox-carrying chunker that replaces Archon's naive
//! `chunk_with_page_anchors` while preserving `page_start/page_end` lineage. Faithful
//! to the god-agent `markdown_chunker.py` (`chunk_marker_json`) per
//! `plans/archon-ingestion-ports-spec.md` §2 (Port C). Deterministic + unit-tested;
//! the Python-reference golden gate (S-1) remains for wire-in time.

pub mod chunk;
pub mod layout;
pub mod marker;
pub mod table;
