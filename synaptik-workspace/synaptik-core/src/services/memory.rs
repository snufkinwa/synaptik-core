// src/services/memory.rs
//! Minimal single-writer memory store with linear DAG promotion.
//!
//! - Owns a single SQLite connection (WAL) to avoid multi-writer contention.
//! - Persists raw `content` plus optional `summary` and `reflection` text.
//! - Tracks cold-storage pointers (`archived_cid` + `archived_at`) for Archivist.
//! - Adds best-effort promotion helpers to write nodes into the DAG **linearly per lobe**.
//! - Leaves reads DB-first for MVP (no DAG reads/pruning in this pass).

use anyhow::{Context, Result, bail};
use blake3;
use chrono::Utc;
use rusqlite::Connection;
use serde_json::{json, Value};
use std::path::Path;
use std::time::Duration;

use crate::memory::dag;
use crate::config::SummarizerKind;
use crate::services::audit;
use crate::services::ethos::{Proposal, RuntimeDecision};
use crate::services::streamgate::Finalized;
use contracts::store::ContractsStore;
use once_cell::sync::OnceCell;

/// Memory is the single authority for writing to SQLite.
/// Expose `db` as `pub(crate)` if other services need read-only helpers internally.
pub struct Memory {
    pub(crate) db: Connection,
}

/// Minimal candidate record for compaction.
#[derive(Debug, Clone)]
pub struct MemoryCandidate {
    pub id: String,
    pub archived_cid: Option<String>,
}

impl Memory {
    /// Open/create the SQLite DB and ensure schema.
    ///
    /// Behavior:
    /// - Creates the parent directory if missing.
    /// - Opens SQLite and enables WAL (good for 1 writer + many readers).
    /// - Creates `memories` table and `(lobe, key)` index if they don't exist.
    pub fn open(db_path: &str) -> Result<Self> {
        if let Some(parent) = Path::new(db_path).parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating parent dir for {}", db_path))?;
        }

        let db =
            Connection::open(db_path).with_context(|| format!("opening sqlite at {}", db_path))?;

