// synaptik-core/src/commands/init.rs

use anyhow::{Context, Result};
use chrono::Utc;
use once_cell::sync::OnceCell;              // <-- use once_cell only (no std::OnceLock)
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct InitReport {
    pub root: PathBuf,
    pub created: Vec<String>,
    pub existed: Vec<String>,
}

// ---------- single global init gate ----------

static INIT: OnceCell<InitReport> = OnceCell::new();

/// Idempotent global initializer. Safe to call often.
/// Returns a &'static InitReport once initialization has completed.
pub fn ensure_initialized_once() -> Result<&'static InitReport> {
    // get_or_try_init takes a closure returning Result<InitReport>
    INIT.get_or_try_init(|| ensure_initialized())
}

/// Resolve the Cogniv root. Allow override via COGNIV_ROOT (tests/venvs).
fn cogniv_root() -> PathBuf {
    std::env::var_os("COGNIV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(".cogniv"))
}

/// Public API: ensure initialized (idempotent). Safe to call often.
pub fn ensure_initialized() -> Result<InitReport> {
    let root = cogniv_root();
    let mut created = Vec::new();
    let mut existed = Vec::new();

    // Directories
    ensure_dir(&root, "", &mut created, &mut existed)?;
    ensure_dir(&root, "objects", &mut created, &mut existed)?;
    ensure_dir(&root, "refs", &mut created, &mut existed)?;
    ensure_dir(&root, "contracts", &mut created, &mut existed)?;
    ensure_dir(&root, "cache", &mut created, &mut existed)?;
    ensure_dir(&root, "dag", &mut created, &mut existed)?;
    ensure_dir(&root, "archive", &mut created, &mut existed)?;
    ensure_dir(&root.join("archive"), "objects", &mut created, &mut existed)?;
    ensure_dir(&root, "logbook", &mut created, &mut existed)?;

    // HEAD ref (Git-like)
    ensure_file(
        &root,
        "HEAD",
        Some("ref: refs/heads/main\n"),
        &mut created,
        &mut existed,
    )?;

    // Config
    ensure_file(
        &root,
        "config.toml",
        Some(DEFAULT_CONFIG_TOML),
        &mut created,
        &mut existed,
    )?;

    // Base contract
    ensure_file(
        &root.join("contracts"),
        "base_ethics.toml",
        Some(BASE_ETHICS_TOML),
        &mut created,
        &mut existed,
    )?;

    // Logbook schema (per-stream JSONL files)
    initialize_logbook_files(&root, &mut created, &mut existed)?;

    Ok(InitReport { root, created, existed })
}

fn ensure_dir(
    base: &Path,
    rel: &str,
    created: &mut Vec<String>,
    existed: &mut Vec<String>,
) -> Result<()> {
    let p = if rel.is_empty() { base.to_path_buf() } else { base.join(rel) };
    if p.exists() {
        existed.push(if rel.is_empty() { ".".to_string() } else { rel.to_string() });
        return Ok(());
    }
    fs::create_dir_all(&p).with_context(|| format!("create_dir_all({:?})", p))?;
    created.push(if rel.is_empty() { ".".to_string() } else { rel.to_string() });
    Ok(())
}

fn ensure_file(
    base: &Path,
    rel_file: &str,
    content_if_absent: Option<&str>,
    created: &mut Vec<String>,
    existed: &mut Vec<String>,
) -> Result<()> {
    let p = base.join(rel_file);
    if p.exists() {
        existed.push(rel_file.to_string());
        return Ok(());
    }
    if let Some(text) = content_if_absent {
        write_atomic(&p, text.as_bytes())?;
    } else {
        write_atomic(&p, b"")?;
    }
    created.push(rel_file.to_string());
    Ok(())
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create_dir_all({:?})", parent))?;
    }
    let tmp = path.with_extension("tmp");
    {
        let mut f = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&tmp)
            .with_context(|| format!("open temp file {:?}", tmp))?;
        f.write_all(bytes)?;
        f.flush()?;
    }
    fs::rename(&tmp, path).with_context(|| format!("rename {:?} -> {:?}", tmp, path))?;
    Ok(())
}

