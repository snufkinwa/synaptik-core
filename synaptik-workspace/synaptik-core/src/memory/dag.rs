// src/memory/dag.rs
//! MVP DAG (super simple):
//! - One folder: <COGNIV_ROOT>/dag/nodes
//! - One ref folder: <COGNIV_ROOT>/refs/streams
//! - On every save:
//!    • stream = (lobe, key) from meta
//!    • compute h = blake3(content_utf8 bytes)
//!    • if last_hash(stream) == h -> NO new node (return latest node id)
//!    • else write a node file with parents = [latest(stream), ...], update stream ref
//! - Parents are stored inline as an ordered array (primary first). For backward
//!   compatibility, older nodes may contain a single `parent` string; readers map it
//!   to `parents = [parent]`.

use anyhow::{Context, Result, anyhow};
use blake3;
use serde_json::Value;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use crate::commands::init::ensure_initialized_once;
use crate::utils::path as pathutil;

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

fn hashes_ref_dir() -> Result<PathBuf> {
    let p = ensure_initialized_once()?.root.join("refs").join("hashes");
    fs::create_dir_all(&p)?;
    Ok(p)
}

fn children_ref_dir() -> Result<PathBuf> {
    let p = ensure_initialized_once()?
        .root
        .join("refs")
        .join("children");
    fs::create_dir_all(&p)?;
    Ok(p)
}

fn paths_ref_dir() -> Result<PathBuf> {
    let p = ensure_initialized_once()?.root.join("refs").join("paths");
    fs::create_dir_all(&p)?;
    Ok(p)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    // Enforce that DAG writes stay within the initialized root.
    let root = ensure_initialized_once()?.root.clone();
    let root = root.canonicalize().unwrap_or(root);
    let _ = pathutil::assert_within_root_abs(&root, path)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create_dir_all({:?})", parent))?;
    }
    let tmp = path.with_extension("tmp");
    {
        let mut f = fs::File::create(&tmp).with_context(|| format!("open temp file {:?}", tmp))?;
        f.write_all(bytes)?;
        f.flush()?;
    }
    fs::rename(&tmp, path).with_context(|| format!("rename {:?} -> {:?}", tmp, path))?;
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
    let idx = IdIndex {
        node: node.to_string(),
        lobe: lobe.to_string(),
        key: key.to_string(),
    };
    write_atomic(&p, &serde_json::to_vec_pretty(&idx)?)
}

fn read_id_index(id: &str) -> Result<Option<IdIndex>> {
    let p = ids_ref_dir()?.join(format!("{}.json", sanitize(id)));
    if !p.exists() {
        return Ok(None);
    }
    let bytes = fs::read(&p)?;
    let idx: IdIndex = serde_json::from_slice(&bytes).unwrap_or_else(|_| IdIndex {
        node: String::new(),
        lobe: String::new(),
        key: String::new(),
    });
    if idx.node.is_empty() {
        return Ok(None);
    }
    Ok(Some(idx))
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct HashIndex {
    node: String,
}

fn write_hash_index(hash: &str, node: &str) -> Result<()> {
    let p = hashes_ref_dir()?.join(format!("{}.json", sanitize(hash)));
    let idx = HashIndex {
        node: node.to_string(),
    };
    write_atomic(&p, &serde_json::to_vec_pretty(&idx)?)
}

fn read_hash_index(hash: &str) -> Result<Option<HashIndex>> {
    let p = hashes_ref_dir()?.join(format!("{}.json", sanitize(hash)));
    if !p.exists() {
        return Ok(None);
    }
    let bytes = fs::read(&p)?;
    let idx: HashIndex = serde_json::from_slice(&bytes).unwrap_or_else(|_| HashIndex {
        node: String::new(),
    });
    if idx.node.is_empty() {
        return Ok(None);
    }
    Ok(Some(idx))
}

// ---------- parents helpers & optional reverse index ----------

fn read_children_index(hash: &str) -> Result<Vec<String>> {
    let p = children_ref_dir()?.join(format!("{}.json", sanitize(hash)));
    if !p.exists() {
        return Ok(Vec::new());
    }
    let bytes = fs::read(&p)?;
    let arr: Vec<String> = serde_json::from_slice(&bytes).unwrap_or_default();
    Ok(arr)
}

fn write_children_index(hash: &str, children: &[String]) -> Result<()> {
    let p = children_ref_dir()?.join(format!("{}.json", sanitize(hash)));
    write_atomic(&p, &serde_json::to_vec_pretty(children)?)
}

fn append_child_to_parent(hash: &str, child_node: &str) -> Result<()> {
    let mut arr = read_children_index(hash)?;
    if !arr.iter().any(|s| s == child_node) {
        arr.push(child_node.to_string());
        arr.sort();
        write_children_index(hash, &arr)?;
    }
    Ok(())
}

// Back-compat helper: extract ordered parents (filenames or hashes).
fn node_parents_list(v: &Value) -> Vec<String> {
    if let Some(arr) = v.get("parents").and_then(|x| x.as_array()) {
        let mut out = Vec::new();
        for it in arr {
            if let Some(s) = it.as_str() {
                if !s.is_empty() {
                    out.push(s.to_string());
                }
            }
        }
        return out;
    }
    if let Some(p) = v.get("parent").and_then(|x| x.as_str()) {
        if !p.is_empty() {
            return vec![p.to_string()];
        }
    }
    Vec::new()
}

fn parent_filenames_from_node(v: &Value) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for p in node_parents_list(v) {
        if p.ends_with(".json") {
            out.push(p);
        } else if let Some(fname) = resolve_parent_filename(&p).ok().flatten() {
            out.push(fname);
        }
    }
    out
}

