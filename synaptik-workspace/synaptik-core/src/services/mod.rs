// src/services/mod.rs

pub mod audit;
pub mod ethos;        // used at the ingress (streamgate)
pub mod librarian;    // thin router: summarize/reflect -> memory; optional promote via archivist
pub mod memory;       // the ONLY SQLite writer
pub mod archivist;    // file-only cold store (CID <-> bytes)
pub mod lobes;        // FS-only versioned bucket (no DB)

// Public API
pub use archivist::Archivist;
pub use memory::Memory;
pub use librarian::Librarian;
pub use lobes::LobeStore;
