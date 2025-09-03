// synaptik-core/src/commands/init.rs

use anyhow::{Context, Result};
use chrono::Utc;
use once_cell::sync::OnceCell;              
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use contracts::assets::write_default_contracts;

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
    ensure_dir(&root, "contracts", &mut created, &mut existed)?;
    // Ensure objects/ exists for LobeStore default path
    ensure_dir(&root, "objects", &mut created, &mut existed)?;
    ensure_dir(&root, "cache", &mut created, &mut existed)?;
    ensure_dir(&root, "dag", &mut created, &mut existed)?;
    ensure_dir(&root, "archive", &mut created, &mut existed)?;
    ensure_dir(&root.join("archive"), "objects", &mut created, &mut existed)?;
    ensure_dir(&root, "logbook", &mut created, &mut existed)?;
    // Optional: endpoints registry lives under services/
    ensure_dir(&root, "services", &mut created, &mut existed)?;

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

    // Contract dir
    ensure_dir(&root, "contracts", &mut created, &mut existed)?;

    // Seed default contracts from the contracts crate (idempotent)
    let _ = write_default_contracts(root.join("contracts"));

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

const DEFAULT_CONFIG_TOML: &str = r#"
[system]
name = "cogniv"
version = "0.1.0"

[memory]
cache_path = ".cogniv/cache/memory.db"
dag_path   = ".cogniv/dag"

[logbook]
path               = ".cogniv/logbook"
aggregate          = ".cogniv/logbook.jsonl"
ethics_log         = ".cogniv/logbook/ethics.jsonl"
agent_actions      = ".cogniv/logbook/actions.jsonl"
contract_violations = ".cogniv/logbook/violations.jsonl"

[services]
ethos_enabled     = true
librarian_enabled = true
audit_enabled     = true

[contracts]
path            = ".cogniv/contracts"
default_contract = "nonviolence.toml"

[cache]
max_hot_memory_mb = 50

[audit]
retention_days = 365
"#;
