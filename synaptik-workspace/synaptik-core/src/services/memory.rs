// src/services/memory.rs
//! Minimal single-writer memory store.
//!
//! - Owns a single SQLite connection (WAL) to avoid multi-writer contention.
//! - Persists raw `content` plus optional `summary` and `reflection` text.
//! - Keeps cold-storage pointers (`archived_cid` + `archived_at`) for Archivist.
//! - Provides just enough read helpers to support Librarian + Commands.

use anyhow::Result;
use chrono::Utc;
use rusqlite::Connection;
use std::path::Path;

/// Memory is the single authority for writing to SQLite.
/// Expose `db` as `pub(crate)` if other services need read-only helpers internally.
pub struct Memory {
    pub(crate) db: Connection,
}

impl Memory {
    /// Open/create the SQLite DB and ensure schema.
    ///
    /// Behavior:
    /// - Creates the parent directory if missing.
    /// - Opens SQLite and enables WAL (good for 1 writer + many readers).
    /// - Creates `memories` table and `(lobe, key)` index if they don't exist.
    pub fn open(db_path: &str) -> Result<Self> {
        // Ensure parent dir exists (e.g., `.cogniv/cache/`)
        if let Some(parent) = Path::new(db_path).parent() {
            std::fs::create_dir_all(parent)?;
        }

        let db = Connection::open(db_path)?;

        // WAL reduces writer/reader blocking; safe for our single-writer design.
        db.execute_batch(
            r#"
            PRAGMA journal_mode = WAL;

            CREATE TABLE IF NOT EXISTS memories (
              memory_id     TEXT PRIMARY KEY,  -- stable id for the row
              lobe          TEXT NOT NULL,     -- logical bucket (e.g., "chat", "vision")
              key           TEXT NOT NULL,     -- caller-defined sub-key/path
              content       BLOB NOT NULL,     -- raw payload (bytes of text or binary)
              summary       TEXT,              -- 1–2 sentence summary (Librarian writes)
              reflection    TEXT,              -- tiny theme note (Commands.reflect writes)
              created_at    TEXT NOT NULL,     -- RFC3339 UTC
              updated_at    TEXT NOT NULL,     -- RFC3339 UTC
              archived_cid  TEXT,              -- content-addressed id from Archivist
              archived_at   TEXT               -- when it was archived (RFC3339 UTC)
            );

            -- Speeds up lookups by lobe/key (e.g., robotics "vision" channel).
            CREATE INDEX IF NOT EXISTS idx_mem_lobe_key ON memories(lobe, key);
            "#,
        )?;

        Ok(Self { db })
    }