// Fallback parent filename resolver: first consult hash index; if missing, scan dag nodes directory
// for a JSON file whose internal "hash" matches the requested parent hash. Returns filename if found.
fn resolve_parent_filename(parent_hash: &str) -> Result<Option<String>> {
    if let Some(idx) = read_hash_index(parent_hash)? {
        return Ok(Some(idx.node));
    }
    let dir = match dag_nodes_dir() {
        Ok(d) => d,
        Err(_) => return Ok(None),
    };
    let entries = match fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return Ok(None),
    };
    for ent in entries.flatten() {
        let p = ent.path();
        if p.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        if let Some(fname) = p.file_name().and_then(|n| n.to_str()) {
            match load_node(&fname.to_string()) {
                Ok(v) => {
                    if let Some(h) = v.get("hash").and_then(|x| x.as_str()) {
                        if h == parent_hash {
                            return Ok(Some(fname.to_string()));
                        }
                    }
                }
                Err(_) => continue,
            }
        }
    }
    Ok(None)
}

// ---------- public API (used by Memory) ----------

/// Save a node for (lobe,key) stream if content changed. Returns the node file name.
pub fn save_node(
    id: &str,
    content_utf8: &str,
    meta: &serde_json::Value,
    parents: &[String],
) -> anyhow::Result<String> {
    let lobe = meta
        .get("lobe")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let key = meta
        .get("key")
        .and_then(|v| v.as_str())
        .unwrap_or("default");

    let h = blake3::hash(content_utf8.as_bytes()).to_hex().to_string();

    // load last state for (lobe,key)
    let mut sref = read_stream_ref(lobe, key)?;
    if sref.last_hash.as_deref() == Some(&h) {
        if let Some(latest) = sref.latest_node.clone() {
            // Even if we don't write a new node, ensure this id is indexed to the latest node
            let _ = write_id_index(id, &latest, lobe, key);
            return Ok(latest); // idempotent: nothing to write
        }
        // else: no latest yet — fall through and write one
    }

    let ts = chrono::Utc::now().to_rfc3339();
    let fname = format!("{}__{}.json", ts.replace(':', "-"), sanitize(id));
    let node_path = dag_nodes_dir()?.join(&fname);

    // Determine ordered parents list (primary first). If none provided, default to current head.
    let parent_list: Vec<String> = if !parents.is_empty() {
        parents.to_vec()
    } else {
        sref.latest_node.clone().into_iter().collect()
    };

    // Merge provided meta (if object) with our required fields. Always update updated_at and cid/hash.
    let mut meta_obj: serde_json::Map<String, Value> = match meta.clone() {
        Value::Object(m) => m,
        _ => serde_json::Map::new(),
    };
    if !meta_obj.contains_key("lobe") {
        meta_obj.insert("lobe".into(), Value::String(lobe.to_string()));
    }
    if !meta_obj.contains_key("key") {
        meta_obj.insert("key".into(), Value::String(key.to_string()));
    }
    if !meta_obj.contains_key("created_at") {
        meta_obj.insert("created_at".into(), Value::String(ts.clone()));
    }
    // Always set these
    meta_obj.insert("updated_at".into(), Value::String(ts.clone()));
    meta_obj.insert("cid".into(), Value::String(h.clone()));
    let summary_len = meta
        .pointer("/summary")
        .and_then(|v| v.as_str())
        .map(|s| s.len())
        .unwrap_or(0);
    meta_obj.insert("summary_len".into(), serde_json::json!(summary_len));

    let node = serde_json::json!({
        "id": id,
        "ts": ts,
        "lobe": lobe,
        "key": key,
        "parents": parent_list,
        "hash": h,
        "content": content_utf8,
        "meta": Value::Object(meta_obj),
    });

    write_atomic(&node_path, &serde_json::to_vec_pretty(&node)?)?;

    sref.latest_node = Some(fname.clone());
    sref.last_hash = Some(h.clone());
    sref.updated_at = Some(ts);
    write_stream_ref(lobe, key, &sref)?;

    // Maintain quick indexes to avoid directory scans.
    let _ = write_id_index(id, &fname, lobe, key);
    let _ = write_hash_index(&h, &fname);

    // Update reverse index: record this node as a child of each parent (by parent hash).
    for pf in parent_filenames_from_node(&node) {
        if let Ok(pnode) = load_node(&pf) {
            if let Some(ph) = pnode.get("hash").and_then(|x| x.as_str()) {
                let _ = append_child_to_parent(ph, &fname);
            }
        }
    }

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

/// Reindex a memory id to the latest node of a given (lobe, key) stream.
/// Returns true if an index was written, false if no latest node exists yet.
pub fn reindex_id_to_latest(id: &str, lobe: &str, key: &str) -> Result<bool> {
    let sref = read_stream_ref(lobe, key)?;
    if let Some(latest) = sref.latest_node {
        let _ = write_id_index(id, &latest, lobe, key);
        return Ok(true);
    }
    Ok(false)
}

// ---------- simple content search (newest-first) ----------

/// Search DAG nodes for content containing all words (case-insensitive), newest-first.
/// Returns a list of minimal dicts: [{"hash", "id", "ts"}]
pub fn search_content_words(words: &[String], limit: usize) -> Result<Vec<Value>> {
    let dir = dag_nodes_dir()?;
    let mut names: Vec<String> = Vec::new();
    for e in fs::read_dir(&dir)? {
        let path = e?.path();
        if path.extension().and_then(|s| s.to_str()) == Some("json") {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                names.push(name.to_string());
            }
        }
    }
    // newest-first by filename (timestamp prefix, lexicographically sortable)
    names.sort();
    names.reverse();

    let words_lower: Vec<String> = words.iter().map(|w| w.to_lowercase()).collect();
    let mut out: Vec<Value> = Vec::new();
    for name in names {
        let v = load_node(&name)?;
        let content = v.get("content").and_then(|x| x.as_str()).unwrap_or("");
        let lc = content.to_lowercase();
        let mut ok = true;
        for w in &words_lower {
            if !lc.contains(w) {
                ok = false;
                break;
            }
        }
        if ok {
            let hash = v.get("hash").and_then(|x| x.as_str()).unwrap_or("");
            let id = v.get("id").and_then(|x| x.as_str()).unwrap_or("");
            let ts = v.get("ts").and_then(|x| x.as_str()).unwrap_or("");
            out.push(serde_json::json!({ "hash": hash, "id": id, "ts": ts }));
            if out.len() >= limit {
                break;
            }
        }
    }
    Ok(out)
}

