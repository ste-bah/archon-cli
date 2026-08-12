//! Outbound-body bookkeeping for the #171 fixture.
//!
//! Two ways to record the same bodies, so one fixture can answer two
//! questions that pull in opposite directions:
//!
//! * **Byte identity** needs every body kept, to be written out and diffed
//!   against a committed capture.
//! * **Peak RSS** needs the opposite — a harness that holds twenty copies of a
//!   400KB transcript owns the high-water mark, and whatever the runner does
//!   with its own allocations disappears underneath it.
//!
//! [`BodyRecord`] is the switch. Both arms build the snapshot the same way and
//! the digest arm serializes it with the same function that writes the capture
//! file, so the digest is exactly the capture file's body bytes — an arm that
//! quietly stopped sending something would change it.

use archon_llm::provider::LlmRequest;
use sha2::{Digest, Sha256};

/// The provider-facing body shape, in the same field order every time.
pub fn request_snapshot(request: &LlmRequest) -> serde_json::Value {
    serde_json::json!({
        "model": &request.model,
        "max_tokens": request.max_tokens,
        "system": &request.system,
        "messages": &request.messages,
        "tools": request.tools.as_ref(),
        "thinking": &request.thinking,
        "speed": &request.speed,
        "effort": &request.effort,
        "extra": &request.extra,
        "request_origin": &request.request_origin,
        "reasoning_encrypted": &request.reasoning_encrypted,
    })
}

/// One captured body exactly as the capture file records it.
pub fn snapshot_bytes(snapshot: &serde_json::Value) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(snapshot).expect("serialize snapshot");
    bytes.push(b'\n');
    bytes
}

/// A running digest over captured bodies.
pub struct BodyDigest {
    bodies: u32,
    bytes: u64,
    hasher: Sha256,
}

impl BodyDigest {
    fn new() -> Self {
        Self {
            bodies: 0,
            bytes: 0,
            hasher: Sha256::new(),
        }
    }

    fn fold(&mut self, bytes: &[u8]) {
        self.bodies += 1;
        self.bytes += bytes.len() as u64;
        self.hasher.update(bytes);
    }

    fn summary(&self) -> BodySummary {
        BodySummary {
            bodies: self.bodies,
            bytes: self.bytes,
            sha256: format!("{:x}", self.hasher.clone().finalize()),
        }
    }
}

/// What a fixture run sent, reduced to numbers that fit in a log line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BodySummary {
    pub bodies: u32,
    pub bytes: u64,
    pub sha256: String,
}

impl std::fmt::Display for BodySummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "bodies={} bytes={} sha256={}",
            self.bodies, self.bytes, self.sha256
        )
    }
}

/// What the fixture keeps from each outbound body.
pub enum BodyRecord {
    /// Every body, for the byte-identity capture (#75 evidence convention).
    Retained(Vec<serde_json::Value>),
    /// Only the running digest. The body is built exactly as the retained arm
    /// builds it and serialized with the same function that writes the capture
    /// file, then dropped — the serialization the retained arm defers to
    /// `write_capture` simply happens inline, so this arm does no less work.
    Digested(BodyDigest),
}

impl BodyRecord {
    pub fn retaining() -> Self {
        Self::Retained(Vec::new())
    }

    pub fn digesting() -> Self {
        Self::Digested(BodyDigest::new())
    }

    pub fn record(&mut self, snapshot: serde_json::Value) {
        match self {
            Self::Retained(bodies) => bodies.push(snapshot),
            Self::Digested(digest) => digest.fold(&snapshot_bytes(&snapshot)),
        }
    }

    /// Every retained body, in send order.
    pub fn snapshots(&self) -> Vec<serde_json::Value> {
        match self {
            Self::Retained(bodies) => bodies.clone(),
            Self::Digested(_) => {
                panic!("the digesting fixture keeps no bodies; use `summary`")
            }
        }
    }

    /// The digest of everything sent, computed either way.
    pub fn summary(&self) -> BodySummary {
        match self {
            Self::Retained(bodies) => {
                let mut digest = BodyDigest::new();
                for snapshot in bodies {
                    digest.fold(&snapshot_bytes(snapshot));
                }
                digest.summary()
            }
            Self::Digested(digest) => digest.summary(),
        }
    }
}
