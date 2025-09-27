use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use serde::Serialize;
use std::path::PathBuf;

use crate::commands::init::ensure_initialized_once;
use contracts::api::{CapsAnnot, Verdict};

#[derive(Debug, Clone, Serialize)]
pub struct RewardEvent {
    pub lobe: String,
    pub capsule_id: String,
    pub parent_id: Option<String>,
    pub value: f32,
    pub ts_ms: i64,
    pub labels: Vec<String>,
    pub verdict: String,
    pub risk: f32,
}

pub trait RewardSink {
    fn publish(&self, ev: &RewardEvent) -> Result<()>;
}

pub struct RewardSqliteSink {
    db_path: PathBuf,
}

impl RewardSqliteSink {
    pub fn open_default() -> Result<Self> {
        let cfg = ensure_initialized_once()?.config.clone();
        let db_path = cfg.memory.cache_path;
        let sink = Self { db_path };
        sink.init_schema()?;
        Ok(sink)
    }

    fn conn(&self) -> Result<Connection> {
        Connection::open(&self.db_path).with_context(|| format!("open sqlite at {:?}", self.db_path))
    }

    fn init_schema(&self) -> Result<()> {
        let conn = self.conn()?;
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS rewards (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                ts_ms INTEGER NOT NULL,
                lobe TEXT NOT NULL,
                capsule_id TEXT NOT NULL,
                parent_id TEXT,
                value REAL NOT NULL,
                verdict TEXT NOT NULL,
                risk REAL NOT NULL,
                labels_json TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS values (
                state_id TEXT PRIMARY KEY,
                value REAL NOT NULL,
                updated_ms INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS steps (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                lobe TEXT NOT NULL,
                state_id TEXT,
                action_capsule_id TEXT NOT NULL,
                reward REAL NOT NULL,
                next_state_id TEXT,
                ts_ms INTEGER NOT NULL
            );
            "#,
        )?;
        Ok(())
    }
}

impl RewardSink for RewardSqliteSink {
    fn publish(&self, ev: &RewardEvent) -> Result<()> {
        let conn = self.conn()?;
        let labels_json = serde_json::to_string(&ev.labels).unwrap_or_else(|_| "[]".to_string());
        conn.execute(
            "INSERT INTO rewards (ts_ms, lobe, capsule_id, parent_id, value, verdict, risk, labels_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![ev.ts_ms, ev.lobe, ev.capsule_id, ev.parent_id, ev.value, ev.verdict, ev.risk, labels_json],
        )?;
        Ok(())
    }
}

/// Map a contracts annotation into a scalar reward.
pub fn reward_from_annotation(ann: &CapsAnnot) -> f32 {
    let base = match ann.verdict {
        Verdict::Allow => 0.2,
        Verdict::AllowWithPatch => 0.1,
        Verdict::Quarantine => -0.5,
    };
    // Risk penalty: normalized 0..1 -> negative contribution up to -1.0
    let risk_penalty = -ann.risk.max(0.0).min(1.0);
    // Simple label shapers
    let mut bonus = 0.0;
    if ann.labels.iter().any(|l| l.eq_ignore_ascii_case("success")) {
        bonus += 1.0;
    }
    if ann.labels.iter().any(|l| l.eq_ignore_ascii_case("patched")) {
        bonus += 0.05; // tiny nudge for patched successes
    }
    base + bonus + risk_penalty
}

