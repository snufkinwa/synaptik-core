// synaptik-core/src/services/streamgate.rs

use anyhow::Result;
use serde::Serialize;
use contracts::types::{ContractRule, MoralContract};
use std::sync::Arc;

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

fn norm(s: &str) -> String {
    // NOTE: Keep in sync with evaluator.rs normalization. Consider moving to a shared util.
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        // Drop control characters early.
        if ch.is_control() {
            continue;
        }
        // Use Unicode-aware lowercasing. Some chars expand to multiple codepoints.
        for lc in ch.to_lowercase() {
            match lc {
                // Filter zero-width characters
                '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{2060}' | '\u{FEFF}' => {}
                _ => out.push(lc),
            }
        }
    }
    out
}

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

        let mut buf = String::new();
        let mut violated: Option<String> = None;

        while let Some(tok) = stream.next() {
            if let Some(spec) = &constraints {
                if hits_stop_phrase(&buf, &tok, &spec.stop_phrases) {
                    violated = Some("stop_phrase".to_string());
                    break;
                }
                let t = apply_masks(&tok, &spec.mask_rules);
                buf.push_str(&t);
                if token_limit_reached(&buf, spec.max_tokens) {
                    break;
                }
            } else {
                buf.push_str(&tok);
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

fn norm_lower(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if ch.is_control() { continue; }
        for lc in ch.to_lowercase() {
            match lc { '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{2060}' | '\u{FEFF}' => {}, _ => out.push(lc) }
        }
    }
    out
}

fn hits_stop_phrase(buf: &str, tok: &str, stop_phrases: &[String]) -> bool {
    if stop_phrases.is_empty() { return false; }
    let merged = format!("{}{}", buf, tok);
    let hay = norm_lower(&merged);
    stop_phrases.iter().any(|s| !s.is_empty() && hay.contains(&norm_lower(s)))
}

fn apply_masks(tok: &str, mask_rules: &[String]) -> String {
    if mask_rules.is_empty() { return tok.to_string(); }
    let mut out = tok.to_string();
    for pat in mask_rules {
        if pat.is_empty() { continue; }
        // simple case-insensitive substring replacement
        let lower_pat = norm_lower(pat);
        let mut idx = 0usize;
        while let Some(pos) = norm_lower(&out[idx..]).find(&lower_pat) {
            let start = idx + pos;
            let end = start + out[start..].chars().take(pat.chars().count()).map(char::len_utf8).sum::<usize>();
            out.replace_range(start..end, "[masked]");
            idx = start + "[masked]".len();
        }
    }
    out
}

fn token_limit_reached(buf: &str, max_tokens: usize) -> bool {
    if max_tokens == 0 { return false; }
    // rough approximation: whitespace tokens
    let cnt = buf.split_whitespace().count();
    cnt >= max_tokens
}
