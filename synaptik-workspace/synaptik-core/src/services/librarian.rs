// src/services/librarian.rs
//! Librarian: summarize → optional reflect → write via Memory (passed in).
//! No SQLite inside this struct; caller provides &Memory to each method.

use anyhow::Result;
use blake3;
use chrono::Utc;
use serde_json::json;
use std::num::NonZeroU32;

use summary::{Language, Summarizer};

use crate::services::archivist::Archivist;
use crate::services::audit::record_action;
use crate::services::memory::Memory;

#[derive(Debug)]
pub struct Librarian {
    archivist: Option<Archivist>, // file-only cold store
}

impl Librarian {
    pub fn new(archivist: Option<Archivist>) -> Self {
        Self { archivist }
    }

    /// Main ingest path: summarize (always, if long) → optional reflect → Memory write.
    /// Returns the `memory_id`.
    pub fn ingest_text(
        &self,
        memory: &Memory,
        lobe: &str,
        key: Option<&str>,
        content: &str,
    ) -> Result<String> {
        // IDs/defaults
        let lobe = if lobe.is_empty() { "notes" } else { lobe };
        let key = key
            .map(|k| k.to_string())
            .unwrap_or_else(|| format!("{}_memory.txt", Utc::now().format("%Y-%m-%dT%H_%M_%S")));

        let memory_id = format!(
            "{}_{}",
            lobe,
            blake3::hash([key.as_bytes(), content.as_bytes()].concat().as_slice()).to_hex()
        );

        // Summarize (short texts skip)
        let summary = if content.len() > 500 {
            let n = NonZeroU32::new(2).unwrap();
            Summarizer::new(Language::English)
                .summarize_sentences(content, n)
                .join(" ")
        } else {
            String::new()
        };

        // Lightweight reflection seed from recent summaries (optional heuristic)
        let reflection = {
            let pool = memory.recent_summaries_by_lobe(lobe, 20)?;
            let mut freq = std::collections::HashMap::<String, usize>::new();
            for s in pool {
                for t in s.split(|c: char| !c.is_alphanumeric()) {
                    let t = t.to_lowercase();
                    if t.len() >= 3 {
                        *freq.entry(t).or_default() += 1;
                    }
                }
            }
            let mut toks: Vec<(String, usize)> = freq.into_iter().collect();
            toks.sort_by(|a, b| b.1.cmp(&a.1));
            toks.truncate(3);
            if toks.is_empty() {
                None
            } else {
                Some(
                    toks.into_iter()
                        .map(|(t, c)| format!("{t}({c})"))
                        .collect::<Vec<_>>()
                        .join(", "),
                )
            }
        };

        // Single writer: write via Memory
        memory.remember_with_summary(
            &memory_id,
            lobe,
            &key,
            content.as_bytes(),
            &summary,
            reflection.as_deref(),
        )?;

        record_action(
            "librarian",
            "memory_stored",
            &json!({
                "memory_id": memory_id, "lobe": lobe, "key": key,
                "len": content.len(), "summarized": !summary.is_empty(),
                "reflected": reflection.is_some()
            }),
            "low",
        );

        Ok(memory_id)
    }

// Promote to archive: file -> CID via Archivist; then Memory writes archived_cid.
pub fn promote_to_archive(&self, memory: &Memory, memory_id: &str) -> Result<Option<String>> {
    let Some(arch) = &self.archivist else { return Ok(None) };
    if let Some(bytes) = memory.recall(memory_id)? {
        // was: let cid = arch.put(&bytes)?;
        let cid = arch.archive(memory_id, &bytes)?;
        let ts = chrono::Utc::now().to_rfc3339();
        memory.mark_archived(memory_id, &cid, &ts)?;
        crate::services::audit::record_action(
            "librarian",
            "memory_promoted",
            &serde_json::json!({ "memory_id": memory_id, "cid": cid }),
            "low",
        );
        Ok(Some(cid))
    } else {
        Ok(None)
    }
}

// Fetch with cold path: read file via Archivist; re-cache via Memory.
pub fn fetch(&self, memory: &Memory, memory_id: &str) -> Result<Option<Vec<u8>>> {
    if let Some(bytes) = memory.recall(memory_id)? {
        crate::services::audit::record_action(
            "librarian", "memory_accessed_hot",
            &serde_json::json!({ "id": memory_id }), "low");
        return Ok(Some(bytes));
    }
    if let Some(cid) = memory.get_archived_cid(memory_id)? {
        if let Some(arch) = &self.archivist {
            // was: let bytes = arch.get(&cid)?;
            let bytes = arch.retrieve(&cid)?;
            memory.remember(memory_id, "restored", "restored", &bytes)?;
            crate::services::audit::record_action(
                "librarian",
                "memory_restored_from_archive",
                &serde_json::json!({ "id": memory_id, "cid": cid }),
                "low",
            );
            return Ok(Some(bytes));
        }
    }
    Ok(None)
}

}