    /// Upsert raw content only (no summary/reflect).
    ///
    /// - On INSERT: sets both timestamps to `now`.
    /// - On CONFLICT(memory_id): updates lobe/key/content and bumps `updated_at`.
    pub fn remember(&self, memory_id: &str, lobe: &str, key: &str, content: &[u8]) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.db.execute(
            r#"
            INSERT INTO memories(memory_id, lobe, key, content, created_at, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?5)
            ON CONFLICT(memory_id) DO UPDATE SET
              lobe       = excluded.lobe,
              key        = excluded.key,
              content    = excluded.content,
              updated_at = excluded.updated_at
            "#,
            (memory_id, lobe, key, content, &now),
        )?;
        Ok(())
    }

    /// Upsert raw content + summary (and optionally reflection).
    ///
    /// Notes:
    /// - `reflection` is **only overwritten** when a **non-empty** string is provided.
    ///   We use `COALESCE(NULLIF(excluded.reflection, ''), memories.reflection)`
    ///   to keep any existing reflection unless a new non-empty value is passed.
    pub fn remember_with_summary(
        &self,
        memory_id: &str,
        lobe: &str,
        key: &str,
        content: &[u8],
        summary: &str,
        reflection: Option<&str>,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.db.execute(
            r#"
            INSERT INTO memories(memory_id, lobe, key, content, summary, reflection, created_at, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)
            ON CONFLICT(memory_id) DO UPDATE SET
              lobe       = excluded.lobe,
              key        = excluded.key,
              content    = excluded.content,
              summary    = excluded.summary,
              -- Only replace reflection if the new value is non-empty; otherwise keep existing.
              reflection = COALESCE(NULLIF(excluded.reflection, ''), memories.reflection),
              updated_at = excluded.updated_at
            "#,
            (
                memory_id,
                lobe,
                key,
                content,
                summary,
                reflection.unwrap_or(""), // empty string = "do not overwrite"
                &now,
            ),
        )?;
        Ok(())
    }

    /// Fetch raw `content` bytes by `memory_id`.
    ///
    /// Returns:
    /// - `Ok(Some(Vec<u8>))` if found,
    /// - `Ok(None)` if missing.
    pub fn recall(&self, memory_id: &str) -> Result<Option<Vec<u8>>> {
        let mut stmt = self.db.prepare("SELECT content FROM memories WHERE memory_id=?1")?;
        let mut rows = stmt.query([memory_id])?;
        if let Some(row) = rows.next()? {
            let bytes: Vec<u8> = row.get(0)?;
            return Ok(Some(bytes));
        }
        Ok(None)
    }

    /// Read the archived content id (CID) if this memory was promoted to cold storage.
    pub fn get_archived_cid(&self, memory_id: &str) -> Result<Option<String>> {
        let mut stmt = self.db.prepare("SELECT archived_cid FROM memories WHERE memory_id=?1")?;
        let mut rows = stmt.query([memory_id])?;
        if let Some(row) = rows.next()? {
            let cid: Option<String> = row.get(0)?;
            return Ok(cid);
        }
        Ok(None)
    }

    /// Mark a row as archived (set `archived_cid` and `archived_at`).
    ///
    /// Caller (Librarian/Archivist flow) supplies:
    /// - `cid` from Archivist (e.g., blake3 hex),
    /// - `archived_at` as RFC3339 UTC.
    pub fn mark_archived(&self, memory_id: &str, cid: &str, archived_at: &str) -> Result<()> {
        self.db.execute(
            "UPDATE memories SET archived_cid=?1, archived_at=?2 WHERE memory_id=?3",
            (cid, archived_at, memory_id),
        )?;
        Ok(())
    }

    /// Set/replace the reflection text after the fact and bump `updated_at`.
    ///
    /// Used by the `reflect()` command once the one-line theme is computed.
    pub fn set_reflection(&self, memory_id: &str, reflection: &str) -> Result<()> {
        self.db.execute(
            "UPDATE memories SET reflection=?1, updated_at=?2 WHERE memory_id=?3",
            (reflection, Utc::now().to_rfc3339(), memory_id),
        )?;
        Ok(())
    }

    /// Return all `memory_id`s that match an exact `(lobe, key)`.
    ///
    /// Useful for deterministic lookups in structured lobes (e.g., robotics paths).
    pub fn find_by_lobe_key(&self, lobe: &str, key: &str) -> Result<Vec<String>> {
        let mut stmt = self.db.prepare("SELECT memory_id FROM memories WHERE lobe=?1 AND key=?2")?;
        let iter = stmt.query_map((lobe, key), |row| row.get::<_, String>(0))?;
        Ok(iter.filter_map(|r| r.ok()).collect())
    }

    /// Fetch the most recent `summary` strings for a lobe (DESC by `updated_at`).
    ///
    /// This powers:
    /// - prompt building (small high-signal context),
    /// - reflection (tiny theme computed from last N summaries).
    pub fn recent_summaries_by_lobe(&self, lobe: &str, limit: usize) -> Result<Vec<String>> {
        let mut stmt = self.db.prepare(
            "SELECT summary FROM memories
             WHERE lobe=?1 AND summary IS NOT NULL
             ORDER BY updated_at DESC
             LIMIT ?2",
        )?;
        let iter = stmt.query_map((lobe, limit as i64), |row| row.get::<_, String>(0))?;
        Ok(iter.filter_map(|r| r.ok()).collect())
    }
}
