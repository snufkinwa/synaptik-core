use crate::types::{ContractRule, MoralContract};
use serde::Serialize;
use std::{collections::HashSet, fs};
use toml;

// ----------------- Result -----------------

#[derive(Debug, Clone, Serialize)]
pub struct EvaluationResult {
    pub passed: bool,
    pub violated_rules: Vec<ContractRule>,
    pub reason: String,
    pub primary_violation_code: Option<String>,
    pub action_suggestion: Option<String>,

    // NEW: merged, deduped constraints from matched rules
    #[serde(default)]
    pub constraints: Vec<String>,
}

// ----------------- I/O -----------------

pub fn load_contract_from_file(path: &str) -> MoralContract {
    let content = fs::read_to_string(path).expect("Failed to read contract file");
    toml::from_str(&content).expect("Failed to parse TOML")
}

// ----------------- Helpers -----------------

fn norm(s: &str) -> String {
    // lowercase; skip control / zero-width
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if ch.is_control() {
            continue;
        }
        match ch {
            '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{2060}' | '\u{FEFF}' => {}
            _ => out.push(ch.to_ascii_lowercase()),
        }
    }
    out
}

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
    let t = norm(text);
    // exact phrase (matches_any) first = more specific
    if let Some(list) = &rule.matches_any {
        for p in list {
            if !p.is_empty() && t.contains(&norm(p)) {
                return true;
            }
        }
    }
    // keyword (contains_any)
    if let Some(list) = &rule.contains_any {
        for k in list {
            if !k.is_empty() && t.contains(&norm(k)) {
                return true;
            }
        }
    }
    // legacy contains
    for k in &rule.contains {
        if !k.is_empty() && t.contains(&norm(k)) {
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
