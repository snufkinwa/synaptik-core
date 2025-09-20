// src/commands/mod.rs
use anyhow::{Result, anyhow};
use serde::Serialize;
use serde_json::{Value, json};
use std::path::PathBuf;

use crate::config::CoreConfig;
use crate::memory::dag::MemoryState as DagMemoryState;
use crate::services::archivist::Archivist;
use crate::services::audit::{lock_contracts, record_action, unlock_contracts};
use crate::services::ethos::{Decision, decision_gate, precheck};
use crate::services::{
    FinalizedStatus, LlmClient, StreamRuntime,
};
use crate::services::ethos::{ContractsDecider, Proposal};
use crate::services::audit as audit_svc;
use crate::services::librarian::{Librarian, LibrarianSettings};
use crate::services::memory::Memory;
use crate::utils::pons::{ObjectMetadata as PonsMetadata, ObjectRef as PonsObjectRef, PonsStore};
use once_cell::sync::OnceCell;
use std::sync::Arc;

use crate::commands::init::ensure_initialized_once;
use crate::commands::{HitSource, Prefer, RecallResult, bytes_to_string_owned};

pub struct Commands {
    memory: Memory,       // one SQLite connection here
    librarian: Librarian, // no DB inside
    config: CoreConfig,
    root: PathBuf,
    pons_store: OnceCell<Arc<PonsStore>>, // lazily initialized, shared store
}

#[derive(Debug, Serialize)]
pub struct EthosReport {
    pub decision: String,
    pub reason: String,
    pub risk: String,
    pub constraints: Vec<String>,
    pub action_suggestion: Option<String>,
    pub violation_code: Option<String>,
}

pub struct CommandsBuilder {
    config: CoreConfig,
    memory: Option<Memory>,
    archivist: Option<Archivist>,
    librarian: Option<Librarian>,
    root: PathBuf,
}

impl CommandsBuilder {
    pub fn from_environment() -> Result<Self> {
        let report = ensure_initialized_once()?;
        Ok(Self {
            config: report.config.clone(),
            memory: None,
            archivist: None,
            librarian: None,
            root: report.root.clone(),
        })
    }

    pub fn with_config(mut self, config: CoreConfig) -> Self {
        self.config = config;
        self
    }

    pub fn with_memory(mut self, memory: Memory) -> Self {
        self.memory = Some(memory);
        self
    }

    pub fn with_archivist(mut self, archivist: Archivist) -> Self {
        self.archivist = Some(archivist);
        self
    }

    pub fn with_librarian(mut self, librarian: Librarian) -> Self {
        self.librarian = Some(librarian);
        self
    }

    pub fn build(mut self) -> Result<Commands> {
        let memory = if let Some(memory) = self.memory.take() {
            memory
        } else {
            let db_path = self
                .config
                .memory
                .cache_path
                .to_str()
                .ok_or_else(|| anyhow!("invalid UTF-8 db path"))?;
            Memory::open(db_path)?
        };

        let archivist = if let Some(archivist) = self.archivist.take() {
            Some(archivist)
        } else if self.config.services.librarian_enabled {
            Some(Archivist::open(&self.config.memory.archive_path)?)
        } else {
            None
        };

        let librarian = if let Some(librarian) = self.librarian.take() {
            librarian
        } else {
            let settings = LibrarianSettings::from_policies(
                &self.config.policies,
                self.config.services.librarian_enabled,
            );
            Librarian::new(archivist.clone(), settings)
        };

        Ok(Commands {
            memory,
            librarian,
            config: self.config,
            root: self.root.clone(),
            pons_store: OnceCell::new(),
        })
    }
}

impl Commands {
    fn pons_store(&self) -> Result<Arc<PonsStore>> {
        let store_ref = self
            .pons_store
            .get_or_try_init(|| PonsStore::open(self.root.clone()).map(Arc::new))?;
        Ok(Arc::clone(store_ref))
    }

