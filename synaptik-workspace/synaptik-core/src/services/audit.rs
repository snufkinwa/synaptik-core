//! services/audit.rs
//! Contract-aware audit logbook: actions, ethics decisions, and raw contract evaluations.
//!
//! - Writes JSONL files under `.cogniv/logbook/`.
//! - Bridges to the `contracts` crate via `evaluate_contract_json` and normalizes results.

use chrono::{DateTime, Utc};
use once_cell::sync::OnceCell;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::commands::init::ensure_initialized_once;
use crate::config::{CoreConfig, PoliciesConfig};
use contracts::assets::{read_verified_or_embedded, write_default_contracts};
use contracts::{MoralContract, evaluate_input_against_rules};
// ----------- Logbook paths -----------

static CONTRACTS_LOCKED: AtomicBool = AtomicBool::new(true);

/// Lock contract files to their embedded versions.
pub fn lock_contracts() {
    CONTRACTS_LOCKED.store(true, Ordering::SeqCst);
}

/// Allow local edits to contract files.
pub fn unlock_contracts() {
    CONTRACTS_LOCKED.store(false, Ordering::SeqCst);
}

/// When contracts are locked, periodically or opportunistically restore the on-disk
/// contract files to the embedded canonical versions. This helps ensure that any
/// accidental edits on disk are reverted and keeps the filesystem consistent.
fn enforce_contracts_on_disk_if_locked() {
    if !CONTRACTS_LOCKED.load(Ordering::SeqCst) {
        return;
    }
    if let Ok(report) = ensure_initialized_once() {
        let contracts = &report.config.contracts;
        let dir = contracts.path.clone();
        // Best effort: restore the known canonical contract files unconditionally.
        let known = [contracts.default_contract.as_str()];
        for name in known {
            let path = dir.join(name);
            if let Ok(text) = read_verified_or_embedded(&path, name, true) {
                let _ = std::fs::write(&path, text.as_ref());
            }
        }
        // Also seed any missing files via the helper (idempotent)
        let _ = write_default_contracts(&dir);
        record_action(
            "audit",
            "contracts_restored",
            &json!({ "dir": dir.to_string_lossy() }),
            "low",
        );
    }
}

// ----------- Public API -----------

/// Lightweight metadata describing a contract evaluation request.
///
/// # Fields
/// - `kind` — Logical contract type (e.g., `"Ethics"`, `"RiskAssessor"`, custom labels).
/// - `contract_name` — Optional human-friendly name/version of the contract.
/// - `metadata` — Arbitrary JSON passed along to the contract engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractEvalMeta {
    pub kind: String,
    #[serde(default)]
    pub contract_name: Option<String>,
    #[serde(default)]
    pub metadata: Value,
}

/// A normalized record of a single contract evaluation, suitable for JSONL logging.
///
/// # Fields
/// - `timestamp` — When the evaluation finished.
/// - `kind` — The contract kind (same as in [`ContractEvalMeta`]).
/// - `contract_name` — Optional name of the contract used.
/// - `input_preview` — Redacted preview of the evaluated input.
/// - `latency_ms` — End-to-end evaluation latency in milliseconds.
/// - `result` — Exact JSON result returned by the contracts engine.
/// - `metadata` — Metadata echoed from the request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractEvalRecord {
    pub timestamp: DateTime<Utc>,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contract_name: Option<String>,
    pub input_preview: String,
    pub latency_ms: f64,
    pub result: Value,
    #[serde(default)]
    pub metadata: Value,
}

/// Normalized ethics decision used for quick querying and violation reporting.
///
/// # Fields
/// - `timestamp` — When the decision was recorded.
/// - `intent_category` — Operation label (e.g., `"memory_storage"`).
/// - `passed` — Overall pass/fail.
/// - `risk` — `"Low" | "Medium" | "High" | "Critical"`.
/// - `constraints` — Soft constraints requested by policy.
/// - `reason` — Human-readable rationale (if provided).
/// - `requires_escalation` — Derived flag: true if `!passed` or high risk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EthicsDecision {
    pub timestamp: DateTime<Utc>,
    pub intent_category: String,
    pub passed: bool,
    pub risk: String,
    pub constraints: Vec<String>,
    pub reason: String,
    #[serde(default)]
    pub requires_escalation: bool,
}

