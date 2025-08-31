// src/commands/mod.rs
use anyhow::{anyhow, Result};
use serde::Serialize;
use serde_json::json;

use crate::services::archivist::Archivist;
use crate::services::audit::record_action;
use crate::services::ethos::{decision_gate, precheck, Decision};
use crate::services::librarian::Librarian;
use crate::services::memory::Memory;

use crate::commands::init::ensure_initialized_once;

pub struct Commands {
    memory: Memory,       // one SQLite connection here
    librarian: Librarian, // no DB inside
}

impl Commands {
    // Keep the signature for now; ignore the args. Prefix with _ to silence warnings.
    pub fn new(_db_path: &str, _archivist: Option<Archivist>) -> Result<Self> {
        let report = ensure_initialized_once()?;

        // Hard-coded canonical locations
        let cache_db = report.root.join("cache").join("memory.db");
        let archive_root = report.root.join("archive");

        // If Memory::open takes &str:
        let memory = Memory::open(
            cache_db
                .to_str()
                .ok_or_else(|| anyhow!("invalid UTF-8 db path"))?,
        )?;

        // Pass by value (impl Into<PathBuf>)
        let archivist = Archivist::open(archive_root)?;
        let librarian = Librarian::new(Some(archivist));

        // Build directly (since from_parts doesn't exist)
        Ok(Self { memory, librarian })
    }


    pub fn remember(&self, lobe: &str, key: Option<&str>, content: &str) -> Result<String> {
        record_action("commands", "remember_called", &json!({"lobe": lobe, "key_is_some": key.is_some()}), "low");

        let v = precheck(content, "memory_storage").map_err(|e| anyhow!("ethos precheck error: {e}"))?;
        match decision_gate(&v) {
            Decision::Block => {
                record_action("commands", "remember_blocked", &json!({"reason": v.reason, "risk": v.risk}), "high");
                return Err(anyhow!("blocked by ethics: {}", v.reason));
            }
            Decision::AllowWithConstraints => {
                record_action("commands", "remember_constraints", &json!({"constraints": v.constraints, "risk": v.risk}), "medium");
            }
            Decision::Allow => {}
        }

        let id = self.librarian.ingest_text(&self.memory, lobe, key, content)?;
        record_action("commands", "remember_stored", &json!({"id": id, "lobe": lobe}), "low");
        Ok(id)
    }

    pub fn reflect(&self, lobe: &str, window: usize) -> Result<String> {
        record_action("commands", "reflect_called", &json!({"lobe": lobe, "window": window}), "low");

        let pool = self.memory.recent_summaries_by_lobe(lobe, window)?;
        let note = compute_reflection(&pool, 3, 3);
        if note.is_empty() {
            record_action("commands", "reflect_noop", &json!({"reason": "no_summaries"}), "low");
            return Ok(String::new());
        }

        let v = precheck(&note, "reflection_update").map_err(|e| anyhow!("ethos precheck error: {e}"))?;
        if matches!(decision_gate(&v), Decision::Block) {
            record_action("commands", "reflect_blocked", &json!({"reason": v.reason, "risk": v.risk}), "high");
            return Ok(String::new());
        }

        if let Some(id) = latest_id_in_lobe(&self.memory, lobe)? {
            self.memory.set_reflection(&id, &note)?;
            record_action("commands", "reflect_set", &json!({"id": id}), "low");
        } else {
            record_action("commands", "reflect_noop", &json!({"reason": "no_rows_in_lobe"}), "low");
        }
        Ok(note)
    }

    pub fn stats(&self, lobe: Option<&str>) -> Result<Stats> {
        record_action("commands", "stats_called", &json!({"lobe": lobe}), "low");
        let _ = precheck("stats_request", "metadata_access");

        let total = count_rows(&self.memory, lobe)?;
        let archived = count_archived(&self.memory, lobe)?;
        let by_lobe = group_by_lobe(&self.memory, 20)?;
        let last_updated = max_updated(&self.memory)?;
        record_action("commands", "stats_returned", &json!({"total": total, "archived": archived}), "low");

        Ok(Stats { total, archived, by_lobe, last_updated })
    }
}

