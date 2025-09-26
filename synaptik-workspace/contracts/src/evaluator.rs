use crate::types::{ContractRule, MoralContract};
use crate::normalize::for_rules;
use serde::Serialize;
use std::{collections::HashSet, fs, thread, time::Duration};
use rand::{Rng, SeedableRng};
use rand::rngs::StdRng;
use toml;

// ----------------- Result -----------------

#[derive(Debug, Clone, Serialize)]
pub struct EvaluationResult {
    pub passed: bool,
    pub violated_rules: Vec<ContractRule>,
    pub reason: String,
    pub primary_violation_code: Option<String>,
    pub action_suggestion: Option<String>,

    // NEW: binding, deduped constraints from matched rules
    #[serde(default)]
    pub constraints: Vec<String>,
}

// ----------------- I/O -----------------

/// Result type for contract loading.
pub type LoadResult<T> = std::result::Result<T, LoadError>;

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("io: {0}")] Io(#[from] std::io::Error),
    #[error("empty_after_retries path={path}")] Empty { path: String },
    #[error("parse path={path} error={err}")] Parse { path: String, err: String },
}

/// Exponential backoff schedule (ms) with jitter 0..=10 ms added each attempt.
const BACKOFF_SERIES: [u64;5] = [25, 50, 100, 200, 400];

/// Attempt to load and parse a contract with resilience and without panicking.
/// Returns a MoralContract on success or a LoadError. Caller may decide to fallback.
pub fn load_contract_from_file(path: &str) -> LoadResult<MoralContract> {
    let mut rng = StdRng::from_entropy();
    let mut last_content = String::new();
    for (i, base) in BACKOFF_SERIES.iter().enumerate() {
        match fs::read_to_string(path) {
            Ok(c) => {
                if c.trim().is_empty() {
                    last_content = c;
                    if i + 1 == BACKOFF_SERIES.len() {
                        return Err(LoadError::Empty { path: path.to_string() });
                    }
                } else {
                    last_content = c;
                    break;
                }
            }
            Err(e) => {
                if i + 1 == BACKOFF_SERIES.len() {
                    return Err(LoadError::Io(e));
                }
            }
        }
        // Sleep with jitter before next attempt if not last
        let jitter = rng.gen_range(0..=10);
        thread::sleep(Duration::from_millis(*base + jitter));
    }

    ATP_COUNTER.fetch_add(ATP_COST_LOAD, std::sync::atomic::Ordering::Relaxed);
    toml::from_str(&last_content).map_err(|e| LoadError::Parse { path: path.to_string(), err: e.to_string() })
}

/// Fallback helper: attempt load, else synthesize a minimal inert contract so callers can proceed.
pub fn load_or_default(path: &str) -> MoralContract {
    match load_contract_from_file(path) {
        Ok(c) => c,
        Err(e) => {
            // Minimal transparent fallback contract (no rules) to keep pipeline alive; log via eprintln.
            eprintln!("[contracts] fallback to empty contract: {}", e);
            MoralContract { name: "fallback".into(), version: "0".into(), description: Some("Empty fallback contract".into()), rules: vec![] }
        }
    }
}

// ----------------- ATP (gas-like) accounting -----------------
use std::sync::atomic::{AtomicU64, Ordering as _Ordering};
/// Global ATP counter (monotonic). Represents cumulative metabolic cost of contract eval operations.
pub static ATP_COUNTER: AtomicU64 = AtomicU64::new(0);
/// Nominal ATP costs (tunable heuristics).
pub const ATP_COST_LOAD: u64 = 10; // loading/parsing a contract
pub const ATP_COST_EVAL_RULE: u64 = 1; // per rule evaluated

/// Session-scoped ATP meter: snapshot then measure delta.
#[derive(Debug, Clone)]
pub struct AtpSession {
    start: u64,
}
impl AtpSession {
    pub fn start() -> Self { Self { start: ATP_COUNTER.load(_Ordering::Relaxed) } }
    pub fn delta(&self) -> u64 { ATP_COUNTER.load(_Ordering::Relaxed).saturating_sub(self.start) }
}

// ----------------- Helpers -----------------


fn severity_rank(s: Option<&str>) -> i32 {
    match s {
        Some(sev) if sev.eq_ignore_ascii_case("critical") => 4,
        Some(sev) if sev.eq_ignore_ascii_case("high") => 3,
        Some(sev) if sev.eq_ignore_ascii_case("medium") => 2,
        Some(sev) if sev.eq_ignore_ascii_case("low") => 1,
        Some(sev) if sev.eq_ignore_ascii_case("none") => 0,
        _ => 0,
    }
}