/// Initialize the audit agent (idempotent).
///
/// # Behavior
/// - Ensures the log directory exists.
/// - Prints basic readiness info (useful in CLI demos).
///
/// # Returns
/// Nothing. Panics are avoided; directory creation failures are ignored silently here
/// because individual writers also create parents as needed.
pub fn start() {
    println!(" Audit Agent — Contract-aware compliance logging");
    ensure_dirs();
    println!("   • Logbook directory: {}", log_paths().dir.display());
    println!("   • Ready.");
}

/// Record a generic action event (lightweight telemetry).
///
/// # Arguments
/// * `agent` — Logical component name (e.g., `"commands"`, `"streamgate"`).
/// * `action` — Short verb label (e.g., `"remember_called"`, `"reflect_set"`).
/// * `details` — Arbitrary JSON payload (parameters, ids, etc.).
/// * `severity` — `"low" | "medium" | "high"` — for quick triage.
///
/// # Returns
/// Nothing. Appends a single JSON object to `actions.jsonl`.
pub fn record_action(agent: &str, action: &str, details: &Value, severity: &str) {
    if !audit_enabled() {
        return;
    }
    let entry = json!({
        "timestamp": Utc::now().to_rfc3339(),
        "event": "action",
        "agent": agent,
        "action": action,
        "severity": severity,
        "details": details
    });
    append_jsonl(&log_paths().actions, &entry);
}

/// Evaluate a contract via the **contracts** package and **log** the evaluation.
///
/// # Arguments
/// * `meta` — [`ContractEvalMeta`] describing the contract kind/name/metadata.
/// * `message` — The text to evaluate.
///
/// # Returns
/// * `Ok(Value)` — The exact JSON result produced by the contracts engine.
/// * `Err(String)` — If the request/serialization or evaluation fails.
///
/// # Side effects
/// * Appends a [`ContractEvalRecord`] to `contracts.jsonl` (timestamp, preview, latency, result).
///
/// # Notes
/// - This function builds a minimal envelope `{ kind, rules: [], metadata }`
///   to accommodate flexible backends in the `contracts` crate.
/// - The `message` is redacted to a short preview before logging.
pub fn evaluate_and_audit_contract(
    meta: &ContractEvalMeta,
    message: &str,
) -> Result<Value, String> {
    // Opportunistically restore canonical contracts when locked.
    enforce_contracts_on_disk_if_locked();
    let settings = contracts_settings();
    let path = settings.dir.join(&settings.default_contract);
    let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
    let locked = CONTRACTS_LOCKED.load(Ordering::SeqCst);
    let text = read_verified_or_embedded(&path, file_name, locked).map_err(|e| e.to_string())?;
    let contract: MoralContract = toml::from_str(text.as_ref()).map_err(|e| e.to_string())?;
    let t0 = std::time::Instant::now();
    let result_struct = evaluate_input_against_rules(message, &contract);
    let latency = t0.elapsed().as_secs_f64() * 1000.0;

    let result_json = serde_json::to_value(&result_struct).map_err(|e| e.to_string())?;
    let rec = ContractEvalRecord {
        timestamp: Utc::now(),
        kind: meta.kind.clone(),
        contract_name: meta.contract_name.clone(),
        input_preview: redact_preview(message),
        latency_ms: latency,
        result: result_json.clone(),
        metadata: meta.metadata.clone(),
    };
    if audit_enabled() {
        append_jsonl(&log_paths().contracts, &rec);
    }
    Ok(result_json)
}

/// Log a normalized ethics decision and, if needed, a violation event.
///
/// # Arguments
/// * `intent_category` — Operation label (e.g., `"memory_storage"`).
/// * `passed` — Overall pass/fail.
/// * `risk` — `"Low" | "Medium" | "High" | "Critical"`.
/// * `constraints` — Soft constraints (e.g., `["request_clarification"]`).
/// * `reason` — Human-readable rationale.
///
/// # Side effects
/// * Appends a normalized decision to `ethics.jsonl`.
/// * Appends a violation to `violations.jsonl` when `!passed` or `risk ∈ {High, Critical}`.
///
/// # Returns
/// Nothing.
pub fn record_ethics_decision(
    intent_category: &str,
    passed: bool,
    risk: &str,
    constraints: &[String],
    reason: &str,
) {
    if !audit_enabled() {
        return;
    }
    let entry = serde_json::json!({
        "timestamp": Utc::now().to_rfc3339(),
        "intent_category": intent_category,
        "passed": passed,
        "risk": risk,
        "constraints": constraints,
        "reason": reason,
        "requires_escalation": (!passed) || matches!(risk, "High" | "Critical"),
    });

    // Write to ethics log
    append_jsonl(&log_paths().ethics, &entry);

    // Also write a violation event if escalation-worthy
    if (!passed) || matches!(risk, "High" | "Critical") {
        let viol = serde_json::json!({
            "timestamp": Utc::now().to_rfc3339(),
            "event": "violation",
            "intent_category": intent_category,
            "risk": risk,
            "reason": reason,
            "constraints": constraints,
        });
        append_jsonl(&log_paths().violations, &viol);
    }
}

