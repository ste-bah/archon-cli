//! Stable capability identities for prepared freeze publication.

use super::*;

impl PreparedAcceptanceFreeze {
    pub(crate) fn publication_identity(&self) -> String {
        publication_identity(vec![
            self.tasks_root.to_string_lossy().as_bytes().to_vec(),
            self.project_root.to_string_lossy().as_bytes().to_vec(),
            self.contract_bytes.clone(),
            serde_json::to_vec(&self.lock).expect("acceptance lock serializes"),
            serde_json::to_vec(&self.pin).expect("acceptance pin serializes"),
        ])
    }
}

impl PreparedSkeletonFreeze {
    pub(crate) fn publication_identity(&self) -> String {
        publication_identity(vec![
            self.tasks_root.to_string_lossy().as_bytes().to_vec(),
            self.pin_path.to_string_lossy().as_bytes().to_vec(),
            self.skeleton_bytes.clone(),
            serde_json::to_vec(&self.lock).expect("skeleton lock serializes"),
            serde_json::to_vec(&self.pin).expect("acceptance pin serializes"),
        ])
    }
}

fn publication_identity(parts: Vec<Vec<u8>>) -> String {
    let mut framed = Vec::new();
    for part in parts {
        framed.extend_from_slice(&(part.len() as u64).to_le_bytes());
        framed.extend_from_slice(&part);
    }
    content_digest(&framed)
}
