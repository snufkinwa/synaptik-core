// src/memory/dag.rs
//! MVP DAG (super simple):
//! - One folder: <COGNIV_ROOT>/dag/nodes
//! - One ref folder: <COGNIV_ROOT>/refs/streams
//! - On every save:
//!    • stream = (lobe, key) from meta
//!    • compute h = blake3(content_utf8 bytes)
//!    • if last_hash(stream) == h -> NO new node (return latest node id)
//!    • else write a node file with parent = latest(stream), update stream ref
//! - Parent is stored inline; no edge files.

use anyhow::{anyhow, Context, Result};
use blake3;
use serde_json::Value;
use std::{fs, io::Write, path::{Path, PathBuf}};

use crate::commands::init::ensure_initialized_once;

// ---------- paths ----------

fn dag_nodes_dir() -> Result<PathBuf> {
    let p = ensure_initialized_once()?.root.join("dag").join("nodes");
    fs::create_dir_all(&p)?;
    Ok(p)
}

fn stream_refs_dir() -> Result<PathBuf> {
    let p = ensure_initialized_once()?.root.join("refs").join("streams");
    fs::create_dir_all(&p)?;
    Ok(p)
}

fn ids_ref_dir() -> Result<PathBuf> {
    let p = ensure_initialized_once()?.root.join("refs").join("ids");
    fs::create_dir_all(&p)?;
    Ok(p)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create_dir_all({:?})", parent))?;
    }
    let tmp = path.with_extension("tmp");
    {
        let mut f = fs::File::create(&tmp)
            .with_context(|| format!("open temp file {:?}", tmp))?;
        f.write_all(bytes)?;
        f.flush()?;
    }
    fs::rename(&tmp, path)
        .with_context(|| format!("rename {:?} -> {:?}", tmp, path))?;
    Ok(())
}

// ---------- tiny stream refs ----------

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
struct StreamRef {
    latest_node: Option<String>,
    last_hash: Option<String>,
    updated_at: Option<String>,
}