/// ----------- Helpers -----------

/// Ensure the logbook directory exists (idempotent).
fn ensure_dirs() {
    let path = &log_paths().dir;
    if !path.exists() {
        let _ = fs::create_dir_all(path);
    }
}

/// Append a single JSON value as a line to a JSONL file.
///
/// # Arguments
/// * `path` — Destination path.
/// * `val` — Any `Serialize` value.
///
/// # Returns
/// Nothing. Creates parent directories if missing; ignores write errors to avoid crashing the caller.
fn append_jsonl<P: AsRef<std::path::Path>, S: Serialize>(path: P, val: &S) {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(f, "{}", serde_json::to_string(val).unwrap());
    }
}

/// Produce a privacy-safe preview of an input string for logging.
///
/// # Arguments
/// * `s` — Original (potentially sensitive) input.
///
/// # Returns
/// * A single-line preview: newlines removed, truncated to the configured preview length with an ellipsis.
fn redact_preview(s: &str) -> String {
    let mut t = s.replace('\n', " ");
    let max_len = preview_len();
    if t.len() > max_len {
        t.truncate(max_len);
        t.push('…');
    }
    t
}

fn log_paths() -> &'static LogPaths {
    static CELL: OnceCell<LogPaths> = OnceCell::new();
    CELL.get_or_init(|| match ensure_initialized_once() {
        Ok(report) => LogPaths::from_config(&report.config),
        Err(_) => LogPaths::default(),
    })
}

fn contracts_settings() -> &'static ContractsSettings {
    static CELL: OnceCell<ContractsSettings> = OnceCell::new();
    CELL.get_or_init(|| match ensure_initialized_once() {
        Ok(report) => ContractsSettings::from_config(&report.config),
        Err(_) => ContractsSettings::default(),
    })
}

fn policies() -> &'static PoliciesConfig {
    static CELL: OnceCell<PoliciesConfig> = OnceCell::new();
    CELL.get_or_init(|| match ensure_initialized_once() {
        Ok(report) => report.config.policies.clone(),
        Err(_) => PoliciesConfig::default(),
    })
}

fn audit_enabled() -> bool {
    static CELL: OnceCell<bool> = OnceCell::new();
    *CELL.get_or_init(|| {
        ensure_initialized_once()
            .map(|report| report.config.services.audit_enabled)
            .unwrap_or(true)
    })
}

fn preview_len() -> usize {
    policies().log_preview_len
}

#[derive(Clone)]
struct LogPaths {
    dir: PathBuf,
    ethics: PathBuf,
    actions: PathBuf,
    violations: PathBuf,
    contracts: PathBuf,
}

impl LogPaths {
    fn from_config(cfg: &CoreConfig) -> Self {
        Self {
            dir: cfg.logbook.path.clone(),
            ethics: cfg.logbook.ethics_log.clone(),
            actions: cfg.logbook.agent_actions.clone(),
            violations: cfg.logbook.contract_violations.clone(),
            contracts: cfg.logbook.contracts_log.clone(),
        }
    }
}

impl Default for LogPaths {
    fn default() -> Self {
        let cfg = CoreConfig::default();
        Self::from_config(&cfg)
    }
}

#[derive(Clone)]
struct ContractsSettings {
    dir: PathBuf,
    default_contract: String,
}

impl ContractsSettings {
    fn from_config(cfg: &CoreConfig) -> Self {
        Self {
            dir: cfg.contracts.path.clone(),
            default_contract: cfg.contracts.default_contract.clone(),
        }
    }
}

impl Default for ContractsSettings {
    fn default() -> Self {
        let cfg = CoreConfig::default();
        Self::from_config(&cfg)
    }
}
