// src/utils/logbook.rs
use anyhow::Result;
use serde::Serialize;
use serde_json::Value;
use std::{fs, io::Write, path::Path};

#[derive(Serialize)]
struct LogLine<'a> {
    id: String,
    ts: String,
    event: &'a str,
    content_preview: String,
}

pub fn append_log(
    base: &Path,
    id: &str,
    ts_rfc3339: &str,
    event: &str,
    content: &str,
) -> Result<()> {
    let log_path = base.join("logbook.jsonl");
    let preview = content.chars().take(120).collect::<String>();
    let line = LogLine {
        id: id.to_string(),
        ts: ts_rfc3339.to_string(),
        event,
        content_preview: preview,
    };
    let json = serde_json::to_string(&line)?;
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)?;
    writeln!(f, "{}", json)?;
    Ok(())
}

pub fn emit_event(base: &Path, event: &str, data: Value, ts: &str) -> Result<()> {
    let log_path = base.join("logbook.jsonl");
    let line = serde_json::json!({
        "timestamp": ts,
        "event": event,
        "data": data
    });
    let json = serde_json::to_string(&line)?;
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)?;
    writeln!(f, "{}", json)?;
    Ok(())
}
