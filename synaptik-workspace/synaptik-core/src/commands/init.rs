// synaptik-core/src/commands/init.rs

use anyhow::{Context, Result};
use chrono::Utc;
use once_cell::sync::OnceCell;
use serde_json::json;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use contracts::assets::write_default_contracts;

use crate::config::{CoreConfig, LogbookConfig};

#[derive(Debug, Clone)]
pub struct InitReport {
    pub root: PathBuf,
    pub created: Vec<String>,
    pub existed: Vec<String>,
    pub config: CoreConfig,
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
    ensure_dir(&root, "refs", &mut created, &mut existed)?;

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

    // Load configuration (relative paths are resolved against root)
    let config = CoreConfig::load(&root)?;

    // Derived directories from config
    ensure_parent_dir_abs(&config.memory.cache_path, &mut created, &mut existed)?;
    ensure_dir_abs(&config.memory.dag_path, &mut created, &mut existed)?;
    ensure_dir_abs(&config.memory.archive_path, &mut created, &mut existed)?;
    ensure_dir_abs(
        &config.memory.archive_path.join("objects"),
        &mut created,
        &mut existed,
    )?;
    ensure_dir_abs(&config.contracts.path, &mut created, &mut existed)?;

    // Seed default contracts from the contracts crate (idempotent)
    let _ = write_default_contracts(&config.contracts.path);

    // Logbook schema (per-stream JSONL files)
    initialize_logbook_files(&config.logbook, &mut created, &mut existed)?;

    Ok(InitReport {
        root,
        created,
        existed,
        config,
    })
}

fn ensure_dir(
    base: &Path,
    rel: &str,
    created: &mut Vec<String>,
    existed: &mut Vec<String>,
) -> Result<()> {
    let p = if rel.is_empty() {
        base.to_path_buf()
    } else {
        base.join(rel)
    };
    if p.exists() {
        existed.push(if rel.is_empty() {
            ".".to_string()
        } else {
            rel.to_string()
        });
        return Ok(());
    }
    fs::create_dir_all(&p).with_context(|| format!("create_dir_all({:?})", p))?;
    created.push(if rel.is_empty() {
        ".".to_string()
    } else {
        rel.to_string()
    });
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

fn initialize_logbook_files(
    log_cfg: &LogbookConfig,
    created: &mut Vec<String>,
    existed: &mut Vec<String>,
) -> Result<()> {
    let ts = Utc::now().to_rfc3339();
    let init_event = json!({
        "ts": ts,
        "event": "system_init",
        "agent": "system",
        "data": {
            "version": "1.0.0",
            "architecture": "hybrid_tiered"
        }
    })
    .to_string();

    ensure_dir_abs(&log_cfg.path, created, existed)?;
    ensure_seeded_jsonl_abs(&log_cfg.aggregate, &init_event, created, existed)?;
    ensure_seeded_jsonl_abs(&log_cfg.ethics_log, &init_event, created, existed)?;
    ensure_seeded_jsonl_abs(&log_cfg.agent_actions, &init_event, created, existed)?;
    ensure_seeded_jsonl_abs(&log_cfg.contract_violations, &init_event, created, existed)?;
    ensure_seeded_jsonl_abs(&log_cfg.contracts_log, &init_event, created, existed)?;
    Ok(())
}

fn ensure_dir_abs(path: &Path, created: &mut Vec<String>, existed: &mut Vec<String>) -> Result<()> {
    if path.exists() {
        existed.push(path.display().to_string());
        return Ok(());
    }
    fs::create_dir_all(path).with_context(|| format!("create_dir_all({:?})", path))?;
    created.push(path.display().to_string());
    Ok(())
}

fn ensure_parent_dir_abs(
    path: &Path,
    created: &mut Vec<String>,
    existed: &mut Vec<String>,
) -> Result<()> {
    if let Some(parent) = path.parent() {
        ensure_dir_abs(parent, created, existed)
    } else {
        Ok(())
    }
}

fn ensure_seeded_jsonl_abs(
    path: &Path,
    init_line: &str,
    created: &mut Vec<String>,
    existed: &mut Vec<String>,
) -> Result<()> {
    if path.exists() {
        existed.push(path.display().to_string());
        if fs::metadata(path)?.len() == 0 {
            let mut f = OpenOptions::new()
                .append(true)
                .open(path)
                .with_context(|| format!("Failed to open {:?} for appending", path))?;
            f.write_all(init_line.as_bytes())?;
            f.write_all(b"\n")?;
            f.flush()?;
        }
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create_dir_all({:?})", parent))?;
    }
    write_atomic(path, format!("{}\n", init_line).as_bytes())?;
    created.push(path.display().to_string());
    Ok(())
}

// ---------- defaults ----------

const DEFAULT_CONFIG_TOML: &str = r#"
[system]
name = "cogniv"
version = "0.1.0"

[memory]
cache_path = "cache/memory.db"
dag_path   = "dag"
archive_path = "archive"

[logbook]
path               = "logbook"
aggregate          = "logbook.jsonl"
ethics_log         = "logbook/ethics.jsonl"
agent_actions      = "logbook/actions.jsonl"
contract_violations = "logbook/violations.jsonl"
contracts_log      = "logbook/contracts.jsonl"

[services]
ethos_enabled     = true
librarian_enabled = true
audit_enabled     = true

[contracts]
path            = "contracts"
default_contract = "nonviolence.toml"

[cache]
max_hot_memory_mb = 50

[audit]
retention_days = 365

[policies]
promote_hot_threshold = 5
auto_prune_duplicates = true
reflection_min_count = 3
reflection_max_keywords = 3
reflection_pool_size = 20
summary_min_len = 500
log_preview_len = 160
"#;
