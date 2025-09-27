// src/services/librarian.rs
//! Librarian: summarize → optional reflect → write via Memory (passed in).
//! No SQLite inside this struct; caller provides &Memory to each method.

use anyhow::Result;
use blake3;
use chrono::Utc;
use serde_json::json;
use std::num::NonZeroU32;

use summary::{Language, Summarizer};

use crate::config::PoliciesConfig;
use crate::services::archivist::Archivist;
use crate::services::audit::record_action;
use crate::services::memory::Memory;
use crate::commands::init::ensure_initialized_once;

// Contracts integration (SimCapsules)
use contracts::api::{Purpose};
use contracts::capsule::{SimCapsule, CapsuleMeta, CapsuleSource};
use contracts::store::ContractsStore;
use crate::services::reward::{RewardSqliteSink, RewardEvent, RewardSink};
use once_cell::sync::OnceCell;

#[derive(Debug)]
pub struct Librarian {
    archivist: Option<Archivist>,
    settings: LibrarianSettings,
    // Optional injected contracts store to reduce global coupling
    contracts: Option<ContractsStore>,
}

impl Librarian {
    pub fn new(archivist: Option<Archivist>, settings: LibrarianSettings) -> Self {
        Self {
            archivist,
            settings,
            contracts: None,
        }
    }

    /// Optional injection to avoid global coupling with the contracts store.
    pub fn with_contracts_store(mut self, store: ContractsStore) -> Self {
        self.contracts = Some(store);
        self
    }