        db.busy_timeout(Duration::from_secs(5))?;

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
              archived_cid  TEXT,              -- content-addressed id from Archivist (blake3 hex)
              archived_at   TEXT               -- when it was archived (RFC3339 UTC)
            );

            CREATE INDEX IF NOT EXISTS idx_mem_lobe_key ON memories(lobe, key);
            "#,
        )?;

        Ok(Self { db })
    }

    // -------------------------------------------------------------------------
    // Hot-path writes
    // -------------------------------------------------------------------------

    /// Find an exact duplicate row by lobe and content bytes. Returns the latest matching memory_id.
    pub fn find_exact_duplicate_in_lobe(
        &self,
        lobe: &str,
        content: &[u8],
    ) -> Result<Option<String>> {
        let mut stmt = self.db.prepare(
            "SELECT memory_id FROM memories WHERE lobe=?1 AND content=?2 ORDER BY updated_at DESC LIMIT 1",
        )?;
        let mut rows = stmt.query((lobe, content))?;
        if let Some(row) = rows.next()? {
            let id: String = row.get(0)?;
            Ok(Some(id))
        } else {
            Ok(None)
        }
    }

    /// Bump the `updated_at` timestamp for a row.
    pub fn touch(&self, memory_id: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.db.execute(
            "UPDATE memories SET updated_at=?1 WHERE memory_id=?2",
            (&now, memory_id),
        )?;
        Ok(())
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

    /// Public: fetch (lobe, key) for a given memory_id, if present.
    pub fn lobe_key(&self, memory_id: &str) -> Result<Option<(String, String)>> {
        let mut stmt = self
            .db
            .prepare("SELECT lobe, key FROM memories WHERE memory_id=?1")?;
        let mut rows = stmt.query([memory_id])?;
        if let Some(row) = rows.next()? {
            let l: String = row.get(0)?;
            let k: String = row.get(1)?;
            Ok(Some((l, k)))
        } else {
            Ok(None)
        }
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

    // -------------------------------------------------------------------------
    // Reads / simple metadata
    // -------------------------------------------------------------------------

    /// Fetch raw `content` bytes by `memory_id`.
    ///
    /// Returns:
    /// - `Ok(Some(Vec<u8>))` if found,
    /// - `Ok(None)` if missing.
    pub fn recall(&self, memory_id: &str) -> Result<Option<Vec<u8>>> {
        let mut stmt = self
            .db
            .prepare("SELECT content FROM memories WHERE memory_id=?1")?;
        let mut rows = stmt.query([memory_id])?;
        if let Some(row) = rows.next()? {
            let bytes: Vec<u8> = row.get(0)?;
            return Ok(Some(bytes));
        }
        Ok(None)
    }

    /// Read the archived content id (CID) if this memory was promoted to cold storage.
    pub fn get_archived_cid(&self, memory_id: &str) -> Result<Option<String>> {
        let mut stmt = self
            .db
            .prepare("SELECT archived_cid FROM memories WHERE memory_id=?1")?;
        let mut rows = stmt.query([memory_id])?;
        if let Some(row) = rows.next()? {
            let cid: Option<String> = row.get(0)?;
            return Ok(cid);
        }
        Ok(None)
    }

    /// Mark a row as archived (set `archived_cid` and `archived_at`).
    ///
    /// Caller supplies:
    /// - `cid` (e.g., blake3 hex of content),
    /// - `archived_at` as RFC3339 UTC.
    pub fn mark_archived(&self, memory_id: &str, cid: &str, archived_at: &str) -> Result<()> {
        self.db.execute(
            "UPDATE memories SET archived_cid=?1, archived_at=?2 WHERE memory_id=?3",
            (cid, archived_at, memory_id),
        )?;
        Ok(())
    }

    /// Set/replace the reflection text after the fact and bump `updated_at`.
    pub fn set_reflection(&self, memory_id: &str, reflection: &str) -> Result<()> {
        self.db.execute(
            "UPDATE memories SET reflection=?1, updated_at=?2 WHERE memory_id=?3",
            (reflection, Utc::now().to_rfc3339(), memory_id),
        )?;
        Ok(())
    }

    /// Return all `memory_id`s that match an exact `(lobe, key)`.
    pub fn find_by_lobe_key(&self, lobe: &str, key: &str) -> Result<Vec<String>> {
        let mut stmt = self
            .db
            .prepare("SELECT memory_id FROM memories WHERE lobe=?1 AND key=?2")?;
        let iter = stmt.query_map((lobe, key), |row| row.get::<_, String>(0))?;
        Ok(iter.filter_map(|r| r.ok()).collect())
    }

    /// Fetch the most recent `summary` strings for a lobe (DESC by `updated_at`).
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

    // -------------------------------------------------------------------------
    // Promotion to DAG (linear per lobe) — MVP-safe, best-effort
    // -------------------------------------------------------------------------

    /// Internal: load a row we plan to promote.
    /// Returns (lobe, key, content, summary_opt, created_at, updated_at).
    fn load_row_for_promotion(
        &self,
        memory_id: &str,
    ) -> Result<(String, String, Vec<u8>, Option<String>, String, String)> {
        let mut stmt = self.db.prepare(
            "SELECT lobe, key, content, summary, created_at, updated_at
             FROM memories WHERE memory_id=?1",
        )?;
        let mut rows = stmt.query([memory_id])?;
        if let Some(row) = rows.next()? {
            let lobe: String = row.get(0)?;
            let key: String = row.get(1)?;
            let content: Vec<u8> = row.get(2)?;
            let summary: Option<String> = row.get(3)?;
            let created_at: String = row.get(4)?;
            let updated_at: String = row.get(5)?;
            Ok((lobe, key, content, summary, created_at, updated_at))
        } else {
            bail!("memory row not found: {memory_id}");
        }
    }

    /// Internal: latest archived CID in this lobe (used for linear parent).
    fn latest_archived_cid_in_lobe(&self, lobe: &str) -> Result<Option<String>> {
        let mut stmt = self.db.prepare(
            "SELECT archived_cid FROM memories
             WHERE lobe=?1 AND archived_cid IS NOT NULL
             ORDER BY archived_at DESC
             LIMIT 1",
        )?;
        let mut rows = stmt.query([lobe])?;
        if let Some(row) = rows.next()? {
            let cid: Option<String> = row.get(0)?;
            Ok(cid)
        } else {
            Ok(None)
        }
    }

    /// Promote a single memory row into the DAG, linking linearly within the lobe.
    ///
    /// MVP behavior:
    /// - Computes CID as blake3(content) hex and stores it in `archived_cid`.
    /// - Parent = latest previously archived CID in the same lobe (linear chain).
    /// - Writes a JSON `meta` with lobe/key/summary_len/cid/created_at/updated_at.
    /// - Calls dag::save_node(...). DAG write failures do **not** break the hot path.
    pub fn promote_to_dag(&self, memory_id: &str) -> Result<()> {
        let (lobe, key, content, summary_opt, created_at, updated_at) =
            self.load_row_for_promotion(memory_id)?;

        // CID = blake3(content)
        let cid = blake3::hash(&content).to_hex().to_string();
        let parent_cid = self.latest_archived_cid_in_lobe(&lobe)?;
        let parents = parent_cid.into_iter().collect::<Vec<_>>();

        // Small, stable metadata for the DAG
        let mut meta = json!({
            "cid": cid,
            "lobe": lobe,
            "key": key,
            "summary_len": summary_opt.as_deref().map(str::len).unwrap_or(0),
            "created_at": created_at,
            "updated_at": updated_at,
        });

        // If a capsule mapping exists for this memory row, include it in the DAG meta for provenance.
        if let Some(store) = contracts_store() {
            if let Ok(Some(caps_id)) = store.capsule_for_memory(memory_id) {
                if let Some(obj) = meta.as_object_mut() {
                    obj.insert("capsule_id".to_string(), Value::String(caps_id));
                }
            }
        }

        // Best-effort DAG write — never break memory hot path
        let _ = dag::save_node(
            memory_id,
            &String::from_utf8_lossy(&content),
            &meta,
            &parents,
        );

        // Mark row as archived
        let now = Utc::now().to_rfc3339();
        self.mark_archived(memory_id, &cid, &now)?;
        Ok(())
    }

    /// Promote all non-archived rows in a lobe, oldest→newest, to keep the chain linear.
    ///
    /// Returns list of (memory_id, archived_cid).
    pub fn promote_all_hot_in_lobe(&self, lobe: &str) -> Result<Vec<(String, String)>> {
        let mut stmt = self.db.prepare(
            "SELECT memory_id FROM memories
             WHERE lobe=?1 AND archived_cid IS NULL
             ORDER BY created_at ASC",
        )?;
        let ids = stmt
            .query_map([lobe], |row| row.get::<_, String>(0))?
            .filter_map(|r| r.ok())
            .collect::<Vec<_>>();
        drop(stmt);

        let mut out = Vec::with_capacity(ids.len());
        for id in &ids {
            self.promote_to_dag(id)?;
            if let Some(cid) = self.get_archived_cid(id)? {
                out.push((id.clone(), cid));
            }
        }
        Ok(out)
    }

    /// Promote the most recent non-archived row in a lobe (if any).
    /// Returns Some((memory_id, cid)) if one was promoted.
    pub fn promote_latest_hot_in_lobe(&self, lobe: &str) -> Result<Option<(String, String)>> {
        // Scope the read so the statement is dropped before we write.
        let id_opt: Option<String> = {
            let mut stmt = self.db.prepare(
                "SELECT memory_id FROM memories
                 WHERE lobe=?1 AND archived_cid IS NULL
                 ORDER BY created_at DESC
                 LIMIT 1",
            )?;
            let mut rows = stmt.query([lobe])?;
            if let Some(row) = rows.next()? {
                Some(row.get(0)?)
            } else {
                None
            }
        };

        if let Some(id) = id_opt {
            self.promote_to_dag(&id)?;
            if let Some(cid) = self.get_archived_cid(&id)? {
                return Ok(Some((id, cid)));
            }
        }
        Ok(None)
    }

    /// Remove exact-duplicate rows within a lobe, keeping the most recently updated copy of each unique content.
    /// Returns the number of rows deleted.
    pub fn prune_exact_duplicates_in_lobe(&self, lobe: &str) -> Result<usize> {
        // Load candidate rows (id, updated_at desc, content) to keep newest per hash.
        let mut stmt = self.db.prepare(
            "SELECT memory_id, content FROM memories WHERE lobe=?1 ORDER BY updated_at DESC",
        )?;
        let rows = stmt.query_map([lobe], |r| {
            let id: String = r.get(0)?;
            let content: Vec<u8> = r.get(1)?;
            Ok((id, content))
        })?;

        let mut seen: std::collections::HashSet<[u8; 32]> = std::collections::HashSet::new();
        let mut to_delete: Vec<String> = Vec::new();
        for row in rows {
            let (id, content) = row?;
            let h = blake3::hash(&content).as_bytes().to_owned();
            let h_arr: [u8; 32] = h.try_into().unwrap();
            if !seen.insert(h_arr) {
                to_delete.push(id);
            }
        }
        drop(stmt);

        if to_delete.is_empty() {
            return Ok(0);
        }

        let tx = self.db.unchecked_transaction()?;
        let mut del = tx.prepare("DELETE FROM memories WHERE memory_id=?1")?;
        let mut cnt = 0usize;
        for id in &to_delete {
            del.execute([id])?;
            cnt += 1;
        }
        del.finalize()?;
        tx.commit()?;
        Ok(cnt)
    }

    // -------------------------------------------------------------------------
    // Compaction helpers (minimal API for Compactor)
    // -------------------------------------------------------------------------

    /// Select candidate rows for compaction within a lobe.
    ///
    /// Strategy (minimal):
    /// - If `prefer_rarely_accessed` pick oldest by created_at (ASC), else newest by updated_at (DESC).
    /// - Return at most `top_k` rows.
    pub fn select_compaction_candidates(
        &self,
        lobe: &str,
        top_k: u32,
        prefer_rarely_accessed: bool,
    ) -> Result<Vec<MemoryCandidate>> {
        // Use fixed SQL variants to avoid interpolating ORDER BY dynamically.
        // This eliminates any possibility of SQL injection via ORDER BY fragments.
        const SQL_BY_CREATED: &str =
            "SELECT memory_id, archived_cid FROM memories WHERE lobe=?1 ORDER BY created_at ASC LIMIT ?2";
        const SQL_BY_UPDATED: &str =
            "SELECT memory_id, archived_cid FROM memories WHERE lobe=?1 ORDER BY updated_at DESC LIMIT ?2";

        let sql = if prefer_rarely_accessed { SQL_BY_CREATED } else { SQL_BY_UPDATED };
        let mut stmt = self.db.prepare(sql)?;
        let iter = stmt.query_map((lobe, top_k as i64), |row| {
            Ok(MemoryCandidate {
                id: row.get::<_, String>(0)?,
                archived_cid: row.get::<_, Option<String>>(1)?,
            })
        })?;
        let out: Vec<MemoryCandidate> = iter.filter_map(|r| r.ok()).collect();

        // Value-aware reordering: if a values table exists and has entries for these ids,
        // prefer lower-value items for compaction (stable to preserve base ordering otherwise).
        // Best-effort; silently ignores missing table.
        let mut with_scores: Vec<(Option<f32>, MemoryCandidate)> = Vec::with_capacity(out.len());
        for c in out.into_iter() {
            let score = self
                .db
                .prepare("SELECT value FROM \"values\" WHERE state_id=?1")
                .ok()
                .and_then(|mut st| st.query([&c.id]).ok().and_then(|mut rows| rows.next().ok().flatten().and_then(|row| row.get::<_, f32>(0).ok())));
            with_scores.push((score, c));
        }
        // Sort by score ascending (None last)
        with_scores.sort_by(|a, b| match (a.0, b.0) {
            (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        });
        Ok(with_scores.into_iter().map(|(_, c)| c).collect())
    }

    /// Fetch content as UTF-8 string for a given memory id.
    pub fn get_content(&self, memory_id: &str) -> Result<String> {
        let mut stmt = self
            .db
            .prepare("SELECT content FROM memories WHERE memory_id=?1")?;
        let mut rows = stmt.query([memory_id])?;
        if let Some(row) = rows.next()? {
            let bytes: Vec<u8> = row.get(0)?;
            return Ok(String::from_utf8_lossy(&bytes).into_owned());
        }
        bail!("memory row not found: {memory_id}")
    }

    /// Minimal raw accessor (same as `get_content` for MVP).
    pub fn get_raw(&self, memory_id: &str) -> Result<String> {
        self.get_content(memory_id)
    }

    /// Replace the `content` with a compacted summary, also store it in `summary` and bump timestamp.
    pub fn replace_with_summary(&self, memory_id: &str, summary: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.db.execute(
            "UPDATE memories SET content=?1, summary=?2, updated_at=?3 WHERE memory_id=?4",
            (summary.as_bytes(), summary, &now, memory_id),
        )?;
        Ok(())
    }

    /// Minimal built-in summarizer used by Compactor.
    /// Supports several simple modes without external dependencies.
    pub fn summarize(&self, kind: SummarizerKind, text: &str) -> Result<String> {
        fn split_sentences(s: &str) -> Vec<String> {
            s.split_terminator(|c| c == '.' || c == '!' || c == '?')
                .map(|p| p.trim())
                .filter(|p| !p.is_empty())
                .map(|p| format!("{}.", p))
                .collect()
        }
        fn first_lines(s: &str, n: usize) -> String {
            s.lines()
                .map(|l| l.trim())
                .filter(|l| !l.is_empty())
                .take(n)
                .collect::<Vec<_>>()
                .join(" \n")
        }
        let out = match kind {
            SummarizerKind::Heuristic => {
                let mut out = String::new();
                let mut sentences = 0usize;
                for sent in split_sentences(text) {
                    if !out.is_empty() { out.push(' '); }
                    out.push_str(&sent);
                    sentences += 1;
                    if sentences >= 2 || out.len() >= 400 { break; }
                }
                if out.is_empty() {
                    out = text.chars().take(400).collect();
                }
                out
            }
            SummarizerKind::Extractive => {
                let mut out = String::new();
                let mut sentences = 0usize;
                for sent in split_sentences(text) {
                    if !out.is_empty() { out.push(' '); }
                    out.push_str(&sent);
                    sentences += 1;
                    if sentences >= 3 || out.len() >= 600 { break; }
                }
                if out.is_empty() {
                    out = text.chars().take(600).collect();
                }
                out
            }
            SummarizerKind::Minimal => {
                let s = first_lines(text, 2);
                if s.is_empty() { text.lines().next().unwrap_or("").chars().take(160).collect() } else { s }
            }
            SummarizerKind::Compressive => {
                // Placeholder: behave like heuristic but tighter budget
                let mut out = String::new();
                let mut sentences = 0usize;
                for sent in split_sentences(text) {
                    if !out.is_empty() { out.push(' '); }
                    out.push_str(&sent);
                    sentences += 1;
                    if sentences >= 2 || out.len() >= 256 { break; }
                }
                if out.is_empty() {
                    out = text.chars().take(256).collect();
                }
                out
            }
        };
        Ok(out)
    }
}

// -------------------- Contracts Store helper --------------------

fn contracts_store() -> Option<&'static ContractsStore> {
    // Cache only successful initialization; failed attempts do NOT poison the cell, allowing retries.
    static CELL: OnceCell<ContractsStore> = OnceCell::new();
    if let Some(store) = CELL.get() { return Some(store); }
    let root_dir = match crate::commands::init::ensure_initialized_once() {
        Ok(r) => r.config.contracts.path.join("caps_store"),
        Err(_) => return None,
    };
    match ContractsStore::new(root_dir) {
        Ok(store) => {
            // Ignore set errors (race) and return the instance.
            let _ = CELL.set(store);
            CELL.get()
        }
        Err(_) => None,
    }
}

// -------------------------------------------------------------------------
// Replay helpers (thin wrappers over DAG)
// -------------------------------------------------------------------------

impl Memory {
    /// Recall an immutable snapshot by content-addressed id (blake3 hex).
    pub fn recall_snapshot(&self, snapshot_id: &str) -> Result<crate::memory::dag::MemoryState> {
        crate::memory::dag::recall_snapshot(snapshot_id)
    }

    /// Create or reset a named path diverging from the given snapshot.
    /// Returns the `path_id` (sanitized name).
    pub fn diverge_from(&self, snapshot_id: &str, path_name: &str) -> Result<String> {
        crate::memory::dag::diverge_from(snapshot_id, path_name)
    }

    /// Append a new immutable snapshot to a named path and advance its head.
    /// Returns the new snapshot id (blake3 hex).
    pub fn extend_path(
        &self,
        path_name: &str,
        state: crate::memory::dag::MemoryState,
    ) -> Result<String> {
        crate::memory::dag::extend_path(path_name, state)
    }

    /// Public helper: return the latest archived CID in a lobe, if any.
    pub fn latest_archived_cid_in_lobe_public(&self, lobe: &str) -> Result<Option<String>> {
        self.latest_archived_cid_in_lobe(lobe)
    }
}

// -------------------------------------------------------------------------
// Stream runtime write barrier (MVP: audit-only commit)
// -------------------------------------------------------------------------

/// Best-effort snapshot commit for stream runtime. For the MVP we only audit the commit
/// attempt rather than persisting the streamed output in SQLite.
pub fn commit_snapshot(
    proposal: &Proposal,
    decision: &RuntimeDecision,
    finalized: &Finalized,
) -> Result<()> {
    let status = match finalized.status {
        crate::services::streamgate::FinalizedStatus::Ok => "ok",
        crate::services::streamgate::FinalizedStatus::Violated => "violated",
        crate::services::streamgate::FinalizedStatus::Stopped => "stopped",
        crate::services::streamgate::FinalizedStatus::Escalated => "escalated",
    };
    audit::record_action(
        "streamruntime",
        "commit_snapshot",
        &serde_json::json!({
            "intent": proposal.intent,
            "decision": decision,
            "status": status,
            "preview": String::from(finalized.text.chars().take(160).collect::<String>()),
        }),
        "low",
    );
    Ok(())
}
