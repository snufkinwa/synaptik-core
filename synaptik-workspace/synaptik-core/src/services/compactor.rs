// src/services/compactor.rs
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

use crate::config::{CompactionPolicy, SummarizerKind};
use crate::services::memory::{Memory, MemoryCandidate};
use crate::commands::init::ensure_initialized_once;
use contracts::{EvaluationResult, MoralContract, Verdict};
use contracts::evaluator::load_or_default;
use contracts::{evaluate_input_against_rules};
use crate::utils::pons::PonsStore;
use crate::services::reward::RewardSink;
use contracts::capsule::{SimCapsule, CapsuleMeta, CapsuleSource};
use contracts::api::CapsAnnot;
use contracts::store::ContractsStore;
use once_cell::sync::OnceCell;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CompactionReport {
    pub lobe: String,
    pub dry_run: bool,
    pub candidates: usize,
    pub archived: usize,
    pub compressed: usize,   // summarized & replaced when not dry_run
    pub regrets: usize,      // failed ethos / rejected summaries
    pub notes: Vec<String>,
}

impl Default for CompactionReport {
    fn default() -> Self {
        Self {
            lobe: String::new(),
            dry_run: true,
            candidates: 0,
            archived: 0,
            compressed: 0,
            regrets: 0,
            notes: vec![],
        }
    }
}

pub struct Compactor<'a> {
    pub memory: &'a Memory,
    pub pons: Option<&'a PonsStore>,
}

impl<'a> Compactor<'a> {
    pub fn compact_lobe(
        &self,
        lobe: &str,
        policy: &CompactionPolicy,
        dry_run: bool,
    ) -> Result<CompactionReport> {
        let mut report = CompactionReport {
            lobe: lobe.to_string(),
            dry_run,
            ..Default::default()
        };

        // 1) candidate selection (stable ordering)
        let top_k: u32 = policy.select_top_k.unwrap_or(50);
        let prefer_rarely = policy.prefer_rarely_accessed;
        let candidates = self
            .memory
            .select_compaction_candidates(lobe, top_k, prefer_rarely)?;
        report.candidates = candidates.len();
        report.notes.push(format!("selected {} candidates (top_k={})", candidates.len(), top_k));

        if candidates.is_empty() {
            report.notes.push("no candidates -> done".into());
            return Ok(report);
        }

        // 2) archive originals to DAG (idempotent)
        if !dry_run && policy.archive_to_dag {
            for c in candidates.iter() {
                if c.archived_cid.is_none() {
                    match self.memory.promote_to_dag(&c.id) {
                        Ok(_) => report.archived += 1,
                        Err(e) => report.notes.push(format!("archive failed {}: {}", c.id, e)),
                    }
                }
            }
        } else if dry_run && policy.archive_to_dag {
            report.notes.push("dry_run: would archive un-archived candidates".into());
        }

        // 3) summarization / replacement pipeline
        if dry_run {
            report.compressed = report.candidates;
            report.notes.push("dry_run: counted candidates as 'compressed' (no mutation)".into());
            return Ok(report);
        }

        if let Err(e) = self.summarize_and_replace(lobe, &candidates, policy, &mut report) {
            report.notes.push(format!("summarization pass encountered error: {}", e));
        }

        Ok(report)
    }

    // ---- Internal helpers --------------------------------------------------

