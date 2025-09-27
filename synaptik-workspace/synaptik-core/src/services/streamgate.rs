// synaptik-core/src/services/streamgate.rs

use anyhow::Result;
use serde::Serialize;
use contracts::types::{ContractRule, MoralContract};
use contracts::normalize::for_rules;
use std::sync::Arc;
use std::time::Instant;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateDecision {
    Pass,
    Hold,
    CutAndReplace(String),
}

#[derive(Debug, Clone)]
pub struct StreamGateConfig {
    pub budget_ms: u64,
    pub window_bytes: usize,
    pub fail_closed_on_finalize: bool,
}

// Normalization delegated to contracts::normalize to keep consistent with evaluator.
fn norm(s: &str) -> String { for_rules(s) }

/// Minimal evaluation result to satisfy your debug test
#[derive(Debug, Clone)]
pub struct EvalResult {
    pub passed: bool,
    pub violated_rules: Vec<ContractRule>,
}

#[derive(Debug)]
pub struct StreamingIndex {
    _action: String,
    idiom_allowlist: Vec<String>, // from matches_any with effect=allow
    rules_for_action: Vec<ContractRule>, // all rules with this action
}

impl StreamingIndex {
    pub fn from_contract_for_action(contract: MoralContract, action: &str) -> Result<Self> {
        let mut idioms = Vec::new();
        let mut rules = Vec::new();

        for r in contract.rules.into_iter() {
            if !r.action.eq_ignore_ascii_case(action) {
                continue;
            }

            // Collect allowlist idioms from matches_any when effect == Some("allow")
            if r.effect
                .as_deref()
                .map(|e| e.eq_ignore_ascii_case("allow"))
                .unwrap_or(false)
            {
                if let Some(list) = &r.matches_any {
                    for m in list {
                        let n = norm(m);
                        if !n.is_empty() {
                            idioms.push(n);
                        }
                    }
                }
            }

            rules.push(r);
        }

        Ok(Self {
            _action: action.to_string(),
            idiom_allowlist: idioms,
            rules_for_action: rules,
        })
    }

    /// Public: used by your debug test
    pub fn evaluate_chunk(&self, text: &str) -> EvalResult {
        let ntext = norm(text);

        // idiom allowlist = pass
        for idiom in &self.idiom_allowlist {
            if ntext.contains(idiom) {
                return EvalResult {
                    passed: true,
                    violated_rules: vec![],
                };
            }
        }

        let mut violated = Vec::new();

        'rules: for r in &self.rules_for_action {
            // matches_any (exact substring)
            if let Some(list) = &r.matches_any {
                for m in list {
                    if !m.is_empty() && ntext.contains(&norm(m)) {
                        violated.push(r.clone());
                        continue 'rules;
                    }
                }
            }
            // contains_any (legacy-ish, treat as substring too)
            if let Some(list) = &r.contains_any {
                for m in list {
                    if !m.is_empty() && ntext.contains(&norm(m)) {
                        violated.push(r.clone());
                        continue 'rules;
                    }
                }
            }
            // legacy `contains` vector (keep compatibility)
            if !r.contains.is_empty() {
                for m in &r.contains {
                    if !m.is_empty() && ntext.contains(&norm(m)) {
                        violated.push(r.clone());
                        continue 'rules;
                    }
                }
            }
        }

        EvalResult {
            passed: violated.is_empty(),
            violated_rules: violated,
        }
    }

    /// Public: used by your debug test
    pub fn has_pronouns(&self, text: &str) -> bool {
        let t = norm(text);
        // simple pronoun heuristic for “personal threat”
        let pronouns = [
            "him ", " her ", " them ", " you ", " your ", " his ", " their ", " my ", " me ",
            " she ", " he ",
        ];
        // also check end-of-text cases by padding spaces
        let padded = format!(" {} ", t);
        pronouns.iter().any(|p| padded.contains(p))
    }

    /// Is the violation a high-severity personal threat (needs pronouns)
    fn is_personal_threat(&self, rule: &ContractRule, text: &str) -> bool {
        let is_high = rule
            .severity
            .as_deref()
            .map(|s| s.eq_ignore_ascii_case("high"))
            .unwrap_or(false);
        if !is_high {
            return false;
        }
        self.has_pronouns(text)
    }

    /// Get a suggestion string to use when cutting.
    fn suggestion_for(&self, violated: &[ContractRule]) -> String {
        // first available action_suggestion, else generic
        for r in violated {
            if let Some(s) = &r.action_suggestion {
                if !s.trim().is_empty() {
                    return s.clone();
                }
            }
        }
        "I won't help with harm. Let’s discuss something constructive.".to_string()
    }
}

pub struct StreamGate {
    index: Arc<StreamingIndex>,
    _cfg: StreamGateConfig,
    saw_violation: bool, // any non-idiom violation seen
    is_held: bool,       // streaming hold
    pending_cut_msg: Option<String>,
}