// ---------- Replay Mode (branching paths over immutable snapshots) ----------

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MemoryState {
    pub content: String,
    pub meta: serde_json::Value,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
struct PathRef {
    name: String,
    base_snapshot: String, // content hash used to seed the path
    base_node: String,     // node filename for the base snapshot
    head_node: String,     // current head node filename in this path
    created_at: String,
    updated_at: String,
}

fn path_id_from_name(name: &str) -> String {
    sanitize(name)
}

fn read_path_ref(path_name: &str) -> Result<Option<PathRef>> {
    let id = path_id_from_name(path_name);
    let p = paths_ref_dir()?.join(format!("{}.json", id));
    if !p.exists() {
        return Ok(None);
    }
    let bytes = fs::read(&p)?;
    let r: PathRef = serde_json::from_slice(&bytes).unwrap_or_default();
    if r.head_node.is_empty() {
        return Ok(None);
    }
    Ok(Some(r))
}

fn write_path_ref(path_name: &str, r: &PathRef) -> Result<()> {
    let id = path_id_from_name(path_name);
    let p = paths_ref_dir()?.join(format!("{}.json", id));
    write_atomic(&p, &serde_json::to_vec_pretty(r)?)
}

/// Recall a snapshot by its content-addressed hash id (blake3 hex).
pub fn recall_snapshot(snapshot_id: &str) -> Result<MemoryState> {
    let node_filename = if let Some(idx) = read_hash_index(snapshot_id)? {
        idx.node
    } else {
        // Fallback: linear scan for robustness in early states
        let dir = dag_nodes_dir()?;
        let mut found: Option<String> = None;
        for e in fs::read_dir(&dir)? {
            let path = e?.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let bytes = fs::read(&path)?;
            if let Ok(v) = serde_json::from_slice::<Value>(&bytes) {
                if v.get("hash").and_then(|x| x.as_str()) == Some(snapshot_id) {
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        found = Some(name.to_string());
                        break;
                    }
                }
            }
        }
        found.ok_or_else(|| anyhow!("snapshot not found: {}", snapshot_id))?
    };

    let v = load_node(&node_filename)?;
    let content = v
        .get("content")
        .and_then(|x| x.as_str())
        .unwrap_or_default()
        .to_string();
    // Merge top-level lobe/key/ts/id/hash with nested meta for convenient replay
    let mut meta_map = serde_json::Map::new();
    if let Some(m) = v.get("meta").and_then(|m| m.as_object()) {
        for (k, vv) in m.iter() {
            meta_map.insert(k.clone(), vv.clone());
        }
    }
    for k in ["lobe", "key", "ts", "id", "hash"] {
        if let Some(val) = v.get(k) {
            meta_map.insert(k.to_string(), val.clone());
        }
    }
    Ok(MemoryState {
        content,
        meta: Value::Object(meta_map),
    })
}

