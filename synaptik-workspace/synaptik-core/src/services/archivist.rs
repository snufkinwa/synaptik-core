// src/services/archivist.rs
//! Archivist: content-addressed cold storage (filesystem only).
//!
//! - Files are stored under `<root>/<cid>` where `cid = blake3(bytes)`.
//! - **No database writes here.** Memory is the *only* SQLite writer.
//! - Librarian calls `archive(memory_id, bytes)` to get a CID, then
//!   calls `Memory::mark_archived(memory_id, cid, ts)`.
//!
//! MVP flow:
//!   remember → Memory
//!   promote  → Archivist::archive(bytes) → Memory::mark_archived(cid)
//!   fetch    → Memory.recall | (Archivist::retrieve → Memory.remember re-cache)

use anyhow::Result;
use blake3;
use chrono::Utc;
use serde_json::json;
use std::{fs, path::PathBuf};

use crate::services::audit::record_action;

/// Filesystem-backed content store (no DB).
#[derive(Debug, Clone)]
pub struct Archivist {
    /// Directory where blobs are written by CID, e.g. `.cogniv/archive/`
    root: PathBuf,
}

impl Archivist {
    // Conservative per-object size cap to avoid disk exhaustion from a single write.
    // Adjust when exposing as config. Textual memories typically fall well below this.
    const MAX_OBJECT_BYTES: usize = 16 * 1024 * 1024; // 16 MiB
    /// Initialize the archive root (idempotent). No DB handle.
    ///
    /// # Arguments
    /// * `root` — directory where blobs will be written, addressed by `cid`.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    /// Archive raw bytes and return the CID (content hash).
    ///
    /// # Arguments
    /// * `memory_id` — the originating row id in the `memories` table (for audit log context).
    /// * `bytes`     — raw content to store; `cid = blake3(bytes)`.
    ///
    /// # Returns
    /// * `Ok(cid)` — hex BLAKE3 content id.
    ///
    /// # Behavior
    /// - Computes `cid` from `bytes`.
    /// - Writes file `<root>/<cid>` **only if missing** (idempotent).
    /// - **Does not** touch SQLite. Caller (Librarian) must update Memory.
    pub fn archive(&self, memory_id: &str, bytes: &[u8]) -> Result<String> {
        if bytes.len() > Self::MAX_OBJECT_BYTES {
            anyhow::bail!(
                "archive object too large: {} bytes (max {})",
                bytes.len(),
                Self::MAX_OBJECT_BYTES
            );
        }
        let cid = blake3::hash(bytes).to_hex().to_string();

        // Write object once (idempotent)
        let path = self.root.join(&cid);
        if !path.exists() {
            fs::write(&path, bytes)?;
        }

        // Lightweight audit (for traceability)
        record_action(
            "archivist",
            "archive_write",
            &json!({
                "memory_id": memory_id,
                "cid": cid,
                "bytes": bytes.len(),
                "ts": Utc::now().to_rfc3339(),
            }),
            "low",
        );

        Ok(cid)
    }

    /// Retrieve bytes by CID (content-addressed).
    ///
    /// # Arguments
    /// * `cid` — hex BLAKE3 content id.
    ///
    /// # Returns
    /// * `Ok(Vec<u8>)` — the archived bytes.
    ///
    /// # Behavior
    /// - Reads `<root>/<cid>`.
    /// - Audits the read (can be removed if too chatty).
    pub fn retrieve(&self, cid: &str) -> Result<Vec<u8>> {
        let path = self.root.join(cid);
        let meta = fs::metadata(&path)?;
        if meta.len() > Self::MAX_OBJECT_BYTES as u64 {
            anyhow::bail!(
                "archived object too large to retrieve safely: {} bytes (max {})",
                meta.len(),
                Self::MAX_OBJECT_BYTES
            );
        }
        let bytes = fs::read(&path)?;

        record_action(
            "archivist",
            "archive_read",
            &json!({
                "cid": cid,
                "bytes": bytes.len(),
                "ts": Utc::now().to_rfc3339(),
            }),
            "low",
        );

        Ok(bytes)
    }
}
