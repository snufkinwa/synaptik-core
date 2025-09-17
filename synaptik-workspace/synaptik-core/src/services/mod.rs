// src/services/mod.rs

pub mod archivist;
pub mod audit;
pub mod ethos; // used at the ingress (streamgate)
pub mod librarian; // thin router: summarize/reflect -> memory; optional promote via archivist
pub mod memory; // the ONLY SQLite writer // file-only cold store (CID <-> bytes)

// Public API
pub use archivist::Archivist;
pub use librarian::Librarian;
pub use memory::Memory;
