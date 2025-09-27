use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Reference to a binary artifact managed by PonsStore (content-addressed).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ArtifactRef {
    /// Logical name within the capsule (e.g., "frame_0001.png").
    pub name: String,
    /// Content ID from PonsStore (e.g., blake3 hex or other CID scheme).
    pub cid: Option<String>,
    /// Optional MIME/type hint (e.g., "image/png").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Optional size in bytes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    /// Optional extra metadata.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum CapsuleSource {
    #[default]
    Real,
    Sim,
    Derived,
}

/// Capsule metadata that is intrinsic to the experience unit.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CapsuleMeta {
    /// Globally-unique capsule id (uuidv7-like string). Filled at ingest time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capsule_id: Option<String>,

    /// Optional agent identifier (who produced this capsule).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,

    /// Logical lobe/bucket for the experience (e.g., "chat", "vision").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lobe: Option<String>,

    /// Milliseconds since Unix epoch for start/end timestamps.
    pub t_start_ms: u64,
    pub t_end_ms: u64,

    /// "real" | "sim" | "derived"
    pub source: CapsuleSource,

    /// Schema version for SimCapsule serialization.
    pub schema_ver: String,

    /// Optional integrity fields (filled by the store at ingest).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capsule_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issuer_signature: Option<String>,

    /// Optional parent capsule id (for derived/sim capsules).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
}

/// Atomic experience unit used by contracts for evaluation and gating.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SimCapsule {
    pub inputs: Value,
    pub context: Value,
    pub actions: Value,
    pub outputs: Value,
    pub trace: Value,
    pub artifacts: Vec<ArtifactRef>,
    pub meta: CapsuleMeta,
}

impl CapsuleSource {
    /// Return a stable kind string for policy branching.
    pub fn kind(&self) -> &'static str {
        match self {
            CapsuleSource::Real => "raw",
            CapsuleSource::Sim => "sim",
            CapsuleSource::Derived => "derived",
        }
    }
}
