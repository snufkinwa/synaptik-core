// src/services/mod.rs

pub mod archivist;
pub mod audit;
pub mod ethos; // used at the ingress (streamgate)
pub mod librarian; // thin router: summarize/reflect -> memory; optional promote via archivist
pub mod masking; // shared normalization + masking helpers
pub mod memory; // the ONLY SQLite writer // file-only cold store (CID <-> bytes)
// (Removed) learner & reward modules deprecated; streamgate retained for gating only.
pub mod streamgate; // streaming gate (no adaptive learner)

// Public API
pub use archivist::Archivist;
pub use ethos::{ConstraintSpec, EthosContract, Proposal, RuntimeDecision};
pub use librarian::Librarian;
pub use memory::Memory;
pub use streamgate::{
    Finalized, FinalizedStatus, GateDecision, GateError, LlmClient, StreamGate, StreamGateConfig,
    StreamRuntime, StreamingIndex,
};