impl StreamGate {
    pub fn from_index(index: Arc<StreamingIndex>, cfg: StreamGateConfig) -> Self {
        Self {
            index,
            _cfg: cfg,
            saw_violation: false,
            is_held: false,
            pending_cut_msg: None,
        }
    }

    pub fn push(&mut self, chunk: &str) -> GateDecision {
        // budget/window enforcement can be added later; no-op for now

        // Evaluate this chunk
        let eval = self.index.evaluate_chunk(chunk);
        if eval.passed {
            // If we were already holding, keep holding (don’t flicker)
            return if self.is_held {
                GateDecision::Hold
            } else {
                GateDecision::Pass
            };
        }

        // There are violated rules. Decide:
        // If any is a personal threat (high severity + pronouns), cut immediately
        if eval
            .violated_rules
            .iter()
            .any(|r| self.index.is_personal_threat(r, chunk))
        {
            let suggestion = self.index.suggestion_for(&eval.violated_rules);
            self.pending_cut_msg = Some(suggestion.clone());
            self.saw_violation = true;
            // no need to set is_held; we’re cutting right now
            return GateDecision::CutAndReplace(suggestion);
        }

        // Otherwise: generic harm -> HOLD now, CUT at finalize
        self.saw_violation = true;
        self.is_held = true;
        GateDecision::Hold
    }

    pub fn finalize(&mut self) -> GateDecision {
        if self.saw_violation {
            // If we have a suggestion from earlier, reuse; else derive from rules
            let msg = self.pending_cut_msg.take().unwrap_or_else(|| {
                // Create a generic suggestion when we didn’t cut immediately
                "I can’t assist with harm. Let’s switch to a safe, constructive topic.".to_string()
            });
            return GateDecision::CutAndReplace(msg);
        }

        // No violations observed
        GateDecision::Pass
    }
}

// -------------------------------------------------------------------------
// Contract-enforced runtime (synchronous skeleton)
// -------------------------------------------------------------------------

use crate::services::audit;
use crate::services::ethos::{ConstraintSpec, EthosContract, Proposal, RuntimeDecision};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum FinalizedStatus {
    Ok,
    Violated,
    Stopped,
    Escalated,
}

#[derive(Debug, Clone, Serialize)]
pub struct Finalized {
    pub status: FinalizedStatus,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub violation_label: Option<String>,
}

impl Finalized {
    pub fn ok(text: String) -> Self {
        Self { status: FinalizedStatus::Ok, text, violation_label: None }
    }
    pub fn violated(text: String, label: String) -> Self {
        Self { status: FinalizedStatus::Violated, text, violation_label: Some(label) }
    }
    pub fn stopped(template: String) -> Self {
        Self { status: FinalizedStatus::Stopped, text: template, violation_label: None }
    }
    pub fn escalated(reason: String) -> Self {
        Self { status: FinalizedStatus::Escalated, text: reason, violation_label: None }
    }
    pub fn is_ok(&self) -> bool { matches!(self.status, FinalizedStatus::Ok) }
}

#[derive(Debug)]
pub struct GateError(pub String);

impl std::fmt::Display for GateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl std::error::Error for GateError {}

/// Minimal sync streaming client contract; adapt as needed.
pub trait LlmClient {
    type Stream: Iterator<Item = String>;
    fn stream(&self, system_prompt: String) -> std::result::Result<Self::Stream, GateError>;
}

pub struct StreamRuntime<C: EthosContract, M: LlmClient> {
    pub contract: C,
    pub model: M,
}

