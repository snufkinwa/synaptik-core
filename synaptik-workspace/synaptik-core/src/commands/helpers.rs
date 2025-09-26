use anyhow::Result;

use crate::services::memory::Memory;

/// Tiny, deterministic keyword theme line (command-level helper).
pub fn compute_reflection(summaries: &[String], min_count: usize, max_keywords: usize) -> String {
    use std::collections::HashMap;
    const STOP: &[&str] = &[
        "the", "and", "for", "with", "that", "this", "from", "have", "are", "was", "were", "you",
        "your", "but", "not", "into", "over", "under", "then", "than", "there", "about", "just",
        "like", "they", "them", "their", "will", "would", "could", "has", "had", "can", "may",
        "might", "should",
    ];

    let mut freq: HashMap<String, usize> = HashMap::new();
    for s in summaries {
        for t in s.split(|c: char| !c.is_alphanumeric()) {
            let t = t.to_lowercase();
            if t.len() < 3 || STOP.contains(&t.as_str()) {
                continue;
            }
            *freq.entry(t).or_insert(0) += 1;
        }
    }
    let mut toks: Vec<(String, usize)> =
        freq.into_iter().filter(|(_, c)| *c >= min_count).collect();
    toks.sort_by(|a, b| b.1.cmp(&a.1));
    toks.truncate(max_keywords);
    if toks.is_empty() {
        return String::new();
    }
    let joined = toks
        .into_iter()
        .map(|(t, c)| format!("{t}({c})"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("Recurring themes: {joined}")
}

// ---------- tiny SQL helpers (read-only) ----------

pub fn latest_id_in_lobe(memory: &Memory, lobe: &str) -> Result<Option<String>> {
    let mut stmt = memory
        .db
        .prepare("SELECT memory_id FROM memories WHERE lobe=?1 ORDER BY updated_at DESC LIMIT 1")?;
    let mut rows = stmt.query([lobe])?;
    if let Some(r) = rows.next()? {
        let id: String = r.get(0)?;
        return Ok(Some(id));
    }
    Ok(None)
}

pub fn recent_ids_in_lobe(memory: &Memory, lobe: &str, limit: usize) -> Result<Vec<String>> {
    let mut stmt = memory.db.prepare(
        "SELECT memory_id
         FROM memories
         WHERE lobe = ?1
         ORDER BY updated_at DESC
         LIMIT ?2",
    )?;
    use std::convert::TryFrom;
    let limit_i64 = i64::try_from(limit).map_err(|_| anyhow::anyhow!("limit out of range for i64: {limit}"))?;
    let rows = stmt.query_map((lobe, limit_i64), |r| r.get::<_, String>(0))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

pub fn count_rows(memory: &Memory, lobe: Option<&str>) -> Result<u64> {
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

pub fn count_archived(memory: &Memory, lobe: Option<&str>) -> Result<u64> {
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

pub fn group_by_lobe(memory: &Memory, limit: usize) -> Result<Vec<(String, u64)>> {
    let mut stmt = memory.db.prepare(
        "SELECT lobe, COUNT(*) as c FROM memories GROUP BY lobe ORDER BY c DESC LIMIT ?1",
    )?;
    use std::convert::TryFrom;
    let limit_i64 = i64::try_from(limit).map_err(|_| anyhow::anyhow!("limit out of range for i64: {limit}"))?;
    let rows = stmt.query_map([limit_i64], |r| {
        let l: String = r.get(0)?;
        let c: i64 = r.get(1)?;
        Ok((l, c as u64))
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

pub fn max_updated(memory: &Memory) -> Result<Option<String>> {
    let mut stmt = memory.db.prepare("SELECT MAX(updated_at) FROM memories")?;
    let mut rows = stmt.query([])?;
    if let Some(r) = rows.next()? {
        let ts: Option<String> = r.get(0)?;
        return Ok(ts);
    }
    Ok(None)
}

