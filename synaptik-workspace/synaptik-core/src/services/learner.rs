use anyhow::Result;
use rusqlite::{Connection, params};

use crate::commands::init::ensure_initialized_once;

/// Minimal TD(λ) learner (λ treated as 0 for now). Stores values in the shared SQLite DB.
pub struct TDLearner {
    db_path: std::path::PathBuf,
    pub gamma: f32,
    pub alpha: f32,
}

impl TDLearner {
    pub fn open_default() -> Result<Self> {
        let cfg = ensure_initialized_once()?.config.clone();
        Ok(Self {
            db_path: cfg.memory.cache_path,
            gamma: 0.95,
            alpha: 0.1,
        })
    }

    fn conn(&self) -> Result<Connection> {
        Ok(Connection::open(&self.db_path)?)
    }

    /// Construct a learner bound to a specific SQLite path (primarily for tests/tools).
    pub fn open_at(db_path: std::path::PathBuf) -> Self {
        Self {
            db_path,
            gamma: 0.95,
            alpha: 0.1,
        }
    }

    fn get_value(&self, state_id: &str) -> Result<f32> {
        let conn = self.conn()?;
        let mut stmt = match conn.prepare("SELECT value FROM \"values\" WHERE state_id=?1") {
            Ok(s) => s,
            Err(e) => {
                // Only treat a missing table as benign; propagate other errors.
                let msg = e.to_string();
                if msg.contains("no such table") {
                    return Ok(0.0);
                }
                return Err(e.into());
            }
        };
        let mut rows = stmt.query([state_id])?;
        if let Some(row) = rows.next()? {
            Ok(row.get::<_, f32>(0)?)
        } else {
            Ok(0.0)
        }
    }

    fn upsert_value(&self, state_id: &str, value: f32) -> Result<()> {
        let conn = self.conn()?;
        let now_ms = chrono::Utc::now().timestamp_millis();
        conn.execute(
            "INSERT INTO \"values\"(state_id, value, updated_ms) VALUES(?1, ?2, ?3)
             ON CONFLICT(state_id) DO UPDATE SET value=excluded.value, updated_ms=excluded.updated_ms",
            params![state_id, value, now_ms],
        )?;
        Ok(())
    }

    pub fn td_update(&self, s: &str, r: f32, sp: Option<&str>) -> Result<f32> {
        let v_s = self.get_value(s)?;
        let v_sp = match sp {
            Some(id) => self.get_value(id)?,
            None => 0.0,
        };
        let td_error = r + self.gamma * v_sp - v_s;
        let new_v = v_s + self.alpha * td_error;
        self.upsert_value(s, new_v)?;
        Ok(new_v)
    }
}

/// Step assembler: writes steps and triggers TD updates.
pub struct StepAssembler {
    db_path: std::path::PathBuf,
    learner: TDLearner,
}

impl StepAssembler {
    pub fn open_default() -> Result<Self> {
        let cfg = ensure_initialized_once()?.config.clone();
        Ok(Self {
            db_path: cfg.memory.cache_path.clone(),
            learner: TDLearner::open_default()?,
        })
    }

    fn conn(&self) -> Result<Connection> {
        Ok(Connection::open(&self.db_path)?)
    }

    /// Construct an assembler bound to a specific SQLite path (primarily for tests/tools).
    pub fn open_at(db_path: std::path::PathBuf) -> Result<Self> {
        Ok(Self {
            db_path: db_path.clone(),
            learner: TDLearner::open_at(db_path),
        })
    }

    pub fn record_step(
        &self,
        lobe: &str,
        state_id: &str,
        action_capsule_id: &str,
        reward: f32,
        next_state_id: Option<&str>,
        ts_ms: i64,
    ) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO steps (lobe, state_id, action_capsule_id, reward, next_state_id, ts_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![lobe, state_id, action_capsule_id, reward, next_state_id, ts_ms],
        )?;
        let _ = self.learner.td_update(state_id, reward, next_state_id)?;
        Ok(())
    }

    /// Convenience: derive next_state_id by finding the next Real memory in the same lobe after ts_ms.
    /// If none, treat as terminal (next_state_id=None).
    pub fn record_from_reward(
        &self,
        lobe: &str,
        state_id: &str,
        action_capsule_id: &str,
        reward: f32,
        ts_ms: i64,
    ) -> Result<()> {
        // Translate ts_ms to RFC3339 to compare with `updated_at` strings.
        let dt_utc = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ts_ms)
            .unwrap_or_else(|| chrono::Utc::now());
        let ts_rfc3339 = dt_utc.to_rfc3339();

        let conn = self.conn()?;
        // Find the next memory row in the same lobe strictly after this timestamp.
        let mut stmt = conn.prepare(
            "SELECT memory_id FROM memories WHERE lobe=?1 AND updated_at > ?2 ORDER BY updated_at ASC LIMIT 1",
        )?;
        let mut rows = stmt.query(params![lobe, ts_rfc3339])?;
        let next_state_opt: Option<String> = if let Some(row) = rows.next()? {
            Some(row.get(0)?)
        } else {
            None
        };

        match next_state_opt.as_deref() {
            Some(sprime) => self.record_step(
                lobe,
                state_id,
                action_capsule_id,
                reward,
                Some(sprime),
                ts_ms,
            ),
            None => self.record_step(lobe, state_id, action_capsule_id, reward, None, ts_ms),
        }
    }
}