    /// Run content through the contract-enforced runtime. Returns sanitized text on success,
    /// or Ok(None) if the runtime stopped/escalated/violated (barrier applied).
    fn govern_text(&self, intent: &str, input: &str) -> Result<Option<String>> {
        struct EchoStream { yielded: bool, text: String }
        impl Iterator for EchoStream {
            type Item = String;
            fn next(&mut self) -> Option<Self::Item> {
                if self.yielded { None } else { self.yielded = true; Some(self.text.clone()) }
            }
        }
        struct TextEchoModel { text: String }
        impl LlmClient for TextEchoModel {
            type Stream = EchoStream;
            fn stream(&self, _system_prompt: String) -> std::result::Result<Self::Stream, crate::services::GateError> {
                Ok(EchoStream { yielded: false, text: self.text.clone() })
            }
        }

        let proposal = Proposal {
            intent: intent.to_string(),
            input: input.to_string(),
            prior: None,
            tools_requested: vec![],
        };
        let contract = ContractsDecider;
        let model = TextEchoModel { text: input.to_string() };
        let runtime = StreamRuntime { contract, model };
        let result = runtime.generate(proposal).map_err(|e| anyhow!(e.0))?;

        match result.status {
            FinalizedStatus::Ok => Ok(Some(result.text)),
            FinalizedStatus::Stopped => {
                audit_svc::record_action(
                    "commands",
                    "govern_stopped",
                    &json!({"intent": intent}),
                    "medium",
                );
                Ok(None)
            }
            FinalizedStatus::Escalated | FinalizedStatus::Violated => {
                audit_svc::record_action(
                    "commands",
                    "govern_blocked",
                    &json!({"intent": intent, "status": format!("{:?}", result.status)}),
                    "high",
                );
                Ok(None)
            }
        }
    }

    // -------------------- Path/name helpers --------------------

