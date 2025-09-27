// src/services/mod.rs

pub mod archivist;
pub mod audit;
pub mod ethos; // used at the ingress (streamgate)
pub mod librarian; // thin router: summarize/reflect -> memory; optional promote via archivist
pub mod memory; // the ONLY SQLite writer // file-only cold store (CID <-> bytes)
pub mod streamgate;
pub mod compactor; // summarization + safety gate + replace
pub mod masking;   // shared normalization + masking helpers
pub mod reward;    // reward bus / sink for online learning hooks
pub mod learner;   // step assembler + TD micro-learner

// Public API
pub use archivist::Archivist;
pub use librarian::Librarian;
pub use memory::Memory;
pub use streamgate::{
    Finalized, FinalizedStatus, GateDecision, GateError, LlmClient, StreamGate, StreamGateConfig,
    StreamRuntime, StreamingIndex,
};
pub use ethos::{ConstraintSpec, EthosContract, Proposal, RuntimeDecision};