/// Tiny, deterministic keyword theme line (command-level helper).
fn compute_reflection(summaries: &[String], min_count: usize, max_tokens: usize) -> String {
    use std::collections::HashMap;
    const STOP: &[&str] = &[
        "the","and","for","with","that","this","from","have","are","was","were","you","your",
        "but","not","into","over","under","then","than","there","about","just","like","they",
        "them","their","will","would","could","has","had","can","may","might","should"
    ];

    let mut freq: HashMap<String, usize> = HashMap::new();
    for s in summaries {
        for t in s.split(|c: char| !c.is_alphanumeric()) {
            let t = t.to_lowercase();
            if t.len() < 3 || STOP.contains(&t.as_str()) { continue; }
            *freq.entry(t).or_insert(0) += 1;
        }
    }
    let mut toks: Vec<(String, usize)> = freq.into_iter().filter(|(_, c)| *c >= min_count).collect();
    toks.sort_by(|a, b| b.1.cmp(&a.1));
    toks.truncate(max_tokens);
    if toks.is_empty() { return String::new(); }
    let joined = toks.into_iter().map(|(t,c)| format!("{t}({c})")).collect::<Vec<_>>().join(", ");
    format!("Recurring themes: {joined}")
}


#[derive(Debug, Serialize)]
pub struct Stats {
    pub total: u64,
    pub archived: u64,
    pub by_lobe: Vec<(String, u64)>,
    pub last_updated: Option<String>,
}

// ---------- tiny SQL helpers (read-only) ----------

fn latest_id_in_lobe(memory: &Memory, lobe: &str) -> Result<Option<String>> {
    let mut stmt = memory.db.prepare(
        "SELECT memory_id FROM memories WHERE lobe=?1 ORDER BY updated_at DESC LIMIT 1"
    )?;
    let mut rows = stmt.query([lobe])?;
    if let Some(r) = rows.next()? {
        let id: String = r.get(0)?;
        return Ok(Some(id));
    }
    Ok(None)
}

fn count_rows(memory: &Memory, lobe: Option<&str>) -> Result<u64> {
    let sql = match lobe {
        Some(_) => "SELECT COUNT(*) FROM memories WHERE lobe=?1",
        None => "SELECT COUNT(*) FROM memories",
    };
    let mut stmt = memory.db.prepare(sql)?;
    let cnt: i64 = match lobe {
        Some(l) => stmt.query_row([l], |r| r.get(0))?,
        None => stmt.query_row([], |r| r.get(0))?,
    };
    Ok(cnt as u64)
}

fn count_archived(memory: &Memory, lobe: Option<&str>) -> Result<u64> {
    let sql = match lobe {
        Some(_) => "SELECT COUNT(*) FROM memories WHERE lobe=?1 AND archived_cid IS NOT NULL",
        None => "SELECT COUNT(*) FROM memories WHERE archived_cid IS NOT NULL",
    };
    let mut stmt = memory.db.prepare(sql)?;
    let cnt: i64 = match lobe {
        Some(l) => stmt.query_row([l], |r| r.get(0))?,
        None => stmt.query_row([], |r| r.get(0))?,
    };
    Ok(cnt as u64)
}

fn group_by_lobe(memory: &Memory, limit: usize) -> Result<Vec<(String, u64)>> {
    let mut stmt = memory.db.prepare(
        "SELECT lobe, COUNT(*) as c FROM memories GROUP BY lobe ORDER BY c DESC LIMIT ?1"
    )?;
    let rows = stmt.query_map([limit as i64], |r| {
        let l: String = r.get(0)?;
        let c: i64 = r.get(1)?;
        Ok((l, c as u64))
    })?;
    let mut out = Vec::new();
    for r in rows { out.push(r?); }
    Ok(out)
}

fn max_updated(memory: &Memory) -> Result<Option<String>> {
    let mut stmt = memory.db.prepare("SELECT MAX(updated_at) FROM memories")?;
    let mut rows = stmt.query([])?;
    if let Some(r) = rows.next()? {
        let ts: Option<String> = r.get(0)?;
        return Ok(ts);
    }
    Ok(None)
}