fn ensure_seeded_jsonl(
    dir: &Path,
    file: &str,
    init_line: &str,
    created: &mut Vec<String>,
    existed: &mut Vec<String>,
) -> Result<()> {
    let p = dir.join(file);
    if !p.exists() {
        ensure_file(dir, file, Some(&(init_line.to_string() + "\n")), created, existed)?;
        return Ok(());
    }
    existed.push(file.to_string());
    // If exists but empty, seed it
    if fs::metadata(&p)?.len() == 0 {
        let mut f = OpenOptions::new().append(true).open(&p)?;
        f.write_all(init_line.as_bytes())?;
        f.write_all(b"\n")?;
    }
    Ok(())
}

fn initialize_logbook_files(
    root: &Path,
    created: &mut Vec<String>,
    existed: &mut Vec<String>,
) -> Result<()> {
    let ts = Utc::now().to_rfc3339();
    let init_event = format!(
        r#"{{"ts":"{}","event":"system_init","agent":"system","data":{{"version":"1.0.0","architecture":"hybrid_tiered"}}}}"#,
        ts
    );

    // aggregate
    ensure_seeded_jsonl(root, "logbook.jsonl", &init_event, created, existed)?;

    // per-stream
    let log_dir = root.join("logbook");
    ensure_dir(root, "logbook", created, existed)?;
    ensure_seeded_jsonl(&log_dir, "ethics.jsonl", &init_event, created, existed)?;
    ensure_seeded_jsonl(&log_dir, "actions.jsonl", &init_event, created, existed)?;
    ensure_seeded_jsonl(&log_dir, "violations.jsonl", &init_event, created, existed)?;
    Ok(())
}

// ---------- defaults ----------

const DEFAULT_CONFIG_TOML: &str = r#"[system]
name = "cogniv"
version = "0.1.0"
architecture = "hybrid_tiered"

[memory]
objects_path = ".cogniv/objects"
cache_path = ".cogniv/cache/memory.db"
dag_path = ".cogniv/dag"

[logbook]
path = ".cogniv/logbook"
aggregate = ".cogniv/logbook.jsonl"
ethics_log = ".cogniv/logbook/ethics.jsonl"
agent_actions = ".cogniv/logbook/actions.jsonl"
contract_violations = ".cogniv/logbook/violations.jsonl"

[services]
ethos_enabled = true
librarian_enabled = true
audit_enabled = true
sqlite_cache_enabled = true
dag_consensus_enabled = false

[endpoints]
registry = ".cogniv/services/registry.json"

[contracts]
path = ".cogniv/contracts"
default_contract = "base_ethics.toml"
validation_required = true
audit_all_decisions = true

[cache]
max_hot_memory_mb = 50
warm_memory_retention_hours = 24
enable_embeddings = true

[demo]
single_workspace = true
ttl_hours = 24
rate_limit_per_min = 30

[librarian.prune]
enable = true
keep_newest = 500
on_commit = true
hot_budget_mb = 100

[audit]
log_all_agent_actions = true
log_memory_operations = true
log_contract_evaluations = true
retention_days = 365
enable_real_time_monitoring = true
"#;

const BASE_ETHICS_TOML: &str = r#"# Base Ethical Contract v1.0
[contract]
name = "base_ethics"
version = "1.0.0"
description = "Foundational ethical guardrails"
audit_level = "full"

[rules]
no_harm = true
respect_privacy = true
truthfulness_required = true
user_consent_required = true
transparency_required = true

[memory]
consent_required = true
retention_limits = true
deletion_rights = true

[validation]
before_memory_storage = true
before_external_action = true
on_sensitive_topics = true
on_personal_information = true
before_decision_making = true

[audit]
log_all_decisions = true
require_justification = true
enable_trace_back = true
store_in_logbook = true

[consequences]
warn_user = true
block_action = true
escalate_to_human = true
log_violation = true
"#;
