// src/services/compactor.rs
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

use crate::config::{CompactionPolicy, SummarizerKind};
use crate::services::memory::{Memory, MemoryCandidate};
use crate::services::ethos::{precheck, decision_gate, Decision};
use crate::utils::pons::PonsStore;

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

            // Ethos / safety precheck via centralized contracts (best-effort).
            let (ok, reason, risk) = self.ethos_precheck(&summary)?;
            if !ok {
                match (risk, reason) {
                    (Some(risk), Some(reason)) => {
                        report
                            .notes
                            .push(format!("ethos rejected summary {}: {} - {}", c.id, risk, reason));
                    }
                    (Some(risk), None) => {
                        report
                            .notes
                            .push(format!("ethos rejected summary {}: {}", c.id, risk));
                    }
                    (None, Some(reason)) => {
                        report
                            .notes
                            .push(format!("ethos rejected summary {}: {}", c.id, reason));
                    }
                    _ => {
                        report
                            .notes
                            .push(format!("ethos rejected summary {}", c.id));
                    }
                }
                report.regrets += 1;
                continue;
            }

            // Replace memory content with summary (original archived/sidecar).
            if let Err(e) = self.memory.replace_with_summary(&c.id, &summary) {
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
            // if you used Option B (accessor), uncomment next lines:
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

    fn ethos_precheck(&self, summary: &str) -> Result<(bool, Option<String>, Option<String>)> {
        // Delegate to the shared ethos precheck to avoid duplicated logic.
        // If contracts evaluation fails, default to allow (keep pipeline robust).
        match precheck(summary, "compaction_summary") {
            Ok(verdict) => {
                let dec = decision_gate(&verdict);
                let allowed = !matches!(dec, Decision::Block);
                if allowed {
                    Ok((true, None, None))
                } else {
                    Ok((false, Some(verdict.reason), Some(verdict.risk)))
                }
            }
            Err(_e) => Ok((true, None, None)),
        }
    }
}
