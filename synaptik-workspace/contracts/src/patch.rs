use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Minimal patch operations supported by the runtime and store.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum PatchOp {
    /// Case-insensitive text masking; pattern semantics are implementation-defined
    /// (typically a literal substring or simple glob) and applied by the consumer.
    MaskText { pattern: String },

    /// Swap a named artifact for an alternate CID (precomputed redaction/blur/etc.).
    SwapArtifact { name: String, cid: String },
}

/// A patch plan that may include text masks and/or alternate artifacts.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PatchPlan {
    pub id: String,
    /// Declarative operations; consumers (e.g., StreamGate/Librarian) decide how to apply.
    pub ops: Vec<PatchOp>,
    /// Convenience map from artifact name to alternate CID (if any).
    #[serde(default)]
    pub alt_artifacts: HashMap<String, String>,
}