fn rule_matches(rule: &ContractRule, text: &str) -> bool {
    let t = for_rules(text);
    // exact phrase (matches_any) first = more specific
    if let Some(list) = &rule.matches_any {
        for p in list {
            if !p.is_empty() && t.contains(&for_rules(p)) {
                return true;
            }
        }
    }
    // keyword (contains_any)
    if let Some(list) = &rule.contains_any {
        for k in list {
            if !k.is_empty() && t.contains(&for_rules(k)) {
                return true;
            }
        }
    }
    // legacy contains
    for k in &rule.contains {
        if !k.is_empty() && t.contains(&for_rules(k)) {
            return true;
        }
    }
    false
}

fn extend_constraints(dst: &mut HashSet<String>, rule: &ContractRule) {
    if let Some(list) = &rule.constraints {
        for c in list {
            if !c.trim().is_empty() {
                dst.insert(c.trim().to_string());
            }
        }
    }
}

// ----------------- Core -----------------

pub fn evaluate_input_against_rules(input: &str, contract: &MoralContract) -> EvaluationResult {
    // Pass 1: allowlist (takes precedence)
    let mut allow_constraints: HashSet<String> = HashSet::new();
    for rule in &contract.rules {
        ATP_COUNTER.fetch_add(ATP_COST_EVAL_RULE, _Ordering::Relaxed);
        let eff = rule.effect.as_deref().unwrap_or("");
        let is_allow = eff.eq_ignore_ascii_case("allow")
            || rule
                .severity
                .as_deref()
                .map(|s| s.eq_ignore_ascii_case("none"))
                .unwrap_or(false)
            || eff.eq_ignore_ascii_case("allow_with_constraints");

        if is_allow && rule_matches(rule, input) {
            extend_constraints(&mut allow_constraints, rule);
            // Short-circuit on allow
            return EvaluationResult {
                passed: true,
                violated_rules: vec![],
                reason: "Input matches allowlisted pattern.".into(),
                primary_violation_code: None,
                action_suggestion: None,
                constraints: allow_constraints.into_iter().collect(),
            };
        }
    }

    // Pass 2: collect violations
    let mut violations: Vec<ContractRule> = Vec::new();
    let mut constraints: HashSet<String> = HashSet::new();

    for rule in &contract.rules {
        ATP_COUNTER.fetch_add(ATP_COST_EVAL_RULE, _Ordering::Relaxed);
        // Skip allow rules in violation pass
        let eff = rule.effect.as_deref().unwrap_or("");
        let is_allow = eff.eq_ignore_ascii_case("allow")
            || rule
                .severity
                .as_deref()
                .map(|s| s.eq_ignore_ascii_case("none"))
                .unwrap_or(false)
            || eff.eq_ignore_ascii_case("allow_with_constraints");
        if is_allow {
            continue;
        }

        if rule_matches(rule, input) {
            extend_constraints(&mut constraints, rule);
            violations.push(rule.clone());
        }
    }

    if violations.is_empty() {
        return EvaluationResult {
            passed: true,
            violated_rules: vec![],
            reason: "No violations detected.".into(),
            primary_violation_code: None,
            action_suggestion: None,
            constraints: vec![],
        };
    }

    // Pick primary by highest severity, then by specificity (matches_any > contains_any > contains)
    let mut best_idx = 0;
    let mut best_rank = -1i32;
    let mut best_specificity = -1i32;

    for (i, r) in violations.iter().enumerate() {
        let rank = severity_rank(r.severity.as_deref());
        let spec = if r
            .matches_any
            .as_ref()
            .map(|v| !v.is_empty())
            .unwrap_or(false)
        {
            3
        } else if r
            .contains_any
            .as_ref()
            .map(|v| !v.is_empty())
            .unwrap_or(false)
        {
            2
        } else if !r.contains.is_empty() {
            1
        } else {
            0
        };

        if rank > best_rank || (rank == best_rank && spec > best_specificity) {
            best_idx = i;
            best_rank = rank;
            best_specificity = spec;
        }
    }

    let primary = &violations[best_idx];

    EvaluationResult {
        passed: false,
        violated_rules: violations.clone(),
        reason: format!("Violated {} rule(s).", violations.len()),
        primary_violation_code: primary.violation_code.clone(),
        action_suggestion: primary.action_suggestion.clone(),
        constraints: constraints.into_iter().collect(),
    }
}