    fn summarize_and_replace(
        &self,
        lobe: &str,
        candidates: &[MemoryCandidate],
        policy: &CompactionPolicy,
        report: &mut CompactionReport,
    ) -> Result<()> {
        if candidates.is_empty() {
            return Ok(());
        }

        let summarizer = self.choose_summarizer(policy);

        for c in candidates {
            // Fetch original content.
            let original = match self.fetch_original(&c.id) {
                Ok(s) if !s.trim().is_empty() => s,
                Ok(_) => {
                    report.notes.push(format!("skip empty {}", c.id));
                    continue;
                }
                Err(e) => {
                    report.notes.push(format!("fetch failed {}: {}", c.id, e));
                    continue;
                }
            };

            // Sidecar store of original for external audit (best-effort).
            if let Some(pons) = self.pons {
                if let Err(e) = pons.store_original(lobe, &c.id, &original) {
                    report.notes.push(format!("pons store_original failed {}: {}", c.id, e));
                }
            }

            // Generate summary.
            let summary = match self.invoke_summarizer(&original, summarizer.clone()) {
                Ok(s) => s,
                Err(e) => {
                    report.notes.push(format!("summarizer failed {}: {}", c.id, e));
                    report.regrets += 1;
                    continue;
                }
            };

            // Contracts evaluation for the derived summary; support AllowWithPatch masks.
            let verdict = self.eval_summary_with_contracts(&summary)?;
            let mut final_summary = summary.clone();
            let (verdict_variant, risk_score, _reason_opt, patched_applied) = {
                match verdict {
                    (Verdict::Allow, _pat, risk, reason) => (Verdict::Allow, risk, reason, false),
                    (Verdict::AllowWithPatch, Some(patterns), risk, reason) => {
                        final_summary = crate::services::masking::apply_masks_ci(&final_summary, &patterns);
                        report.notes.push(format!(
                            "patched summary {} with {} mask(s)",
                            c.id,
                            patterns.len()
                        ));
                        (Verdict::AllowWithPatch, risk, reason, true)
                    }
                    (Verdict::AllowWithPatch, None, risk, reason) => {
                        (Verdict::AllowWithPatch, risk, reason, false)
                    }
                    (Verdict::Quarantine, _p, risk, reason) => {
                        // Preserve legacy wording expected by tests.
                        report.notes.push(match reason.clone() {
                            Some(r) => format!("ethos rejected summary {} (risk={:.2}): {}", c.id, risk, r),
                            None => format!("ethos rejected summary {} (risk={:.2})", c.id, risk),
                        });
                        report.regrets += 1;
                        continue; // Skip replace & ingestion
                    }
                }
            };

            // Ingest AFTER masking so stored capsule matches persisted memory row.
            if let Some(store) = contracts_store() {
                let parent_id = store.capsule_for_memory(&c.id).ok().flatten();
                let now_ms = chrono::Utc::now().timestamp_millis() as u64;
                let labels = match (verdict_variant.clone(), patched_applied) {
                    (Verdict::Allow, _) => vec!["summary".into()],
                    (Verdict::AllowWithPatch, true) => vec!["summary".into(), "patched".into()],
                    (Verdict::AllowWithPatch, false) => vec!["summary".into()],
                    (Verdict::Quarantine, _) => vec!["summary".into()], // Should not occur here due to continue.
                };
                let cap = SimCapsule {
                    inputs: serde_json::json!({}),
                    context: serde_json::json!({ "lobe": lobe, "memory_id": c.id, "parent_capsule_id": parent_id }),
                    actions: serde_json::json!(["compaction_summarize"]),
                    outputs: serde_json::json!({ "summary": final_summary }),
                    trace: serde_json::json!({ "summarizer": summarizer.as_str(), "orig_len": original.len(), "sum_len": final_summary.len(), "patched": patched_applied }),
                    artifacts: vec![],
                    meta: CapsuleMeta {
                        capsule_id: None,
                        agent_id: Some("core".to_string()),
                        lobe: Some(lobe.to_string()),
                        t_start_ms: now_ms,
                        t_end_ms: now_ms,
                        source: CapsuleSource::Derived,
                        schema_ver: "1.0".to_string(),
                        capsule_hash: None,
                        issuer_signature: None,
                        parent_id,
                    },
                };
                let store_clone = store.clone();
                let mem_id = c.id.clone();
                let lobe_c = lobe.to_string();
                let annot = CapsAnnot { verdict: verdict_variant, risk: risk_score, labels, policy_ver: "default".into(), patch_id: None, ts_ms: now_ms };
                std::thread::spawn(move || {
                    if let Ok(handle) = store_clone.ingest_capsule(cap) {
                        let _ = store_clone.map_memory(&mem_id, &handle.id);
                        let _ = store_clone.annotate(&handle.id, &annot);

                        // Publish reward event (best-effort)
                        if let Ok(sink) = crate::services::reward::RewardSqliteSink::open_default() {
                            let parent = store_clone.capsule_for_memory(&mem_id).ok().flatten();
                            let ev = crate::services::reward::RewardEvent {
                                lobe: lobe_c.clone(),
                                capsule_id: handle.id.clone(),
                                parent_id: parent,
                                value: crate::services::reward::reward_from_annotation(&annot),
                                ts_ms: annot.ts_ms as i64,
                                labels: annot.labels.clone(),
                                verdict: format!("{:?}", annot.verdict),
                                risk: annot.risk,
                            };
                            let _ = sink.publish(&ev);
                        }

                        // Assemble step and update value: use memory_id as state, no next_state yet.
                        if let Ok(asm) = crate::services::learner::StepAssembler::open_default() {
                            let _ = asm.record_from_reward(&lobe_c, &mem_id, &handle.id, crate::services::reward::reward_from_annotation(&annot), annot.ts_ms as i64);
                        }
                    }
                });
            }

            // Replace memory content with final (possibly patched) summary (original archived/sidecar).
            if let Err(e) = self.memory.replace_with_summary(&c.id, &final_summary) {
                report.notes.push(format!("replace_with_summary failed {}: {}", c.id, e));
                report.regrets += 1;
                continue;
            }

            report.compressed += 1;
        }

        if report.candidates > 0 {
            let regret_rate = report.regrets as f32 / report.candidates as f32;
            report.notes.push(format!(
                "regret_rate {:.2} (regrets={}, candidates={})",
                regret_rate, report.regrets, report.candidates
            ));
        }

        Ok(())
    }

