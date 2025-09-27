//! services/ethos.rs
//! Contract-centric Ethos agent: risk + ethics precheck with unified auditing.

use serde::{Deserialize, Serialize};

use serde_json::json;

use crate::commands::init::ensure_initialized_once;
use crate::services::audit::{
    ContractEvalMeta, evaluate_and_audit_contract, record_ethics_decision,
};

/// Verdict returned by [`precheck`]: normalized signal from contracts.
///
/// ## Fields
/// - `risk` — `"Low" | "Medium" | "High" | "Critical"`
/// - `constraints` — list of soft constraints (e.g., `"request_clarification"`)
/// - `passed` — overall ethics pass/fail
/// - `reason` — human-readable rationale from the ethics contract
#[derive(Debug, Clone)]
pub struct EthosVerdict {
    pub risk: String,
    pub constraints: Vec<String>,
    pub passed: bool,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Allow,
    AllowWithConstraints,
    Block,
}

// -------------------------------------------------------------------------
// Contract-enforced runtime primitives
// -------------------------------------------------------------------------

/// Proposal received at the runtime ingress.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Proposal {
    pub intent: String,
    pub input: String,
    #[serde(default)]
    pub prior: Option<String>,
    #[serde(default)]
    pub tools_requested: Vec<String>,
}

/// Constraint specification for Constrain() decisions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConstraintSpec {
    #[serde(default)]
    pub mask_rules: Vec<String>, // regex or literal patterns (we use literal substrings in MVP)
    #[serde(default)]
    pub allow_tools: Vec<String>, // tool names
    #[serde(default)]
    pub stop_phrases: Vec<String>, // early stop triggers
    #[serde(default = "ConstraintSpec::default_max_tokens")]
    pub max_tokens: usize,
    #[serde(default = "ConstraintSpec::default_temperature_cap")]
    pub temperature_cap: f32,
}

impl ConstraintSpec {
    fn default_max_tokens() -> usize {
        256
    }
    fn default_temperature_cap() -> f32 {
        0.7
    }
}

/// Runtime decision emitted by an ethos contract.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RuntimeDecision {
    Proceed,
    Constrain(ConstraintSpec),
    Stop { safe_template: String },
    Escalate { reason: String },
}

/// Contract interface for runtime decisions.
pub trait EthosContract {
    fn evaluate(&self, p: &Proposal) -> RuntimeDecision;
}

// A contracts-driven decider that delegates to the contracts package (WASM-capable backend).
// Note: The actual WASM execution lives in the contracts crate; here we only call into its API
// via the audit bridge, which resolves the configured contract and records evaluation.
pub struct ContractsDecider;

impl EthosContract for ContractsDecider {
    fn evaluate(&self, p: &Proposal) -> RuntimeDecision {
        // Always evaluate contracts; no disable path.
        let cfg = ensure_initialized_once()
            .map(|r| r.config.clone())
            .unwrap_or_default();
        let default_name = Some(cfg.contracts.default_contract.clone());
        let eval = match crate::services::audit::evaluate_and_audit_contract(
            &ContractEvalMeta {
                kind: "Ethics".into(),
                contract_name: default_name,
                metadata: json!({ "intent": p.intent }), // borrow to avoid moving from &Proposal
            },
            &p.input,
        ) {
            Ok(v) => v,
            Err(_) => json!({"passed": false}),
        };

        let passed = eval.get("passed").and_then(|v| v.as_bool()).unwrap_or(true);
        if !passed {
            return RuntimeDecision::Stop { safe_template: "I can’t assist with that. If you’re concerned about safety, consider reaching out to local resources for help.".to_string() };
        }

        // Soft constraints: if any were suggested by policy, apply a conservative runtime spec.
        let constraints = eval
            .get("constraints")
            .and_then(|v| v.as_array())
            .map(|arr| !arr.is_empty())
            .unwrap_or(false);
        if constraints {
            return RuntimeDecision::Constrain(ConstraintSpec {
                mask_rules: vec![],
                allow_tools: vec![],
                stop_phrases: vec!["how to".into(), "step by step".into()],
                max_tokens: ConstraintSpec::default_max_tokens(),
                temperature_cap: ConstraintSpec::default_temperature_cap(),
            });
        }

        RuntimeDecision::Proceed
    }
}