/// Create or reset a named path to diverge from a specific snapshot.
/// Returns the `path_id` (sanitized name).
pub fn diverge_from(snapshot_id: &str, path_name: &str) -> Result<String> {
    // Resolve snapshot to node filename (use index; fallback to scan like recall)
    let node_filename = if let Some(idx) = read_hash_index(snapshot_id)? {
        idx.node
    } else {
        // Fallback: linear scan for robustness if index is missing
        let dir = dag_nodes_dir()?;
        let mut found: Option<String> = None;
        for e in fs::read_dir(&dir)? {
            let path = e?.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let bytes = fs::read(&path)?;
            if let Ok(v) = serde_json::from_slice::<Value>(&bytes) {
                if v.get("hash").and_then(|x| x.as_str()) == Some(snapshot_id) {
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        found = Some(name.to_string());
                        break;
                    }
                }
            }
        }
        found.ok_or_else(|| anyhow!("snapshot not found: {}", snapshot_id))?
    };
    let now = chrono::Utc::now().to_rfc3339();
    let r = PathRef {
        name: path_name.to_string(),
        base_snapshot: snapshot_id.to_string(),
        base_node: node_filename.clone(),
        head_node: node_filename,
        created_at: now.clone(),
        updated_at: now,
    };
    write_path_ref(path_name, &r)?;
    Ok(path_id_from_name(path_name))
}