    fn normalize_path_name(&self, name: &str) -> String {
        let mut s = name.to_lowercase();
        s.retain(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        if s.is_empty() { "path".to_string() } else { s }
    }

    /// Get newest snapshot hash on a named path.
    pub fn dag_head(&self, path_name: &str) -> Result<Option<String>> {
        crate::memory::dag::path_head_hash(path_name)
    }

    /// Update a named path head to a specific snapshot hash.
    pub fn update_path_head(&self, path_name: &str, snapshot_hash: &str) -> Result<()> {
        let r = crate::memory::dag::set_path_head(path_name, snapshot_hash);
        if r.is_ok() {
            record_action(
                "commands",
                "update_path_head",
                &json!({ "path": path_name, "hash": snapshot_hash }),
                "low",
            );
        }
        r
    }

    // Keep the signature for now; ignore the args. Prefix with _ to silence warnings.
    pub fn new(_db_path: &str, _archivist: Option<Archivist>) -> Result<Self> {
        Self::builder()?.build()
    }

    pub fn builder() -> Result<CommandsBuilder> {
        CommandsBuilder::from_environment()
    }

    pub fn config(&self) -> &CoreConfig {
        &self.config
    }

    /// Ensure a pons namespace exists under the shared root.
    pub fn pons_create(&self, pons: &str) -> Result<()> {
        let store = self.pons_store()?;
        store.create_pons(pons)
    }

    /// Write bytes plus optional metadata into a pons/key stream.
    pub fn pons_put_object(
        &self,
        pons: &str,
        key: &str,
        data: &[u8],
        media_type: Option<&str>,
        extra: Option<Value>,
    ) -> Result<PonsObjectRef> {
        let store = self.pons_store()?;
        // Filesystem paths are an internal detail of the Pons store.
        // We expose only the content-addressed ObjectRef; callers shouldn't rely on on-disk paths.
        let (obj, path) = store.put_object_with_meta(pons, key, data, media_type, extra)?;
        let _ = path; // explicitly discard internal path to make intent clear
        Ok(obj)
    }

    /// Read newest bytes for a pons/key.
    pub fn pons_get_latest_bytes(&self, pons: &str, key: &str) -> Result<Vec<u8>> {
        let store = self.pons_store()?;
        store.get_object_latest(pons, key)
    }

    /// Fetch newest ObjectRef for a pons/key.
    pub fn pons_get_latest_ref(&self, pons: &str, key: &str) -> Result<PonsObjectRef> {
        let store = self.pons_store()?;
        store.get_object_latest_ref(pons, key)
    }

    /// Fetch a specific version's bytes and metadata.
    pub fn pons_get_version_with_meta(
        &self,
        pons: &str,
        key: &str,
        version: &str,
    ) -> Result<(Vec<u8>, PonsMetadata)> {
        let store = self.pons_store()?;
        store.get_object_version_with_meta(pons, key, version)
    }

    /// List the latest refs under a pons namespace.
    pub fn pons_list_latest(
        &self,
        pons: &str,
        prefix: Option<&str>,
        limit: usize,
    ) -> Result<Vec<PonsObjectRef>> {
        let store = self.pons_store()?;
        store.list_latest(pons, prefix, limit)
    }

    /// Gate arbitrary text with Ethos (for normal chat).
    pub fn precheck_text(&self, text: &str, purpose: &str) -> Result<EthosReport> {
        if !self.config.services.ethos_enabled {
            return Ok(EthosReport {
                decision: "allow".to_string(),
                reason: "ethos_disabled".to_string(),
                risk: "Low".to_string(),
                constraints: Vec::new(),
                action_suggestion: None,
                violation_code: None,
            });
        }

        let v = precheck(text, purpose).map_err(|e| anyhow!("ethos precheck error: {e}"))?;
        let decision = match decision_gate(&v) {
            Decision::Allow => "allow",
            Decision::AllowWithConstraints => "allow_with_constraints",
            Decision::Block => "block",
        }
        .to_string();

        let action_suggestion = None;
        let violation_code = None;

        record_action(
            "commands",
            "precheck_called",
            &json!({"purpose": purpose, "decision": decision, "violation_code": violation_code}),
            "low",
        );

        Ok(EthosReport {
            decision,
            reason: v.reason.clone(),
            risk: v.risk.clone(),
            constraints: v.constraints.clone(),
            action_suggestion,
            violation_code,
        })
    }

    /// Newest → oldest memory_ids for a lobe.
    pub fn recent(&self, lobe: &str, n: usize) -> Result<Vec<String>> {
        recent_ids_in_lobe(&self.memory, lobe, n)
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

    pub fn remember(&self, lobe: &str, key: Option<&str>, content: &str) -> Result<String> {
        record_action(
            "commands",
            "remember_called",
            &json!({"lobe": lobe, "key_is_some": key.is_some()}),
            "low",
        );

        // Governance: contract decision + runtime interceptor + write barrier
        let governed_text = if self.config.services.ethos_enabled {
            match self.govern_text("memory_storage", content) {
                Ok(Some(sanitized)) => sanitized,
                Ok(None) => return Err(anyhow!("blocked by runtime")),
                Err(e) => return Err(anyhow!("runtime error: {}", e)),
            }
        } else {
            content.to_string()
        };

        // Normalize to match Librarian’s behavior when lobe is empty.
        let lobe_eff = if lobe.is_empty() { "notes" } else { lobe };

        // 1) write hot via Librarian (after governance)
        let id = self
            .librarian
            .ingest_text(&self.memory, lobe_eff, key, &governed_text)?;
        record_action(
            "commands",
            "remember_stored",
            &json!({"id": id, "lobe": lobe_eff}),
            "low",
        );

        // 2) AUTO-PROMOTE RULE (count-based)
        //    Hot = total - archived (we reuse existing tiny helpers here).
        let total = count_rows(&self.memory, Some(lobe_eff))?;
        let archived = count_archived(&self.memory, Some(lobe_eff))?;
        let hot = total.saturating_sub(archived);

        // 2a) AUTO-PRUNE (exact duplicates) after every write to keep hot store clean.
        if self.config.policies.auto_prune_duplicates {
            if let Ok(deleted) = self.memory.prune_exact_duplicates_in_lobe(lobe_eff) {
                record_action(
                    "commands",
                    "auto_prune_duplicates",
                    &json!({"lobe": lobe_eff, "deleted": deleted, "hot": hot}),
                    if deleted > 0 { "medium" } else { "low" },
                );
            }
        }

        let promote_threshold = self.config.policies.promote_hot_threshold as u64;
        if promote_threshold > 0 && hot >= promote_threshold {
            if let Ok(promoted) = self.memory.promote_all_hot_in_lobe(lobe_eff) {
                record_action(
                    "commands",
                    "auto_promote_to_dag",
                    &json!({
                        "lobe": lobe_eff,
                        "hot_before": hot,
                        "promoted_count": promoted.len()
                    }),
                    "low",
                );

                // Ensure cold archive objects are written alongside DAG promotion.
                // This keeps README’s promise: files under .cogniv/archive/<cid>.
                for (id, _cid) in &promoted {
                    let _ = self.librarian.promote_to_archive(&self.memory, id);
                }
            }
        }

        Ok(id)
    }

    pub fn reflect(&self, lobe: &str, window: usize) -> Result<String> {
        record_action(
            "commands",
            "reflect_called",
            &json!({"lobe": lobe, "window": window}),
            "low",
        );

        let pool = self.memory.recent_summaries_by_lobe(lobe, window)?;
        let note = compute_reflection(
            &pool,
            self.config.policies.reflection_min_count,
            self.config.policies.reflection_max_keywords,
        );
        if note.is_empty() {
            record_action(
                "commands",
                "reflect_noop",
                &json!({"reason": "no_summaries"}),
                "low",
            );
            return Ok(String::new());
        }

        if self.config.services.ethos_enabled {
            let v = precheck(&note, "reflection_update")
                .map_err(|e| anyhow!("ethos precheck error: {e}"))?;
            if matches!(decision_gate(&v), Decision::Block) {
                record_action(
                    "commands",
                    "reflect_blocked",
                    &json!({"reason": v.reason, "risk": v.risk}),
                    "high",
                );
                return Ok(String::new());
            }
        }

        // Runtime governance for reflection text before writing to memory
        let governed_note = if self.config.services.ethos_enabled {
            match self.govern_text("reflection_update", &note)? {
                Some(s) => s,
                None => {
                    return Ok(String::new());
                }
            }
        } else {
            note.clone()
        };

        if let Some(id) = latest_id_in_lobe(&self.memory, lobe)? {
            self.memory.set_reflection(&id, &governed_note)?;
            record_action("commands", "reflect_set", &json!({"id": id}), "low");
        } else {
            record_action(
                "commands",
                "reflect_noop",
                &json!({"reason": "no_rows_in_lobe"}),
                "low",
            );
        }
        Ok(governed_note)
    }

    pub fn stats(&self, lobe: Option<&str>) -> Result<Stats> {
        record_action("commands", "stats_called", &json!({"lobe": lobe}), "low");
        let _ = precheck("stats_request", "metadata_access");

        let total = count_rows(&self.memory, lobe)?;
        let archived = count_archived(&self.memory, lobe)?;
        let by_lobe = group_by_lobe(&self.memory, 20)?;
        let last_updated = max_updated(&self.memory)?;
        record_action(
            "commands",
            "stats_returned",
            &json!({"total": total, "archived": archived}),
            "low",
        );

        Ok(Stats {
            total,
            archived,
            by_lobe,
            last_updated,
        })
    }

    // ---------------------------------------------------------------------
    // Replay (Rewind & Diverge) helpers exposed via Commands
    // ---------------------------------------------------------------------

    /// Recall an immutable snapshot by content-addressed id (blake3 hex).
    pub fn replay_recall_snapshot(&self, snapshot_id: &str) -> Result<DagMemoryState> {
        // Read-only; no audit log to reduce noise.
        self.memory.recall_snapshot(snapshot_id)
    }

    /// Create or reset a named path diverging from the given snapshot. Returns path_id.
    pub fn replay_diverge_from(&self, snapshot_id: &str, path_name: &str) -> Result<String> {
        let id = self.memory.diverge_from(snapshot_id, path_name)?;
        record_action(
            "commands",
            "replay_diverge_from",
            &json!({
                "snapshot_id": snapshot_id,
                "path_name": path_name,
                "path_id": id
            }),
            "low",
        );
        Ok(id)
    }

    /// Append a new immutable snapshot to a named path and advance its head. Returns new hash.
    pub fn replay_extend_path(&self, path_name: &str, state: DagMemoryState) -> Result<String> {
        let new_id = self.memory.extend_path(path_name, state)?;
        record_action(
            "commands",
            "replay_extend_path",
            &json!({
                "path_name": path_name,
                "new_hash": new_id
            }),
            "low",
        );
        Ok(new_id)
    }

    // ---------------------------------------------------------------------
    // High-level branch/append/consolidate APIs (idempotent, ethos-gated)
    // ---------------------------------------------------------------------

    /// Fork from a base path into a new exploratory branch.
    /// Neuroscience: sprout a dendritic branch from the soma (`base_path`).
    /// Returns the base snapshot CID used for the fork.
    pub fn sprout_dendrite(&self, base_path: &str, new_path: &str) -> Result<String> {
        // Resolve base: prefer head of base_path, otherwise seed from default lobe.
        let base = self
            .dag_head(base_path)?
            .or_else(|| self.replay_base_from_lobe("chat").ok().flatten())
            .ok_or(anyhow!("no base available for sprout_dendrite"))?;

        let base_norm = self.normalize_path_name(base_path);
        let new_norm = self.normalize_path_name(new_path);

        // Ensure base path exists; idempotent.
        if !crate::memory::dag::path_exists(&base_norm)? {
            let _ = self.replay_diverge_from(&base, &base_norm)?;
        }
        // Create or reset new path at same base; idempotent.
        let _ = self.replay_diverge_from(&base, &new_norm)?;
        Ok(base)
    }

    /// Fast-forward the target path to the source head.
    /// Neuroscience: systems consolidation (stabilize the trace into 'cortex'/`dst_path`).
    pub fn systems_consolidate(&self, src_path: &str, dst_path: &str) -> Result<String> {
        let src_head = self.dag_head(src_path)?.ok_or(anyhow!("no src head"))?;
        // If dst missing or behind: repoint head to src (FF). If already equal: noop.
        if let Some(dst_head) = self.dag_head(dst_path)? {
            if dst_head == src_head {
                return Ok(src_head);
            }
            // Only fast-forward when ancestor; otherwise caller should request merge.
            if crate::memory::dag::is_ancestor(&dst_head, &src_head)? {
                self.update_path_head(dst_path, &src_head)?;
            } else {
                return Err(anyhow!("non-fast-forward: dst is not ancestor of src"));
            }
        } else {
            // Create path at src head
            self.update_path_head(dst_path, &src_head)?;
        }
        Ok(src_head)
    }

    /// Create a merge snapshot with parents [main_head, feature_head] and move main to it.
    /// Neuroscience: reconsolidation—integrate multiple traces into one memory.
    /// Note: DAG presently supports single-parent. Until merge nodes are supported, this returns an error
    /// when a fast-forward is not possible.
    pub fn reconsolidate_paths(
        &self,
        main_path: &str,
        feature_path: &str,
        _note: &str,
    ) -> Result<String> {
        let main_head = self.dag_head(main_path)?.ok_or(anyhow!("no main head"))?;
        let feat_head = self
            .dag_head(feature_path)?
            .ok_or(anyhow!("no feature head"))?;
        if main_head == feat_head {
            return Ok(main_head);
        }
        if crate::memory::dag::is_ancestor(&main_head, &feat_head)? {
            self.update_path_head(main_path, &feat_head)?;
            return Ok(feat_head);
        }
        Err(anyhow!(
            "merge commits not yet supported; non-FF reconsolidation blocked"
        ))
    }

    /// Idempotent, normalized: create a branch at a resolved base.
    /// base may be a snapshot hash or a path name; if None, lobe or 'main' are used.
    pub fn branch(&self, path: &str, base: Option<&str>, lobe: Option<&str>) -> Result<String> {
        let path_norm = self.normalize_path_name(path);
        // If path exists already, return its recorded base snapshot id.
        if crate::memory::dag::path_exists(&path_norm)? {
            if let Some(b) = crate::memory::dag::path_base_snapshot(&path_norm)? {
                return Ok(b);
            }
        }

        // Resolve base: explicit hash or path, or by lobe/main fallback.
        let resolved_base = if let Some(b) = base {
            let b_norm = self.normalize_path_name(b);
            // treat as path name if a path exists; else assume it's a cid
            if crate::memory::dag::path_exists(&b_norm)? {
                self.dag_head(&b_norm)?
            } else {
                Some(b.to_string())
            }
        } else if let Some(l) = lobe {
            self.replay_base_from_lobe(l)?
        } else if let Some(h) = self.dag_head("main")? {
            Some(h)
        } else {
            self.replay_base_from_lobe("chat")?
        }
        .ok_or(anyhow!("no base available to branch from"))?;

        let _ = self.replay_diverge_from(&resolved_base, &path_norm)?;
        record_action(
            "commands",
            "branch_created",
            &json!({ "path": path_norm, "base": resolved_base }),
            "low",
        );
        Ok(resolved_base)
    }

    /// Append content to a named path with provenance and ethos gating.
    pub fn append(&self, path: &str, content: &str, meta: Option<Value>) -> Result<String> {
        let path_norm = self.normalize_path_name(path);
        if !crate::memory::dag::path_exists(&path_norm)? {
            return Err(anyhow!(format!(
                "path '{}' not found; call branch() first",
                path_norm
            )));
        }

        // Governance: runtime enforcement for append content
        let governed_text = if self.config.services.ethos_enabled {
            match self.govern_text("replay_append", content) {
                Ok(Some(s)) => s,
                Ok(None) => return Err(anyhow!("blocked by runtime")),
                Err(e) => return Err(anyhow!("runtime error: {}", e)),
            }
        } else {
            content.to_string()
        };

        let parent = self.dag_head(&path_norm)?;
        let base = crate::memory::dag::path_base_snapshot(&path_norm)?;
        let enrich = json!({
            "op": "append",
            "ts": chrono::Utc::now().to_rfc3339(),
            "actor": "core",
            "path": path_norm,
            "parents": parent.clone().into_iter().collect::<Vec<_>>() ,
            "base": base,
            "content_hash": blake3::hash(governed_text.as_bytes()).to_hex().to_string(),
        });
        let merged_meta = match meta.unwrap_or_else(|| json!({})) {
            Value::Object(mut m) => {
                if let Value::Object(e) = enrich {
                    m.extend(e);
                }
                Value::Object(m)
            }
            _ => enrich,
        };
        let state = DagMemoryState {
            content: governed_text,
            meta: merged_meta,
        };
        let id = self.replay_extend_path(&path_norm, state)?;
        Ok(id)
    }

    /// Fast-forward if possible; else no-op with error until merges are supported.
    pub fn consolidate(&self, src_path: &str, dst_path: &str) -> Result<String> {
        self.systems_consolidate(src_path, dst_path)
    }

    /// Placeholder for future two-parent merge support. Errors today if non-FF.
    pub fn merge(&self, src_path: &str, dst_path: &str, note: &str) -> Result<String> {
        let _ = note; // reserved for future merge-commit message
        self.reconsolidate_paths(dst_path, src_path, note)
    }

    // (Removed deprecated backward-compatible aliases)

    // ---------------------------------------------------------------------
    // DAG metadata and citations
    // ---------------------------------------------------------------------

    /// Fetch the meta object for a snapshot (including provenance if present).
    pub fn dag_snapshot_meta(&self, snapshot_id: &str) -> Result<serde_json::Value> {
        // Read-only; no audit log.
        crate::memory::dag::snapshot_meta(snapshot_id)
    }

    /// Trace a named path newest -> oldest up to limit.
    pub fn dag_trace_path(&self, path_name: &str, limit: usize) -> Result<Vec<serde_json::Value>> {
        // Read-only; no audit log.
        crate::memory::dag::trace_path(path_name, limit)
    }

    /// Extract and de-dup provenance sources from a snapshot.
    pub fn dag_cite_sources(&self, snapshot_id: &str) -> Result<Vec<serde_json::Value>> {
        // Read-only; no audit log.
        crate::memory::dag::cite_sources(snapshot_id)
    }

    /// Search DAG nodes by content words (case-insensitive), newest-first.
    /// Returns a list of dicts: [{"hash", "id", "ts"}]
    pub fn dag_search_content(&self, query: &str, limit: usize) -> Result<Vec<serde_json::Value>> {
        let words: Vec<String> = query
            .split_whitespace()
            .filter(|w| !w.is_empty())
            .map(|s| s.to_string())
            .collect();
        crate::memory::dag::search_content_words(&words, limit)
    }

    /// Prune exact duplicates. If `lobe` is Some, prunes within that lobe; otherwise all lobes.
    pub fn prune_duplicates(&self, lobe: Option<&str>) -> Result<usize> {
        let total = if let Some(l) = lobe {
            self.memory.prune_exact_duplicates_in_lobe(l)?
        } else {
            // Collect lobes and prune per-lobe
            let lobes: Vec<String> = {
                let mut stmt = self
                    .memory
                    .db
                    .prepare("SELECT DISTINCT lobe FROM memories")?;
                let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
                let mut out = Vec::new();
                for r in rows {
                    out.push(r?);
                }
                out
            };
            let mut acc = 0usize;
            for l in lobes {
                acc += self.memory.prune_exact_duplicates_in_lobe(&l)?;
            }
            acc
        };
        record_action(
            "commands",
            "prune_duplicates",
            &json!({"lobe": lobe, "deleted": total }),
            if total > 0 { "medium" } else { "low" },
        );
        Ok(total)
    }

    /// Resolve a replay base snapshot for a lobe by preferring the latest archived CID,
    /// otherwise promoting the latest hot item in that lobe. Returns Some(cid) if available.
    pub fn replay_base_from_lobe(&self, lobe: &str) -> Result<Option<String>> {
        if let Some(cid) = self.memory.latest_archived_cid_in_lobe_public(lobe)? {
            return Ok(Some(cid));
        }
        if let Some((_id, cid)) = self.memory.promote_latest_hot_in_lobe(lobe)? {
            return Ok(Some(cid));
        }
        // No archived or hot rows exist for this lobe. Auto-seed a minimal base so
        // replay flows can begin without requiring Python-side seeding.
        //
        // 1) Write a tiny seed row via Librarian (hot store)
        let seed_key = Some("seed_base");
        let seed_text = format!("Seed: auto-seeded base for lobe '{lobe}'");
        let mem_id = self
            .librarian
            .ingest_text(&self.memory, lobe, seed_key, &seed_text)?;
        record_action(
            "commands",
            "replay_seed_written",
            &json!({ "lobe": lobe, "memory_id": mem_id }),
            "low",
        );

        // 2) Promote that hot row into the DAG to obtain a CID
        if let Some((id, cid)) = self.memory.promote_latest_hot_in_lobe(lobe)? {
            // Also ensure cold archive object is created for parity
            let _ = self.librarian.promote_to_archive(&self.memory, &id);
            record_action(
                "commands",
                "replay_seed_promoted",
                &json!({ "lobe": lobe, "memory_id": id, "cid": cid }),
                "low",
            );
            return Ok(Some(cid));
        }

        Ok(None)
    }

    /// Force contract files to match embedded canon.
    pub fn lock_contracts(&self) {
        lock_contracts();
        record_action("commands", "contracts_locked", &json!({}), "low");
    }

    /// Allow local edits to contract files.
    pub fn unlock_contracts(&self) {
        unlock_contracts();
        record_action("commands", "contracts_unlocked", &json!({}), "low");
    }

    /// Return the DAG node JSON for a memory id (if indexed), for frontend inspection.
    pub fn dag_node(&self, memory_id: &str) -> Result<Option<serde_json::Value>> {
        Ok(crate::memory::dag::load_node_by_id(memory_id)?)
    }

    /// Promote most recent hot item in lobe to DAG/archive. Returns (memory_id, cid) if promoted.
    pub fn promote_latest_hot(&self, lobe: &str) -> Result<Option<(String, String)>> {
        // First, promote to DAG (marks archived_cid) using Memory helper
        let res = self.memory.promote_latest_hot_in_lobe(lobe)?;
        // Also ensure Archivist stores the bytes on filesystem for cold recall parity
        if let Some((id, _cid)) = res.as_ref() {
            let _ = self.librarian.promote_to_archive(&self.memory, id);
        }
        Ok(res)
    }

    /// Rebuild the DAG id-index for a given memory id by linking it to the latest node in its (lobe,key) stream.
    /// Returns true if an index was written.
    pub fn reindex_dag_id(&self, memory_id: &str) -> Result<bool> {
        if let Some((lobe, key)) = self.memory.lobe_key(memory_id)? {
            return Ok(crate::memory::dag::reindex_id_to_latest(
                memory_id, &lobe, &key,
            )?);
        }
        Ok(false)
    }

    /// Ensure archive is present and DB pointer (archived_cid) is set for a memory id.
    /// Returns Some(cid) if ensured, None if the memory could not be found.
    pub fn ensure_archive_for(&self, memory_id: &str) -> Result<Option<String>> {
        // If CID already set, ensure the blob exists; if missing, reconstruct from hot or DAG.
        if let Some(existing_cid) = self.memory.get_archived_cid(memory_id)? {
            let arch = Archivist::open(&self.config.memory.archive_path)?;
            match arch.retrieve(&existing_cid) {
                Ok(bytes) => {
                    // Re-cache hot under original lobe/key if possible
                    if let Some((lobe, key)) = self.memory.lobe_key(memory_id)? {
                        let _ = self.memory.remember(memory_id, &lobe, &key, &bytes);
                    } else if let Some(node) = crate::memory::dag::load_node_by_id(memory_id)? {
                        let lobe = node
                            .get("lobe")
                            .and_then(|v| v.as_str())
                            .unwrap_or("restored");
                        let key = node
                            .get("key")
                            .and_then(|v| v.as_str())
                            .unwrap_or("restored");
                        let _ = self.memory.remember(memory_id, lobe, key, &bytes);
                    }
                    return Ok(Some(existing_cid));
                }
                Err(_) => {
                    // Archive object missing — attempt to reconstruct from hot or DAG
                }
            }
        }

        // Load bytes from hot or DAG
        let bytes_opt = match self.memory.recall(memory_id)? {
            Some(b) => Some(b),
            None => crate::memory::dag::content_by_id(memory_id)?.map(|s| s.into_bytes()),
        };
        if let Some(bytes) = bytes_opt {
            // Write archive blob and set DB pointer (open archivist at canonical path)
            let arch = Archivist::open(&self.config.memory.archive_path)?;
            let cid = arch.archive(memory_id, &bytes)?;
            let now = chrono::Utc::now().to_rfc3339();
            self.memory.mark_archived(memory_id, &cid, &now)?;
            return Ok(Some(cid));
        }
        Ok(None)
    }

    // -------- Centralized recall --------

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
                        let _ = self.memory.promote_to_dag(memory_id);
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
        let mut out = Vec::with_capacity(memory_ids.len());
        for id in memory_ids {
            if let Some(hit) = self.recall_any(id, prefer)? {
                out.push(hit);
            }
        }
        Ok(out)
    }
}

// Prefer string shim (backwards compat)
fn parse_prefer(s: Option<&str>) -> Prefer {
    match s.unwrap_or("auto") {
        "hot" => Prefer::Hot,
        "archive" => Prefer::Archive,
        "dag" => Prefer::Dag,
        _ => Prefer::Auto,
    }
}

/// Tiny, deterministic keyword theme line (command-level helper).
fn compute_reflection(summaries: &[String], min_count: usize, max_keywords: usize) -> String {
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

#[derive(Debug, Serialize)]
pub struct Stats {
    pub total: u64,
    pub archived: u64,
    pub by_lobe: Vec<(String, u64)>,
    pub last_updated: Option<String>,
}

// ---------- tiny SQL helpers (read-only) ----------

fn latest_id_in_lobe(memory: &Memory, lobe: &str) -> Result<Option<String>> {
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

fn recent_ids_in_lobe(memory: &Memory, lobe: &str, limit: usize) -> Result<Vec<String>> {
    let mut stmt = memory.db.prepare(
        "SELECT memory_id
         FROM memories
         WHERE lobe = ?1
         ORDER BY updated_at DESC
         LIMIT ?2",
    )?;
    let rows = stmt.query_map((lobe, limit as i64), |r| r.get::<_, String>(0))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
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
        "SELECT lobe, COUNT(*) as c FROM memories GROUP BY lobe ORDER BY c DESC LIMIT ?1",
    )?;
    let rows = stmt.query_map([limit as i64], |r| {
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

fn max_updated(memory: &Memory) -> Result<Option<String>> {
    let mut stmt = memory.db.prepare("SELECT MAX(updated_at) FROM memories")?;
    let mut rows = stmt.query([])?;
    if let Some(r) = rows.next()? {
        let ts: Option<String> = r.get(0)?;
        return Ok(ts);
    }
    Ok(None)
}