/// Synchronous, contract-backed risk + ethics check.
///
/// # Arguments
/// * `candidate_text` - The text to evaluate (original user input or generated note).
/// * `intent_label`   - Short label describing the operation context (e.g., `"memory_storage"`, `"reflection_update"`, `"metadata_access"`).
///
/// # Returns
/// * `Ok(EthosVerdict)` with normalized `risk`, `constraints`, `passed`, and `reason`.
///
/// # Errors
/// * `Err(String)` if contract evaluation fails or returns malformed JSON.
///
/// # Side effects
/// * Writes a normalized ethics decision entry to the logbook via [`record_ethics_decision`].
/// * Also logs the raw contract evaluations (risk + ethics) via [`evaluate_and_audit_contract`].
pub fn precheck(candidate_text: &str, intent_label: &str) -> Result<EthosVerdict, String> {
    // 1) Risk assessment
    let risk_val = evaluate_and_audit_contract(
        &ContractEvalMeta {
            kind: "RiskAssessor".into(),
            contract_name: Some("nonviolence".into()), // renamed
            metadata: json!({ "intent": intent_label }),
        },
        candidate_text,
    )?;

    // 2) Ethics evaluation (same contract for now)
    let ethics_val = evaluate_and_audit_contract(
        &ContractEvalMeta {
            kind: "Ethics".into(),
            contract_name: Some("nonviolence".into()), // renamed
            metadata: json!({}),
        },
        candidate_text,
    )?;

    // 3) Normalize + derive effective risk
    let passed = ethics_val["passed"].as_bool().unwrap_or(true);
    let reason = ethics_val["reason"].as_str().unwrap_or("").to_string();

    // Derive risk from either an explicit risk field, or from the highest rule severity.
    fn sev_rank(s: &str) -> i32 {
        match s.to_ascii_lowercase().as_str() {
            "critical" => 4,
            "high" => 3,
            "medium" => 2,
            "low" => 1,
            _ => 0,
        }
    }
    fn rank_to_label(r: i32) -> &'static str {
        match r {
            4 => "Critical",
            3 => "High",
            2 => "Medium",
            1 => "Low",
            _ => "Low",
        }
    }

    // Pull any explicit risk if present
    let mut effective_rank = 0;
    if let Some(rsk) = risk_val.get("risk").and_then(|v| v.as_str()) {
        effective_rank = sev_rank(rsk);
    }
    // Merge in highest violated rule severity from ethics result
    if let Some(arr) = ethics_val.get("violated_rules").and_then(|v| v.as_array()) {
        for v in arr {
            if let Some(sev) = v.get("severity").and_then(|s| s.as_str()) {
                let r = sev_rank(sev);
                if r > effective_rank {
                    effective_rank = r;
                }
            }
        }
    }
    // If we blocked but still somehow have Low, bump to at least High to reflect violation gravity
    if !passed && effective_rank == 0 {
        effective_rank = 3;
    }
    let risk = rank_to_label(effective_rank).to_string();
    let constraints = ethics_val["constraints"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .filter_map(|v| v.as_str().map(|s| s.to_string()))
        .collect::<Vec<_>>();

    record_ethics_decision(intent_label, passed, &risk, &constraints, &reason);

    Ok(EthosVerdict {
        risk,
        constraints,
        passed,
        reason,
    })
}

// Contract checks are always enabled; no feature flag bypass.

/// Map an [`EthosVerdict`] into an actionable gate decision.
///
/// # Arguments
/// * `verdict` - The result of [`precheck`].
///
/// # Returns
/// * `Decision::Block` if `!passed` **or** `risk ∈ { "High", "Critical" }`.
/// * `Decision::AllowWithConstraints` if constraints exist **or** `risk == "Medium"`.
/// * `Decision::Allow` otherwise.
pub fn decision_gate(verdict: &EthosVerdict) -> Decision {
    if !verdict.passed || matches!(verdict.risk.as_str(), "High" | "Critical") {
        Decision::Block
    } else if !verdict.constraints.is_empty() || verdict.risk == "Medium" {
        Decision::AllowWithConstraints
    } else {
        Decision::Allow
    }
}