    fn contracts_store_ref(&self) -> Option<&ContractsStore> {
        if let Some(ref s) = self.contracts { return Some(s); }
        contracts_store()
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

        // Exact dedupe guard: if identical content already exists in this lobe, return existing id.
        if let Some(existing) = memory.find_exact_duplicate_in_lobe(lobe, content.as_bytes())? {
            // Touch to reflect freshness, but avoid rewriting content.
            let _ = memory.touch(&existing);
            record_action(
                "librarian",
                "dedupe_skipped",
                &json!({"existing_id": existing, "lobe": lobe, "key": key}),
                "low",
            );
            return Ok(existing);
        }

        let memory_id = format!(
            "{}_{}",
            lobe,
            blake3::hash([key.as_bytes(), content.as_bytes()].concat().as_slice()).to_hex()
        );

        let (summary, reflection) = if !self.settings.enabled {
            (String::new(), None)
        } else {
            let summary = if self.settings.summary_min_len > 0
                && content.len() >= self.settings.summary_min_len
            {
                let n = NonZeroU32::new(2).unwrap();
                Summarizer::new(Language::English)
                    .summarize_sentences(content, n)
                    .join(" ")
            } else {
                String::new()
            };

            // Lightweight reflection seed from recent summaries (optional heuristic)
            let reflection = {
                // Hard bounds to prevent large-scale processing if backend over-returns.
                const MAX_POOL: usize = 200; // clamp pool size
                const MAX_SUMMARY_LEN: usize = 2_000; // clamp per summary chars
                const MAX_TOKENS_TOTAL: usize = 50_000; // clamp overall tokens processed

                let requested = self.settings.reflection_pool_size.min(MAX_POOL);
                let pool = memory.recent_summaries_by_lobe(lobe, requested)?;
                let mut freq = std::collections::HashMap::<String, usize>::new();
                let mut tokens_seen = 0usize;
                'outer: for s in pool {
                    let s = if s.len() > MAX_SUMMARY_LEN {
                        // Truncate on character boundary to avoid slicing in middle of UTF-8 codepoint.
                        s.chars().take(MAX_SUMMARY_LEN).collect::<String>()
                    } else { s };
                    for t in s.split(|c: char| !c.is_alphanumeric()) {
                        let t = t.to_lowercase();
                        if t.len() >= 3 {
                            *freq.entry(t).or_default() += 1;
                            tokens_seen += 1;
                            if tokens_seen >= MAX_TOKENS_TOTAL { break 'outer; }
                        }
                    }
                }
                let mut toks: Vec<(String, usize)> = freq.into_iter().collect();
                toks.sort_by(|a, b| b.1.cmp(&a.1));
                toks.truncate(self.settings.reflection_keyword_count);
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

            (summary, reflection)
        };

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

        // Assemble a minimal SimCapsule and ingest asynchronously (best-effort).
        // Non-blocking: errors are swallowed after logging.
        if let Some(store) = self.contracts_store_ref() {
            let now_ms = chrono::Utc::now().timestamp_millis() as u64;
            let cap = SimCapsule {
                inputs: serde_json::Value::Null,
                context: serde_json::json!({ "lobe": lobe, "key": key }),
                actions: serde_json::json!(["ingest_text"]),
                outputs: serde_json::json!({ "text": content }),
                trace: serde_json::json!({ "summary_len": summary.len(), "reflected": reflection.is_some() }),
                artifacts: vec![],
                meta: CapsuleMeta {
                    capsule_id: None,
                    agent_id: Some("core".to_string()),
                    lobe: Some(lobe.to_string()),
                    t_start_ms: now_ms,
                    t_end_ms: now_ms,
                    source: CapsuleSource::Real,
                    schema_ver: "1.0".to_string(),
                    capsule_hash: None,
                    issuer_signature: None,
                    parent_id: None,
                },
            };
            // Spawn a lightweight thread to avoid blocking the caller. Log errors.
            let store_clone = store.clone();
            let mem_id = memory_id.clone();
            let lobe_clone = lobe.to_string();
            if let Err(e) = std::thread::Builder::new().name("capsule_ingest".into()).spawn(move || {
                match store_clone.ingest_capsule(cap) {
                    Ok(handle) => {
                        if let Err(map_err) = store_clone.map_memory(&mem_id, &handle.id) {
                            record_action(
                                "librarian",
                                "capsule_map_error",
                                &json!({"memory_id": mem_id, "error": map_err.to_string()}),
                                "medium",
                            );
                        }

                        // Shaping reward for Real capsule ingest (small positive). Best-effort.
                        if let Ok(sink) = RewardSqliteSink::open_default() {
                            let ev = RewardEvent {
                                lobe: lobe_clone.clone(),
                                capsule_id: handle.id.clone(),
                                parent_id: None,
                                value: 0.05,
                                ts_ms: now_ms as i64,
                                labels: vec!["ingest".into()],
                                verdict: "Real".into(),
                                risk: 0.0,
                            };
                            let _ = sink.publish(&ev);
                        }

                        // Assemble a step anchored to this new state; next state inferred if any.
                        if let Ok(asm) = crate::services::learner::StepAssembler::open_default() {
                            let _ = asm.record_from_reward(&lobe_clone, &mem_id, &handle.id, 0.05, now_ms as i64);
                        }
                    }
                    Err(err) => {
                        record_action(
                            "librarian",
                            "capsule_ingest_error",
                            &json!({"memory_id": mem_id, "error": err.to_string()}),
                            "medium",
                        );
                    }
                }
            }) {
                record_action(
                    "librarian",
                    "spawn_error",
                    &json!({"error": e.to_string()}),
                    "high",
                );
            }
        }

        Ok(memory_id)
    }

    // Promote to archive: file -> CID via Archivist; then Memory writes archived_cid.
    pub fn promote_to_archive(&self, memory: &Memory, memory_id: &str) -> Result<Option<String>> {
        let Some(arch) = &self.archivist else {
            return Ok(None);
        };
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

    // Fetch with hot->cold path (kept for general callers).
    pub fn fetch(&self, memory: &Memory, memory_id: &str) -> Result<Option<Vec<u8>>> {
        if let Some(bytes) = memory.recall(memory_id)? {
            // Contracts gate: only gate when exposing to caller.
            if let Some(store) = self.contracts_store_ref() {
                if let Ok(Some(caps_id)) = store.capsule_for_memory(memory_id) {
                    if let Err(denied) = store.gate_replay(&caps_id, Purpose::Replay) {
                        record_action(
                            "librarian",
                            "gate_denied",
                            &json!({"memory_id": memory_id, "capsule_id": caps_id, "reason": denied.reason }),
                            "high",
                        );
                        return Ok(None);
                    }
                }
            }
            crate::services::audit::record_action(
                "librarian",
                "memory_accessed_hot",
                &serde_json::json!({ "id": memory_id }),
                "low",
            );
            return Ok(Some(bytes));
        }
        let cold = self.fetch_cold(memory, memory_id)?;
        if cold.is_some() {
            // Gate cold recall as well
            if let Some(store) = self.contracts_store_ref() {
                if let Ok(Some(caps_id)) = store.capsule_for_memory(memory_id) {
                    if let Err(denied) = store.gate_replay(&caps_id, Purpose::Replay) {
                        record_action(
                            "librarian",
                            "gate_denied",
                            &json!({"memory_id": memory_id, "capsule_id": caps_id, "reason": denied.reason }),
                            "high",
                        );
                        return Ok(None);
                    }
                }
            }
        }
        Ok(cold)
    }

    /// Fetch only from cold storage via Archivist if a CID exists; re-caches on success.
    pub fn fetch_cold(&self, memory: &Memory, memory_id: &str) -> Result<Option<Vec<u8>>> {
        if let Some(cid) = memory.get_archived_cid(memory_id)? {
            if let Some(arch) = &self.archivist {
                match arch.retrieve(&cid) {
                    Ok(bytes) => {
                        // Try to restore under original lobe/key from DAG metadata; fallback to stable defaults
                        let (lobe, key) = match crate::memory::dag::load_node_by_id(memory_id)? {
                            Some(node) => {
                                let l = node
                                    .get("lobe")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("restored");
                                let k = node
                                    .get("key")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("restored");
                                (l.to_string(), k.to_string())
                            }
                            None => ("restored".to_string(), "restored".to_string()),
                        };
                        memory.remember(memory_id, &lobe, &key, &bytes)?;
                        crate::services::audit::record_action(
                            "librarian",
                            "memory_restored_from_archive",
                            &serde_json::json!({ "id": memory_id, "cid": cid }),
                            "low",
                        );
                        return Ok(Some(bytes));
                    }
                    Err(_e) => {
                        // Archive miss — gracefully degrade to None so callers may try DAG.
                        crate::services::audit::record_action(
                            "librarian",
                            "archive_miss",
                            &serde_json::json!({ "id": memory_id, "cid": cid }),
                            "low",
                        );
                    }
                }
            }
        }
        Ok(None)
    }
}

// -------------------- Contracts Store helper --------------------

fn contracts_store() -> Option<&'static ContractsStore> {
    static CELL: OnceCell<Option<ContractsStore>> = OnceCell::new();
    CELL.get_or_init(|| {
        let root = ensure_initialized_once()
            .map(|r| r.config.contracts.path.join("caps_store"))
            .ok();
        match root {
            Some(dir) => ContractsStore::new(dir).ok(),
            None => None,
        }
    }).as_ref()
}

#[derive(Debug, Clone)]
pub struct LibrarianSettings {
    pub enabled: bool,
    pub summary_min_len: usize,
    pub reflection_pool_size: usize,
    pub reflection_keyword_count: usize,
}

impl LibrarianSettings {
    pub fn from_policies(policies: &PoliciesConfig, enabled: bool) -> Self {
        Self {
            enabled,
            summary_min_len: policies.summary_min_len,
            reflection_pool_size: policies.reflection_pool_size,
            reflection_keyword_count: policies.reflection_max_keywords,
        }
    }
}

impl Default for LibrarianSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            summary_min_len: 500,
            reflection_pool_size: 20,
            reflection_keyword_count: 3,
        }
    }
}