fn stream_key(lobe: &str, key: &str) -> String {
    format!("{}__{}", sanitize(lobe), sanitize(key))
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

fn read_stream_ref(lobe: &str, key: &str) -> Result<StreamRef> {
    let p = stream_refs_dir()?.join(format!("{}.json", stream_key(lobe, key)));
    if !p.exists() {
        return Ok(StreamRef::default());
    }
    let bytes = fs::read(&p)?;
    let v = serde_json::from_slice::<StreamRef>(&bytes).unwrap_or_default();
    Ok(v)
}

fn write_stream_ref(lobe: &str, key: &str, r: &StreamRef) -> Result<()> {
    let p = stream_refs_dir()?.join(format!("{}.json", stream_key(lobe, key)));
    write_atomic(&p, &serde_json::to_vec_pretty(r)?)?;
    Ok(())
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct IdIndex {
    node: String,
    lobe: String,
    key: String,
}

fn write_id_index(id: &str, node: &str, lobe: &str, key: &str) -> Result<()> {
    let p = ids_ref_dir()?.join(format!("{}.json", sanitize(id)));
    let idx = IdIndex { node: node.to_string(), lobe: lobe.to_string(), key: key.to_string() };
    write_atomic(&p, &serde_json::to_vec_pretty(&idx)?)
}

fn read_id_index(id: &str) -> Result<Option<IdIndex>> {
    let p = ids_ref_dir()?.join(format!("{}.json", sanitize(id)));
    if !p.exists() { return Ok(None); }
    let bytes = fs::read(&p)?;
    let idx: IdIndex = serde_json::from_slice(&bytes).unwrap_or_else(|_| IdIndex { node: String::new(), lobe: String::new(), key: String::new() });
    if idx.node.is_empty() { return Ok(None); }
    Ok(Some(idx))
}

// ---------- public API (used by Memory) ----------

/// Save a node for (lobe,key) stream if content changed. Returns the node file name.
pub fn save_node(
    id: &str,
    content_utf8: &str,
    meta: &serde_json::Value,
    parents: &[String],
) -> anyhow::Result<String> {
    let lobe = meta.get("lobe").and_then(|v| v.as_str()).unwrap_or("unknown");
    let key  = meta.get("key").and_then(|v| v.as_str()).unwrap_or("default");

    let h = blake3::hash(content_utf8.as_bytes()).to_hex().to_string();

    // load last state for (lobe,key)
    let mut sref = read_stream_ref(lobe, key)?;
    if sref.last_hash.as_deref() == Some(&h) {
        if let Some(latest) = sref.latest_node.clone() {
            return Ok(latest); // idempotent: nothing to write
        }
        // else: no latest yet — fall through and write one
    }

    let ts = chrono::Utc::now().to_rfc3339();
    let fname = format!("{}__{}.json", ts.replace(':', "-"), sanitize(id));
    let node_path = dag_nodes_dir()?.join(&fname);

    let parent = if !parents.is_empty() {
        Some(parents[0].clone())
    } else {
        sref.latest_node.clone()
    };

    let node = serde_json::json!({
        "id": id,
        "ts": ts,
        "lobe": lobe,
        "key": key,
        "parent": parent,
        "hash": h,
        "content": content_utf8,
        "meta": {
            "lobe":         meta.get("lobe").cloned().unwrap_or(serde_json::json!(lobe)),
            "key":          meta.get("key").cloned().unwrap_or(serde_json::json!(key)),
            "version_id":   meta.get("version_id").cloned().unwrap_or(serde_json::json!(null)),
            "etag":         meta.get("etag").cloned().unwrap_or(serde_json::json!(null)),
            "content_type": meta.get("content_type").cloned().unwrap_or(serde_json::json!(null)),
            "created_at":   meta.get("created_at").cloned().unwrap_or(serde_json::json!(ts)),
            "updated_at":   serde_json::json!(ts),
            "cid":          meta.get("cid").cloned().unwrap_or(serde_json::json!(h)),
            "summary_len":  meta.pointer("/summary")
                                .and_then(|v| v.as_str())
                                .map(|s| s.len()).unwrap_or(0),
        },
    });

    write_atomic(&node_path, &serde_json::to_vec_pretty(&node)?)?;

    sref.latest_node = Some(fname.clone());
    sref.last_hash = Some(h);
    sref.updated_at = Some(ts);
    write_stream_ref(lobe, key, &sref)?;

    // Maintain a quick id -> node index to avoid directory scans.
    let _ = write_id_index(id, &fname, lobe, key);

    Ok(fname)
}

/// Load a node by its filename (as returned by save_node).
pub fn load_node(filename: &str) -> Result<Value> {
    let p = dag_nodes_dir()?.join(filename);
    let bytes = fs::read(&p).map_err(|_| anyhow!("node not found: {}", filename))?;
    Ok(serde_json::from_slice(&bytes)?)
}

/// Load a node by original memory id using the id index.
pub fn load_node_by_id(id: &str) -> Result<Option<Value>> {
    if let Some(idx) = read_id_index(id)? {
        let v = load_node(&idx.node)?;
        return Ok(Some(v));
    }
    Ok(None)
}

/// Return content string from a node by original memory id.
pub fn content_by_id(id: &str) -> Result<Option<String>> {
    if let Some(v) = load_node_by_id(id)? {
        if let Some(s) = v.get("content").and_then(|x| x.as_str()) {
            return Ok(Some(s.to_string()));
        }
    }
    Ok(None)
}

/// Return the child (next) nodes of a given node *within the same stream* by scanning.
// MVP: linear scan; fine for small graphs.
pub fn children_of(filename: &str) -> Result<Vec<String>> {
    let dir = dag_nodes_dir()?;
    let mut kids = Vec::new();
    for e in fs::read_dir(&dir)? {
        let path = e?.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") { continue; }
        let bytes = fs::read(&path)?;
        if let Ok(v) = serde_json::from_slice::<Value>(&bytes) {
            if v.get("parent").and_then(|x| x.as_str()) == Some(filename) {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    kids.push(name.to_string());
                }
            }
        }
    }
    Ok(kids)
}

// ---------- tiny DAG pruner (MVP) ----------

#[derive(Debug, Clone)]
pub struct PruneReport {
    pub examined: usize,
    pub kept: usize,
    pub removed: usize,
}

/// Keep only the newest `keep_last_per_stream` nodes per (lobe,key).
pub fn prune(keep_last_per_stream: usize) -> Result<PruneReport> {
    let dir = dag_nodes_dir()?;
    let mut by_stream: std::collections::BTreeMap<(String, String), Vec<(String, String)>> = Default::default();
    // collect: (lobe,key) -> [(ts, filename)]
    for e in fs::read_dir(&dir)? {
        let path = e?.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") { continue; }
        let bytes = fs::read(&path)?;
        let v: Value = match serde_json::from_slice(&bytes) { Ok(v) => v, Err(_) => continue };
        let lobe = v.get("lobe").and_then(|x| x.as_str()).unwrap_or("unknown").to_string();
        let key  = v.get("key").and_then(|x| x.as_str()).unwrap_or("default").to_string();
        let ts   = v.get("ts").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string();
        by_stream.entry((lobe, key)).or_default().push((ts, name));
    }

    let mut examined = 0usize;
    let mut removed = 0usize;

    for ((_lobe, _key), mut nodes) in by_stream {
        // newest first by timestamp string (RFC3339 sorts fine lexicographically if we replaced ':' above)
        nodes.sort_by(|a, b| b.0.cmp(&a.0));
        examined += nodes.len();
        if nodes.len() > keep_last_per_stream {
            for (_ts, name) in nodes.into_iter().skip(keep_last_per_stream) {
                let p = dir.join(name);
                let _ = fs::remove_file(p);
                removed += 1;
            }
        }
    }

    Ok(PruneReport {
        examined,
        kept: examined.saturating_sub(removed),
        removed,
    })
}
