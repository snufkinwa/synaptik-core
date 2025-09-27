use anyhow::Result;
use crate::commands::{HitSource, Prefer, RecallResult, bytes_to_string_owned, Commands};

impl Commands {
    /// Newest → oldest memory_ids for a lobe.
    pub fn recent(&self, lobe: &str, n: usize) -> Result<Vec<String>> {
        super::helpers::recent_ids_in_lobe(&self.memory, lobe, n)
    }

    /// Recall full text (auto: hot → archive → dag). Returns just the content string.
    pub fn recall(&self, memory_id: &str) -> Result<Option<String>> {
        Ok(self.recall_any(memory_id, Prefer::Auto)?.map(|r| r.content))
    }

    /// Layered recall returning which source was used. prefer: "hot"|"archive"|"dag"|"auto"
    pub fn recall_with_source(
        &self,
        memory_id: &str,
        prefer: Option<&str>,
    ) -> Result<Option<(String, String)>> {
        Ok(self.recall_any(memory_id, parse_prefer(prefer))?.map(|r| {
            let src = match r.source {
                HitSource::Hot => "hot",
                HitSource::Archive => "archive",
                HitSource::Dag => "dag",
            };
            (r.content, src.to_string())
        }))
    }

    /// Bulk alias: for each id, attempt multi-tier recall and include id, content, and source.
    /// Returns Vec of (id, content, source) for all ids that could be recalled.
    pub fn total_recall_many(
        &self,
        memory_ids: &[String],
        prefer: Option<&str>,
    ) -> Result<Vec<(String, String, String)>> {
        let hits = self.recall_many(memory_ids, parse_prefer(prefer))?;
        Ok(hits
            .into_iter()
            .map(|r| {
                let src = match r.source {
                    HitSource::Hot => "hot",
                    HitSource::Archive => "archive",
                    HitSource::Dag => "dag",
                }
                .to_string();
                (r.memory_id, r.content, src)
            })
            .collect())
    }

    /// Centralized recall: one function to rule them all.
    /// Tries according to `Prefer`, returns the first hit with its source.
    pub fn recall_any(&self, memory_id: &str, prefer: Prefer) -> Result<Option<RecallResult>> {
        use Prefer::*;
        let order: &[Prefer] = match prefer {
            Hot => &[Hot],
            Archive => &[Archive],
            Dag => &[Dag],
            Auto => &[Hot, Archive, Dag],
        };

        for tier in order {
            match tier {
                Prefer::Hot => {
                    if let Some(bytes) = self.memory.recall(memory_id)? {
                        return Ok(Some(RecallResult {
                            memory_id: memory_id.to_owned(),
                            content: bytes_to_string_owned(bytes),
                            source: HitSource::Hot,
                        }));
                    }
                }
                Prefer::Archive => {
                    if let Some(bytes) = self.librarian.fetch_cold(&self.memory, memory_id)? {
                        return Ok(Some(RecallResult {
                            memory_id: memory_id.to_owned(),
                            content: bytes_to_string_owned(bytes),
                            source: HitSource::Archive,
                        }));
                    }
                    if let Some(_cid) = self.ensure_archive_for(memory_id)? {
                        if let Some(bytes2) = self.librarian.fetch_cold(&self.memory, memory_id)? {
                            return Ok(Some(RecallResult {
                                memory_id: memory_id.to_owned(),
                                content: bytes_to_string_owned(bytes2),
                                source: HitSource::Archive,
                            }));
                        }
                    }
                }
                Prefer::Dag => {
                    if let Some(s) = crate::memory::dag::content_by_id(memory_id)? {
                        return Ok(Some(RecallResult {
                            memory_id: memory_id.to_owned(),
                            content: s,
                            source: HitSource::Dag,
                        }));
                    }
                    // If DAG missing: ensure hot is present (restore from archive if needed), then promote this id to DAG
                    if self.memory.recall(memory_id)?.is_none() {
                        let _ = self.librarian.fetch_cold(&self.memory, memory_id)?;
                    }
                    if self.memory.recall(memory_id)?.is_some() {
                        // Previously ignored errors here; propagate so callers can surface I/O or corruption issues.
                        self.memory.promote_to_dag(memory_id)?;
                        if let Some(s2) = crate::memory::dag::content_by_id(memory_id)? {
                            if let Some(node) = crate::memory::dag::load_node_by_id(memory_id)? {
                                let lobe = node
                                    .get("lobe")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("restored");
                                let key = node
                                    .get("key")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("restored");
                                self.memory.remember(memory_id, lobe, key, s2.as_bytes())?;
                            }
                            return Ok(Some(RecallResult {
                                memory_id: memory_id.to_owned(),
                                content: s2,
                                source: HitSource::Dag,
                            }));
                        }
                    }
                }
                _ => unreachable!(),
            }
        }
        Ok(None)
    }

    /// Centralized batch recall (keeps order of input ids; drops misses).
    pub fn recall_many(&self, memory_ids: &[String], prefer: Prefer) -> Result<Vec<RecallResult>> {
        // Value-aware ordering: prefer higher value states first if table exists.
        let mut ids: Vec<(Option<f32>, &String)> = memory_ids
            .iter()
            .map(|id| {
                let val = self
                    .memory
                    .db
                    .prepare("SELECT value FROM \"values\" WHERE state_id=?1")
                    .ok()
                    .and_then(|mut st| st.query([id]).ok().and_then(|mut rows| rows.next().ok().flatten().and_then(|row| row.get::<_, f32>(0).ok())));
                (val, id)
            })
            .collect();
        ids.sort_by(|a, b| match (a.0, b.0) {
            (Some(x), Some(y)) => y.partial_cmp(&x).unwrap_or(std::cmp::Ordering::Equal), // desc
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        });

        let mut out = Vec::with_capacity(memory_ids.len());
        for (_, id) in ids.into_iter() {
            if let Some(hit) = self.recall_any(id, prefer)? {
                out.push(hit);
            }
        }
        Ok(out)
    }
}

fn parse_prefer(s: Option<&str>) -> Prefer {
    match s.unwrap_or("auto") {
        "hot" => Prefer::Hot,
        "archive" => Prefer::Archive,
        "dag" => Prefer::Dag,
        _ => Prefer::Auto,
    }
}
