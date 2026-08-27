//! Provider-neutral staged-publication manifests and committed receipts.

use serde::{Deserialize, Serialize};

pub const PREPARED_PUBLICATION_SCHEMA_VERSION: u32 = 1;
pub const PUBLICATION_RECEIPT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreparedPublicationV1 {
    pub schema_version: u32,
    pub call_id: String,
    pub command_id: String,
    pub entries: Vec<PreparedPublicationEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreparedPublicationEntry {
    pub relative_path: String,
    pub byte_len: u64,
    pub blake3: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicationReceiptV1 {
    pub schema_version: u32,
    pub call_id: String,
    pub command_id: String,
    pub entries: Vec<PublishedArtifactReceipt>,
    pub committed_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishedArtifactReceipt {
    pub relative_path: String,
    pub destination_path: String,
    pub byte_len: u64,
    pub blake3: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prior_blake3: Option<String>,
}