/// Append a new immutable snapshot to a named path and advance its head.
/// Returns the new content-addressed snapshot id (blake3 hex).
pub fn extend_path(path_name: &str, state: MemoryState) -> Result<String> {
    let mut r =
        read_path_ref(path_name)?.ok_or_else(|| anyhow!("path not found: {}", path_name))?;

    // Ensure meta has sensible lobe/key for replay isolation
    let mut meta = match state.meta {
        Value::Object(m) => Value::Object(m),
        _ => Value::Object(serde_json::Map::new()),
    };
    if meta.get("lobe").is_none() {
        meta.as_object_mut()
            .unwrap()
            .insert("lobe".into(), Value::String("replay".into()));
    }
    let key_default = path_id_from_name(path_name);
    if meta.get("key").is_none() {
        meta.as_object_mut()
            .unwrap()
            .insert("key".into(), Value::String(key_default.clone()));
    }

    // Timestamps
    let now = chrono::Utc::now().to_rfc3339();
    if meta.get("created_at").is_none() {
        meta.as_object_mut()
            .unwrap()
            .insert("created_at".into(), Value::String(now.clone()));
    }
    meta.as_object_mut()
        .unwrap()
        .insert("updated_at".into(), Value::String(now.clone()));

    // Content-addressed id
    let new_hash = blake3::hash(state.content.as_bytes()).to_hex().to_string();
    meta.as_object_mut()
        .unwrap()
        .insert("cid".into(), Value::String(new_hash.clone()));

    // Write new node, explicitly parented to current head
    let _node_file = save_node(&new_hash, &state.content, &meta, &[r.head_node.clone()])?;

    // Update path head and write back
    let latest_idx =
        read_hash_index(&new_hash)?.ok_or_else(|| anyhow!("hash index missing for new node"))?;
    r.head_node = latest_idx.node;
    r.updated_at = now;
    write_path_ref(path_name, &r)?;

    Ok(new_hash)
}

// ---------- Public helpers for paths (heads, base, ancestry) ----------

/// Return true if a path ref exists.
pub fn path_exists(path_name: &str) -> Result<bool> {
    Ok(read_path_ref(path_name)?.is_some())
}

/// Return the current head snapshot hash for a named path, if any.
pub fn path_head_hash(path_name: &str) -> Result<Option<String>> {
    if let Some(r) = read_path_ref(path_name)? {
        if r.head_node.is_empty() {
            return Ok(None);
        }
        let v = load_node(&r.head_node)?;
        let h = v
            .get("hash")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        if h.is_empty() { Ok(None) } else { Ok(Some(h)) }
    } else {
        Ok(None)
    }
}

/// Return the base snapshot hash recorded for a named path, if present.
pub fn path_base_snapshot(path_name: &str) -> Result<Option<String>> {
    if let Some(r) = read_path_ref(path_name)? {
        if r.base_snapshot.is_empty() {
            Ok(None)
        } else {
            Ok(Some(r.base_snapshot))
        }
    } else {
        Ok(None)
    }
}

/// Update a path's head to point at an existing snapshot by its content hash.
/// Fails if the hash is unknown.
pub fn set_path_head(path_name: &str, snapshot_hash: &str) -> Result<()> {
    let idx = read_hash_index(snapshot_hash)?
        .ok_or_else(|| anyhow!("snapshot not found: {}", snapshot_hash))?;
    let now = chrono::Utc::now().to_rfc3339();
    let r = if let Some(mut existing) = read_path_ref(path_name)? {
        existing.head_node = idx.node;
        existing.updated_at = now.clone();
        existing
    } else {
        // Create a new path ref seeded at this snapshot
        PathRef {
            name: path_name.to_string(),
            base_snapshot: snapshot_hash.to_string(),
            base_node: idx.node.clone(),
            head_node: idx.node,
            created_at: now.clone(),
            updated_at: now,
        }
    };
    write_path_ref(path_name, &r)
}