impl<C: EthosContract, M: LlmClient> StreamRuntime<C, M> {
    pub fn generate(&self, p: Proposal) -> std::result::Result<Finalized, GateError> {
        // Global safeguards to prevent resource exhaustion in absence of explicit constraints
        const DEFAULT_MAX_TOKENS: usize = 512;           // fallback token cap
        const DEFAULT_MAX_OUTPUT_BYTES: usize = 64 * 1024; // 64 KiB output cap
        const DEFAULT_BUDGET_MS: u128 = 3_000;           // wall-clock budget

        let decision = self.contract.evaluate(&p);
        audit::log_proposal(&p, &decision);

        match decision {
            RuntimeDecision::Stop { safe_template } => {
                return Ok(Finalized::stopped(safe_template));
            }
            RuntimeDecision::Escalate { ref reason } => {
                audit::log_escalation(&p, reason);
                return Ok(Finalized::escalated(reason.clone()));
            }
            RuntimeDecision::Proceed | RuntimeDecision::Constrain(_) => {}
        }

        let constraints = match &decision {
            RuntimeDecision::Constrain(spec) => Some(spec.clone()),
            _ => None,
        };

        let sys_prompt = prompt_compile(&p, constraints.as_ref());
        let mut stream = self.model.stream(sys_prompt)?;
        let t0 = Instant::now();

        let mut buf = String::new();
        let mut violated: Option<String> = None;

        // Precompute a conservative window size to remask on each token.
        // Use normalized pattern lengths to cover cross-token matches; grow by a
        // generous UTF-8 factor and clamp to avoid unbounded work.
        let window_bytes: usize = constraints
            .as_ref()
            .map(|spec| {
                let max_pat = spec
                    .mask_rules
                    .iter()
                    .map(|p| crate::services::masking::norm_lower(p).chars().count())
                    .max()
                    .unwrap_or(0);
                // Fallback minimum to catch short patterns; cap to keep cost bounded.
                let margin = (max_pat.saturating_mul(8)).max(128).min(4096);
                margin
            })
            .unwrap_or(0);

        let fallback_max_tokens = DEFAULT_MAX_TOKENS;
        while let Some(tok) = stream.next() {
            // Hard stop: wall-clock budget exceeded
            if t0.elapsed().as_millis() >= DEFAULT_BUDGET_MS {
                break;
            }

            if let Some(spec) = &constraints {
                // Detect stop phrase across token boundaries (buf + tok)
                if hits_stop_phrase(&buf, &tok, &spec.stop_phrases) {
                    violated = Some("stop_phrase".to_string());
                    break;
                }

                // Append raw token, then apply masking over a suffix window so
                // patterns that straddle token boundaries are redacted without
                // reprocessing the entire buffer each time.
                buf.push_str(&tok);
                if !spec.mask_rules.is_empty() {
                    if window_bytes == 0 || buf.len() <= window_bytes {
                        buf = crate::services::masking::apply_masks_ci(&buf, &spec.mask_rules);
                    } else {
                        let target = buf.len() - window_bytes;
                        // Find nearest char boundary <= target.
                        let mut start = 0usize;
                        for (i, _) in buf.char_indices() { if i <= target { start = i; } else { break; } }
                        let tail = buf[start..].to_string();
                        let masked_tail = crate::services::masking::apply_masks_ci(&tail, &spec.mask_rules);
                        buf.truncate(start);
                        buf.push_str(&masked_tail);
                    }
                }

                if token_limit_reached(&buf, spec.max_tokens) {
                    break;
                }
            } else {
                buf.push_str(&tok);
                // Fallback safeguards when no explicit constraints are present
                if token_limit_reached(&buf, fallback_max_tokens) {
                    break;
                }
            }

            // Output size guard regardless of constraints
            if buf.len() >= DEFAULT_MAX_OUTPUT_BYTES {
                break;
            }
        }

        let finalized = if let Some(lbl) = violated.clone() {
            audit::log_violation(&p, &lbl, &buf);
            Finalized::violated(buf, lbl)
        } else {
            Finalized::ok(buf)
        };

        // Memory write barrier
        if finalized.is_ok() {
            // Best-effort commit note: for MVP we log the commit event in memory::commit_snapshot
            let _ = crate::services::memory::commit_snapshot(&p, &decision, &finalized);
        } else {
            audit::log_violation(&p, "rejected_snapshot", &finalized.text);
        }

        Ok(finalized)
    }
}

fn prompt_compile(p: &Proposal, spec: Option<&ConstraintSpec>) -> String {
    let mut lines = vec![
        format!("You are an assistant. Intent: {}.", p.intent),
        "Adhere to the rules below strictly.".into(),
    ];
    if let Some(s) = spec {
        if !s.allow_tools.is_empty() {
            lines.push(format!("Only call tools: [{}]", s.allow_tools.join(", ")));
        }
        if !s.mask_rules.is_empty() {
            lines.push("Do not output sensitive content; redact it as [masked].".into());
        }
        if !s.stop_phrases.is_empty() {
            lines.push("If unsafe instructions are requested, stop and apologize.".into());
        }
    }
    lines.push("Respond helpfully and safely.".into());
    lines.join("\n")
}

fn norm_lower(s: &str) -> String { for_rules(s) }

fn hits_stop_phrase(buf: &str, tok: &str, stop_phrases: &[String]) -> bool {
    if stop_phrases.is_empty() { return false; }
    let binding = format!("{}{}", buf, tok);
    let hay = norm_lower(&binding);
    stop_phrases.iter().any(|s| !s.is_empty() && hay.contains(&norm_lower(s)))
}

// Build a normalized view (case / rule normalization) along with original byte spans.
// Each produced normalized char corresponds to an original (start,end) span. Characters
// removed by normalization (e.g., zero-width or control) emit no span entries so searches
// cannot accidentally shift and reveal trailing suffixes.
// normalized_chars_with_spans and apply_masks moved to crate::services::masking

fn token_limit_reached(buf: &str, max_tokens: usize) -> bool {
    if max_tokens == 0 { return false; }
    // rough approximation: whitespace tokens
    let cnt = buf.split_whitespace().count();
    cnt >= max_tokens
}