    fn choose_summarizer(&self, policy: &CompactionPolicy) -> SummarizerKind {
        // Prefer explicit policy field if present (Option A).
        #[allow(clippy::match_like_matches_macro)]
        {
            // if used Option B (accessor), uncomment next lines:
            // if let Some(kind) = policy.summarizer_kind() {
            //     return kind;
            // }
        }
        // Default fallback
        #[allow(deprecated)]
        {
            policy.summarizer.clone_or_default()
        }
    }

    fn fetch_original(&self, id: &str) -> Result<String> {
        if let Ok(text) = self.memory.get_content(id) {
            return Ok(text);
        }
        if let Ok(text) = self.memory.get_raw(id) {
            return Ok(text);
        }
        Err(anyhow!("no accessor for original content"))
    }

    fn invoke_summarizer(&self, text: &str, kind: SummarizerKind) -> Result<String> {
        if let Ok(s) = self.memory.summarize(kind.clone(), text) {
            if !s.trim().is_empty() {
                return Ok(s);
            }
        }
        // Heuristic fallback to keep the pipeline robust
        let trimmed = text.trim();
        let snippet = if trimmed.len() > 512 {
            format!("{} … (truncated)", &trimmed[..512])
        } else {
            trimmed.to_string()
        };
        Ok(format!(
            "[summary:{} chars={}]\n{}",
            kind.as_str(),
            trimmed.len(),
            snippet
        ))
    }

    fn eval_summary_with_contracts(&self, summary: &str) -> Result<(Verdict, Option<Vec<String>>, f32, Option<String>)> {
        // Load default contract from configured directory and evaluate text.
        let cfg = ensure_initialized_once()?.config.clone();
        let contract_path = cfg.contracts.path.join(&cfg.contracts.default_contract);
    let mc: MoralContract = load_or_default(contract_path.to_string_lossy().as_ref());
        let res: EvaluationResult = evaluate_input_against_rules(summary, &mc);

        // Map severity to a simple risk score for scaffolding.
        fn sev_rank(s: Option<&str>) -> i32 {
            match s.unwrap_or("").to_ascii_lowercase().as_str() {
                "critical" => 4,
                "high" => 3,
                "medium" => 2,
                "low" => 1,
                _ => 0,
            }
        }
        fn sev_to_risk(rank: i32) -> f32 {
            match rank { 4 => 1.0, 3 => 0.75, 2 => 0.5, 1 => 0.25, _ => 0.0 }
        }

        if res.passed {
            return Ok((Verdict::Allow, None, 0.0, None));
        }

        // If any constraint encodes a mask directive, treat as AllowWithPatch; else Quarantine.
        let mut mask_patterns: Vec<String> = Vec::new();
        for c in &res.constraints {
            let t = c.trim();
            if let Some(stripped) = t.strip_prefix("mask:") {
                let p = stripped.trim();
                if !p.is_empty() { mask_patterns.push(p.to_string()); }
            } else if let Some(stripped) = t.strip_prefix("redact:") {
                let p = stripped.trim();
                if !p.is_empty() { mask_patterns.push(p.to_string()); }
            }
        }

        let mut max_rank = 0;
        for r in &res.violated_rules {
            let rank = sev_rank(r.severity.as_deref());
            if rank > max_rank { max_rank = rank; }
        }
        let risk = sev_to_risk(max_rank);
        let reason = res.reason.clone();

        if !mask_patterns.is_empty() {
            Ok((Verdict::AllowWithPatch, Some(mask_patterns), risk, Some(reason)))
        } else {
            Ok((Verdict::Quarantine, None, risk, Some(reason)))
        }
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

// Masking helpers are centralized in crate::services::masking