/// Return true if `ancestor_hash` is on the ancestor chain of `descendant_hash` (or equal).
pub fn is_ancestor(ancestor_hash: &str, descendant_hash: &str) -> Result<bool> {
    if ancestor_hash == descendant_hash {
        return Ok(true);
    }
    let mut queue: std::collections::VecDeque<String> = std::collections::VecDeque::new();
    if let Some(idx) = read_hash_index(descendant_hash)? {
        queue.push_back(idx.node);
    }
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    while let Some(fname) = queue.pop_front() {
        if !seen.insert(fname.clone()) {
            continue;
        }
        let node = load_node(&fname)?;
        if node.get("hash").and_then(|x| x.as_str()) == Some(ancestor_hash) {
            return Ok(true);
        }
        for p in node_parents_list(&node) {
            if p.is_empty() {
                continue;
            }
            if p.ends_with(".json") {
                queue.push_back(p);
            } else if let Some(idx) = read_hash_index(&p)? {
                queue.push_back(idx.node);
            } else {
                // Attempt fallback resolution: scan for a node whose internal hash matches `p`.
                if let Some(fname) = resolve_parent_filename(&p).ok().flatten() {
                    queue.push_back(fname);
                } else {
                    // Retain raw hash only if resolution failed; later iterations cannot load it directly
                    // but this preserves prior behavior for completeness.
                    queue.push_back(p);
                }
            }
        }
    }
    Ok(false)
}

/// Return the child (next) nodes of a given node *within the same stream* by scanning.
// MVP: linear scan; fine for small graphs.
pub fn children_of(filename: &str) -> Result<Vec<String>> {
    let dir = dag_nodes_dir()?;
    let mut kids = Vec::new();
    for e in fs::read_dir(&dir)? {
        let path = e?.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let bytes = fs::read(&path)?;
        if let Ok(v) = serde_json::from_slice::<Value>(&bytes) {
            let parents = parent_filenames_from_node(&v);
            if parents.iter().any(|p| p == filename) {
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

// ---------- Snapshot metadata, citations, and path tracing ----------

/// Return the `meta` object for a snapshot by its content hash id.
///
/// Multiple DAG nodes can share the same content hash (e.g. different lobes
/// remembering identical content). Rather than relying on a single hash index
/// entry, scan for all matching nodes and bind their metadata, prioritising
/// the indexed node when available.
pub fn snapshot_meta(snapshot_id: &str) -> Result<Value> {
    let mut metas: Vec<serde_json::Map<String, Value>> = Vec::new();
    let mut seen_files: std::collections::HashSet<String> = std::collections::HashSet::new();

    if let Some(idx) = read_hash_index(snapshot_id)? {
        let node = load_node(&idx.node)?;
        if let Some(meta_obj) = node.get("meta").and_then(|m| m.as_object()) {
            metas.push(meta_obj.clone());
        } else {
            metas.push(serde_json::Map::new());
        }
        seen_files.insert(idx.node);
    }

    let dir = dag_nodes_dir()?;
    for entry in fs::read_dir(&dir)? {
        let path = entry?.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        if seen_files.contains(&name) {
            continue;
        }
        let bytes = fs::read(&path)?;
        let node: Value = serde_json::from_slice(&bytes)?;
        if node.get("hash").and_then(|x| x.as_str()) != Some(snapshot_id) {
            continue;
        }
        if let Some(meta_obj) = node.get("meta").and_then(|m| m.as_object()) {
            metas.push(meta_obj.clone());
        } else {
            metas.push(serde_json::Map::new());
        }
        seen_files.insert(name);
    }

    if metas.is_empty() {
        return Err(anyhow!("snapshot not found: {}", snapshot_id));
    }

    let mut binding = metas.remove(0);
    for meta in metas.iter() {
        bind_meta_maps(&mut binding, meta);
    }

    Ok(Value::Object(binding))
}

fn bind_meta_maps(
    base: &mut serde_json::Map<String, Value>,
    incoming: &serde_json::Map<String, Value>,
) {
    for (k, v) in incoming {
        if k == "provenance" {
            bind_provenance(base, v);
            continue;
        }
        let should_set = !base.contains_key(k)
            || base
                .get(k)
                .map(|existing| existing.is_null())
                .unwrap_or(false);
        if should_set {
            base.insert(k.clone(), v.clone());
        }
    }
}

fn bind_provenance(base: &mut serde_json::Map<String, Value>, incoming: &Value) {
    let incoming_obj = match incoming.as_object() {
        Some(map) => map,
        None => return,
    };

    let prov_entry = base
        .entry("provenance".to_string())
        .or_insert_with(|| Value::Object(serde_json::Map::new()));
    if !prov_entry.is_object() {
        *prov_entry = Value::Object(serde_json::Map::new());
    }
    let prov_map = prov_entry.as_object_mut().expect("provenance object");

    let mut bindd_sources: Vec<Value> = prov_map
        .get("sources")
        .and_then(|s| s.as_array())
        .map(|arr| arr.clone())
        .unwrap_or_else(Vec::new);

    let mut seen: std::collections::HashSet<String> =
        bindd_sources.iter().map(provenance_source_key).collect();

    if let Some(new_sources) = incoming_obj.get("sources").and_then(|s| s.as_array()) {
        for src in new_sources {
            let key = provenance_source_key(src);
            if seen.insert(key) {
                bindd_sources.push(src.clone());
            }
        }
    }

    prov_map.insert("sources".to_string(), Value::Array(bindd_sources));

    for (k, value) in incoming_obj {
        if k == "sources" {
            continue;
        }
        prov_map.entry(k.clone()).or_insert_with(|| value.clone());
    }
}

fn provenance_source_key(src: &Value) -> String {
    serde_json::json!({
        "kind": src.get("kind"),
        "uri": src.get("uri"),
        "cid": src.get("cid"),
        "range": src.get("range"),
    })
    .to_string()
}

/// Flatten and return any provenance.sources listed in the snapshot meta; de-duplicates basic tuples.
pub fn cite_sources(snapshot_id: &str) -> Result<Vec<Value>> {
    let meta = snapshot_meta(snapshot_id)?;
    let mut out: Vec<Value> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    if let Some(arr) = meta
        .get("provenance")
        .and_then(|p| p.get("sources"))
        .and_then(|s| s.as_array())
    {
        for s in arr {
            let key = provenance_source_key(s);
            if seen.insert(key) {
                out.push(s.clone());
            }
        }
    }
    Ok(out)
}

/// Trace a named path from head backwards following parent pointers; newest -> oldest up to `limit`.
/// Returns a vector of lightweight objects with id/hash/ts/lobe/key and source counts.
pub fn trace_path(path_name: &str, limit: usize) -> Result<Vec<Value>> {
    let r = read_path_ref(path_name)?.ok_or_else(|| anyhow!("path not found: {}", path_name))?;
    let mut cur = Some(r.head_node);
    let mut out: Vec<Value> = Vec::new();
    let mut n = 0usize;
    while let Some(fname) = cur {
        if n >= limit {
            break;
        }
        let node = load_node(&fname)?;
        let meta = node
            .get("meta")
            .cloned()
            .unwrap_or(Value::Object(serde_json::Map::new()));
        let prov_count = meta
            .get("provenance")
            .and_then(|p| p.get("sources"))
            .and_then(|s| s.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        let item = serde_json::json!({
            "filename": fname,
            "id": node.get("id").and_then(|x| x.as_str()).unwrap_or_default(),
            "hash": node.get("hash").and_then(|x| x.as_str()).unwrap_or_default(),
            "ts": node.get("ts").and_then(|x| x.as_str()).unwrap_or_default(),
            "lobe": node.get("lobe").and_then(|x| x.as_str()).unwrap_or_default(),
            "key": node.get("key").and_then(|x| x.as_str()).unwrap_or_default(),
            "provenance_sources": prov_count,
        });
        out.push(item);
        // Choose the primary parent if multiple; prefer the first entry in `parents`.
        let next_parent: Option<String> = {
            let parents = node_parents_list(&node);
            if let Some(p) = parents.first() {
                if p.ends_with(".json") {
                    Some(p.clone())
                } else if let Some(idx) = read_hash_index(p)? {
                    Some(idx.node)
                } else if let Some(fname) = resolve_parent_filename(p)? {
                    // legacy fallback: resolve bare hash to filename
                    Some(fname)
                } else {
                    None
                }
            } else {
                None
            }
        };
        cur = next_parent;
        n += 1;
    }
    Ok(out)
}

/// Keep only the newest `keep_last_per_stream` nodes per (lobe,key).
pub fn prune(keep_last_per_stream: usize) -> Result<PruneReport> {
    let dir = dag_nodes_dir()?;
    let mut by_stream: std::collections::BTreeMap<(String, String), Vec<(String, String)>> =
        Default::default();
    // collect: (lobe,key) -> [(ts, filename)]
    for e in fs::read_dir(&dir)? {
        let path = e?.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let bytes = fs::read(&path)?;
        let v: Value = match serde_json::from_slice(&bytes) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let lobe = v
            .get("lobe")
            .and_then(|x| x.as_str())
            .unwrap_or("unknown")
            .to_string();
        let key = v
            .get("key")
            .and_then(|x| x.as_str())
            .unwrap_or("default")
            .to_string();
        let ts = v
            .get("ts")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
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

// ---------- Merge helpers ----------

/// Compute the bind base (lowest common ancestor) between two snapshot hashes.
/// Returns Some(ancestor_hash) if found.
pub fn bind_base(a_hash: &str, b_hash: &str) -> Result<Option<String>> {
    if a_hash == b_hash {
        return Ok(Some(a_hash.to_string()));
    }
    // Collect ancestors of A (by hash), including A.
    let mut aset: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut qa: std::collections::VecDeque<String> = std::collections::VecDeque::new();
    if let Some(idx) = read_hash_index(a_hash)? {
        qa.push_back(idx.node);
    } else {
        // Fallback: scan dag_nodes_dir for a file whose internal "hash" matches a_hash
        if let Ok(dir) = dag_nodes_dir() {
            if let Ok(entries) = fs::read_dir(&dir) {
                for ent in entries.flatten() {
                    let p = ent.path();
                    if p.extension().and_then(|s| s.to_str()) != Some("json") {
                        continue;
                    }
                    if let Ok(v) = load_node(
                        &p.file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or_default()
                            .to_string(),
                    ) {
                        if let Some(h) = v.get("hash").and_then(|x| x.as_str()) {
                            if h == a_hash {
                                if let Some(fname) = p.file_name().and_then(|n| n.to_str()) {
                                    qa.push_back(fname.to_string());
                                }
                                break;
                            }
                        }
                    }
                }
            }
        }
    }
    while let Some(fname) = qa.pop_front() {
        let node = match load_node(&fname) {
            Ok(n) => n,
            Err(_) => continue,
        };
        if let Some(h) = node.get("hash").and_then(|x| x.as_str()) {
            aset.insert(h.to_string());
        }
        for p in node_parents_list(&node) {
            if p.ends_with(".json") {
                qa.push_back(p);
            } else if let Some(idx) = read_hash_index(&p)? {
                qa.push_back(idx.node);
            } else if let Some(fname) = resolve_parent_filename(&p).ok().flatten() {
                qa.push_back(fname);
            }
        }
    }
    // BFS from B until hitting any in A's ancestor set (nearest to B wins).
    let mut qb: std::collections::VecDeque<String> = std::collections::VecDeque::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    if let Some(idx) = read_hash_index(b_hash)? {
        qb.push_back(idx.node);
    } else {
        if let Ok(dir) = dag_nodes_dir() {
            if let Ok(entries) = fs::read_dir(&dir) {
                for ent in entries.flatten() {
                    let p = ent.path();
                    if p.extension().and_then(|s| s.to_str()) != Some("json") {
                        continue;
                    }
                    if let Ok(v) = load_node(
                        &p.file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or_default()
                            .to_string(),
                    ) {
                        if let Some(h) = v.get("hash").and_then(|x| x.as_str()) {
                            if h == b_hash {
                                if let Some(fname) = p.file_name().and_then(|n| n.to_str()) {
                                    qb.push_back(fname.to_string());
                                }
                                break;
                            }
                        }
                    }
                }
            }
        }
    }
    while let Some(fname) = qb.pop_front() {
        if !seen.insert(fname.clone()) {
            continue;
        }
        let node = match load_node(&fname) {
            Ok(n) => n,
            Err(_) => continue,
        };
        if let Some(h) = node.get("hash").and_then(|x| x.as_str()) {
            if aset.contains(h) {
                return Ok(Some(h.to_string()));
            }
        }
        for p in node_parents_list(&node) {
            if p.ends_with(".json") {
                qb.push_back(p);
            } else if let Some(idx) = read_hash_index(&p)? {
                qb.push_back(idx.node);
            } else if let Some(fname) = resolve_parent_filename(&p).ok().flatten() {
                qb.push_back(fname);
            }
        }
    }
    Ok(None)
}
