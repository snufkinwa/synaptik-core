use pyo3::prelude::*;
use pyo3::types::{PyAny, PyBytes, PyDict, PyList};

use anyhow::anyhow;
use blake3;
use chrono::Utc;
use serde_json::{json, Value};
use std::sync::Arc;

extern crate synaptik_core as syn_core;
use contracts::types::MoralContract;
use syn_core::commands::{Commands, EthosReport, HitSource, Prefer};
use syn_core::memory::dag::MemoryState as DagMemoryState;
use syn_core::services::streamgate::{GateDecision as CoreGateDecision, StreamGate as CoreStreamGate, StreamGateConfig, StreamingIndex};
use syn_core::utils::pons::{ObjectMetadata as PonsMetadata, ObjectRef as PonsObjectRef};
use syn_core::services::{FinalizedStatus, LlmClient, StreamRuntime};
use syn_core::services::ethos::{ContractsDecider, Proposal};

fn pyerr<E: std::fmt::Display>(e: E) -> PyErr {
    pyo3::exceptions::PyRuntimeError::new_err(e.to_string())
}

#[pyclass]
struct PyCommands {
    inner: Commands,
    // Last recalled snapshot id (content hash) to seed auto-diverge/extend flows
    last_recalled: Option<String>,
}

// -------- helpers (local to python bindings) --------

fn sanitize_name(name: &str) -> String {
    let mut s = name.to_lowercase();
    s.retain(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if s.is_empty() {
        s = "path".into();
    }
    s
}

// Mirror DAG's sanitize for path id filenames: replace non-alnum with '_', preserve case.
fn dag_path_id(name: &str) -> String {
    name.chars()
    .map(|c| c.is_ascii_alphanumeric().then_some(c).unwrap_or('_'))
        .collect()
}

fn gen_path_name(prefix: &str, seed: &str) -> String {
    let ts = Utc::now().format("%Y%m%d-%H%M%S");
    let sh = &blake3::hash(seed.as_bytes()).to_hex()[..8];
    format!("{}-{}-{}", prefix, ts, sh)
}

fn merge_json(base: Value, overlay: Value) -> Value {
    use serde_json::Value::*;
    match (base, overlay) {
        (Object(mut b), Object(o)) => {
            for (k, v) in o {
                b.insert(k, v);
            }
            Object(b)
        }
        (_, o) => o,
    }
}

fn path_exists_in_refs(path_name: &str) -> anyhow::Result<bool> {
    let rep = syn_core::commands::init::ensure_initialized_once()?;
    // Normalize to match core's lowercase normalization policy
    let norm = sanitize_name(path_name);
    let pid = dag_path_id(&norm);
    let p = rep
        .root
        .join("refs")
        .join("paths")
        .join(format!("{}.json", pid));
    Ok(p.exists())
}

fn json_to_py(py: Python<'_>, v: &Value) -> PyObject {
    use serde_json::Value::*;
    match v {
        Null => py.None().into_py(py),
        Bool(b) => b.into_py(py),
        Number(n) => {
            if let Some(i) = n.as_i64() {
                i.into_py(py)
            } else if let Some(u) = n.as_u64() {
                (u as i128).into_py(py)
            } else if let Some(f) = n.as_f64() {
                f.into_py(py)
            } else {
                py.None().into_py(py)
            }
        }
        String(s) => s.into_py(py),
        Array(arr) => {
            let list = PyList::empty_bound(py);
            for item in arr {
                list.append(json_to_py(py, item)).ok();
            }
            list.into_any().into_py(py)
        }
        Object(map) => {
            let d = PyDict::new_bound(py);
            for (k, val) in map.iter() {
                let _ = d.set_item(k, json_to_py(py, val));
            }
            d.into_any().into_py(py)
        }
    }
}

fn json_array_to_py(py: Python<'_>, arr: &[Value]) -> PyObject {
    let list = PyList::empty_bound(py);
    for item in arr {
        let _ = list.append(json_to_py(py, item));
    }
    list.into_any().into_py(py)
}

fn py_to_json(any: &Bound<'_, PyAny>) -> Value {
    if let Ok(b) = any.extract::<bool>() {
        return Value::Bool(b);
    }
    if let Ok(i) = any.extract::<i64>() {
        return json!(i);
    }
    if let Ok(f) = any.extract::<f64>() {
        return json!(f);
    }
    if let Ok(s) = any.extract::<String>() {
        return json!(s);
    }
    if any.is_none() {
        return Value::Null;
    }
    if let Ok(dict) = any.downcast::<PyDict>() {
        let mut m = serde_json::Map::new();
        for (k, v) in dict.iter() {
            let ks: String = k.extract().unwrap_or_default();
            m.insert(ks, py_to_json(&v));
        }
        return Value::Object(m);
    }
    if let Ok(list) = any.downcast::<PyList>() {
        let mut a = Vec::with_capacity(list.len());
        for v in list.iter() {
            a.push(py_to_json(&v));
        }
        return Value::Array(a);
    }
    if let Ok(s) = any.str() {
        return json!(s.to_string());
    }
    Value::Null
}

fn object_ref_to_py(py: Python<'_>, r: &PonsObjectRef) -> PyObject {
    let d = PyDict::new_bound(py);
    let _ = d.set_item("pons", &r.pons);
    let _ = d.set_item("key", &r.key);
    // Adjust field names if needed to match the actual struct
    let _ = d.set_item("version", r.version.clone());
    let _ = d.set_item("etag", r.etag.clone());
    let _ = d.set_item("size_bytes", r.size_bytes);
    d.into_any().into_py(py)
}

fn metadata_to_py(py: Python<'_>, meta: &PonsMetadata) -> PyObject {
    let d = PyDict::new_bound(py);
    let _ = d.set_item("media_type", meta.media_type.clone());
    match &meta.extra {
        Some(v) => {
            let _ = d.set_item("extra", json_to_py(py, v));
        }
        None => {
            let _ = d.set_item("extra", py.None());
        }
    }
    d.into_any().into_py(py)
}

fn decision_to_py(decision: CoreGateDecision) -> PyGateDecision {
    match decision {
        CoreGateDecision::Pass => PyGateDecision {
            kind: "Pass",
            message: None,
        },
        CoreGateDecision::Hold => PyGateDecision {
            kind: "Hold",
            message: None,
        },
        CoreGateDecision::CutAndReplace(msg) => PyGateDecision {
            kind: "CutAndReplace",
            message: Some(msg),
        },
    }
}

#[pymethods]
impl PyCommands {
    #[new]
    fn new() -> PyResult<Self> {
        syn_core::commands::init::ensure_initialized_once().map_err(pyerr)?;
        let inner = Commands::new("ignored", None).map_err(pyerr)?;
        Ok(Self {
            inner,
            last_recalled: None,
        })
    }

    fn precheck_text(&self, py: Python<'_>, text: &str, purpose: &str) -> PyResult<PyObject> {
        let rep: EthosReport = self.inner.precheck_text(text, purpose).map_err(pyerr)?;
        let d = PyDict::new_bound(py);
        d.set_item("decision", rep.decision)?;
        d.set_item("reason", rep.reason)?;
        d.set_item("risk", rep.risk)?;
        d.set_item("constraints", rep.constraints)?;
        d.set_item("action_suggestion", rep.action_suggestion)?;
        d.set_item("violation_code", rep.violation_code)?;
        Ok(d.into_any().into_py(py))
    }

    /// Govern text through Synaptik's contract-enforced runtime without persisting.
    /// Returns a dict {status: "ok|violated|stopped|escalated", text, violation_label}.
    #[pyo3(signature = (intent, input))]
    fn govern_text(&self, py: Python<'_>, intent: &str, input: &str) -> PyResult<PyObject> {
        // Minimal echo streaming model to drive the runtime without network calls.
        struct EchoStream { yielded: bool, text: String }
        impl Iterator for EchoStream {
            type Item = String;
            fn next(&mut self) -> Option<Self::Item> {
                if self.yielded { None } else { self.yielded = true; Some(self.text.clone()) }
            }
        }
        struct EchoModel { text: String }
        impl LlmClient for EchoModel {
            type Stream = EchoStream;
            fn stream(&self, _system_prompt: String) -> Result<Self::Stream, syn_core::services::GateError> {
                Ok(EchoStream { yielded: false, text: self.text.clone() })
            }
        }

        let proposal = Proposal { intent: intent.to_string(), input: input.to_string(), prior: None, tools_requested: vec![] };
        let contract = ContractsDecider;
        let model = EchoModel { text: input.to_string() };
        let rt = StreamRuntime { contract, model };
        let res = rt.generate(proposal).map_err(pyerr)?;

        let status = match res.status {
            FinalizedStatus::Ok => "ok",
            FinalizedStatus::Violated => "violated",
            FinalizedStatus::Stopped => "stopped",
            FinalizedStatus::Escalated => "escalated",
        };
        let d = PyDict::new_bound(py);
        d.set_item("status", status)?;
        d.set_item("text", res.text)?;
        d.set_item("violation_label", res.violation_label)?;
        Ok(d.into_any().into_py(py))
    }

    #[pyo3(signature = (lobe, content, key=None))]
    fn remember(&self, lobe: &str, content: &str, key: Option<&str>) -> PyResult<String> {
        self.inner.remember(lobe, key, content).map_err(pyerr)
    }

    fn reflect(&self, lobe: &str, window: usize) -> PyResult<String> {
        self.inner.reflect(lobe, window).map_err(pyerr)
    }

    #[pyo3(signature = (lobe, n=10))]
    fn recent(&self, lobe: &str, n: usize) -> PyResult<Vec<String>> {
        self.inner.recent(lobe, n).map_err(pyerr)
    }

    /// Unified recall that returns a dict {id, content, source} or None.
    /// prefer: "hot" | "archive" | "dag" | "auto" (default)
    #[pyo3(signature = (memory_id, prefer=None))]
    fn recall(
        &self,
        py: Python<'_>,
        memory_id: &str,
        prefer: Option<&str>,
    ) -> PyResult<Option<PyObject>> {
        let p = match prefer.unwrap_or("auto") {
            "hot" => Prefer::Hot,
            "archive" => Prefer::Archive,
            "dag" => Prefer::Dag,
            _ => Prefer::Auto,
        };
        match self.inner.recall_any(memory_id, p).map_err(pyerr)? {
            Some(hit) => {
                let d = PyDict::new_bound(py);
                d.set_item("id", hit.memory_id)?;
                d.set_item("content", hit.content)?;
                let src = match hit.source {
                    HitSource::Hot => "hot",
                    HitSource::Archive => "archive",
                    HitSource::Dag => "dag",
                };
                d.set_item("source", src)?;
                Ok(Some(d.into_any().into_py(py)))
            }
            None => Ok(None),
        }
    }

    /// Simple recall: return just the content string or None, with an optional tier preference.
    /// prefer: "hot" | "archive" | "dag" | "auto" (default = auto)
    #[pyo3(signature = (memory_id, prefer=None))]
    fn recall_prefer(&self, memory_id: &str, prefer: Option<&str>) -> PyResult<Option<String>> {
        let p = match prefer.unwrap_or("auto") {
            "hot" => Prefer::Hot,
            "archive" => Prefer::Archive,
            "dag" => Prefer::Dag,
            _ => Prefer::Auto,
        };
        Ok(self
            .inner
            .recall_any(memory_id, p)
            .map_err(pyerr)?
            .map(|r| r.content))
    }

    /// Bulk recall. Returns a list of dicts: [{id, content, source}]
    #[pyo3(signature = (memory_ids, prefer=None))]
    fn recall_many(
        &self,
        py: Python<'_>,
        memory_ids: Vec<String>,
        prefer: Option<&str>,
    ) -> PyResult<PyObject> {
        let p = match prefer.unwrap_or("auto") {
            "hot" => Prefer::Hot,
            "archive" => Prefer::Archive,
            "dag" => Prefer::Dag,
            _ => Prefer::Auto,
        };
        let results = self.inner.recall_many(&memory_ids, p).map_err(pyerr)?;
        let out = PyList::empty_bound(py);
        for r in results {
            let d = PyDict::new_bound(py);
            d.set_item("id", r.memory_id)?;
            d.set_item("content", r.content)?;
            let src = match r.source {
                HitSource::Hot => "hot",
                HitSource::Archive => "archive",
                HitSource::Dag => "dag",
            };
            d.set_item("source", src)?;
            out.append(d)?;
        }
        Ok(out.into_any().into_py(py))
    }

    fn pons_create(&self, name: &str) -> PyResult<()> {
        self.inner.pons_create(name).map_err(pyerr)
    }

    #[pyo3(signature = (pons, key, data, media_type=None, extra=None))]
    fn pons_put_object(
        &self,
        py: Python<'_>,
        pons: &str,
        key: &str,
        data: &Bound<'_, PyAny>,
        media_type: Option<&str>,
        extra: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<PyObject> {
        let payload = if let Ok(bytes) = data.extract::<Vec<u8>>() {
            bytes
        } else if let Ok(bytes) = data.extract::<&[u8]>() {
            bytes.to_vec()
        } else if let Ok(s) = data.extract::<String>() {
            s.into_bytes()
        } else {
            return Err(pyo3::exceptions::PyTypeError::new_err(
                "data must be bytes, bytearray, or str",
            ));
        };

        let extra_json = extra.and_then(|v| {
            let val = py_to_json(v);
            if val.is_null() {
                None
            } else {
                Some(val)
            }
        });

        let obj = self
            .inner
            .pons_put_object(pons, key, &payload, media_type, extra_json)
            .map_err(pyerr)?;
        Ok(object_ref_to_py(py, &obj))
    }

    fn pons_get_latest_bytes(&self, py: Python<'_>, pons: &str, key: &str) -> PyResult<PyObject> {
        let bytes = self.inner.pons_get_latest_bytes(pons, key).map_err(pyerr)?;
        Ok(PyBytes::new_bound(py, &bytes).into_py(py))
    }

    fn pons_get_latest_ref(&self, py: Python<'_>, pons: &str, key: &str) -> PyResult<PyObject> {
        let rf = self.inner.pons_get_latest_ref(pons, key).map_err(pyerr)?;
        Ok(object_ref_to_py(py, &rf))
    }

    fn pons_get_version_with_meta(
        &self,
        py: Python<'_>,
        pons: &str,
        key: &str,
        version: &str,
    ) -> PyResult<(PyObject, PyObject)> {
        let (bytes, meta) = self
            .inner
            .pons_get_version_with_meta(pons, key, version)
            .map_err(pyerr)?;
        let b = PyBytes::new_bound(py, &bytes).into_py(py);
        let m = metadata_to_py(py, &meta);
        Ok((b, m))
    }

    #[pyo3(signature = (pons, prefix=None, limit=20))]
    fn pons_list_latest(
        &self,
        py: Python<'_>,
        pons: &str,
        prefix: Option<&str>,
        limit: usize,
    ) -> PyResult<Vec<PyObject>> {
        let refs = self
            .inner
            .pons_list_latest(pons, prefix, limit)
            .map_err(pyerr)?;
        let mut out = Vec::with_capacity(refs.len());
        for r in refs {
            out.push(object_ref_to_py(py, &r));
        }
        Ok(out)
    }

    /// Stats dict: { total, archived, by_lobe: [(lobe, count)], last_updated }
    #[pyo3(signature = (lobe=None))] // ← Python<'_> must NOT be in signature list
    fn stats(&self, py: Python<'_>, lobe: Option<&str>) -> PyResult<PyObject> {
        let s = self.inner.stats(lobe).map_err(pyerr)?;
        let d = PyDict::new_bound(py);
        d.set_item("total", s.total)?;
        d.set_item("archived", s.archived)?;

        let by_lobe = PyList::empty_bound(py);
        for (l, c) in s.by_lobe {
            by_lobe.append((l, c))?;
        }
        d.set_item("by_lobe", by_lobe)?;
        d.set_item("last_updated", s.last_updated)?;
        Ok(d.into_any().into_py(py))
    }

    // -------------------- Replay (Rewind & Diverge) --------------------

    /// Recall an immutable snapshot by content hash. Optionally export content to a file.
    /// Returns {"content": str, "meta": dict} and sets this snapshot as the implicit base.
    #[pyo3(signature = (snapshot_id, export=None))]
    fn recall_snapshot(
        &mut self,
        py: Python<'_>,
        snapshot_id: &str,
        export: Option<&str>,
    ) -> PyResult<PyObject> {
        let s = self
            .inner
            .replay_recall_snapshot(snapshot_id)
            .map_err(pyerr)?;
        if let Some(path) = export {
            std::fs::write(path, &s.content).map_err(pyerr)?;
        }
        self.last_recalled = Some(snapshot_id.to_string());
        let d = PyDict::new_bound(py);
        d.set_item("content", s.content)?;
        d.set_item("meta", json_to_py(py, &s.meta))?;
        Ok(d.into_any().into_py(py))
    }

    /// Recall and immediately create/reset a named branch. If `path_name` is None, generate one.
    /// Returns the path_id (sanitized name) and sets it as active by convention on next extend.
    #[pyo3(signature = (snapshot_id, path_name=None, export=None))]
    fn recall_and_diverge(
        &mut self,
        py: Python<'_>,
        snapshot_id: &str,
        path_name: Option<&str>,
        export: Option<&str>,
    ) -> PyResult<String> {
        let _ = self.recall_snapshot(py, snapshot_id, export)?;
        let name = path_name
            .map(sanitize_name)
            .unwrap_or_else(|| gen_path_name("replay", snapshot_id));
        let id = self
            .inner
            .replay_diverge_from(snapshot_id, &name)
            .map_err(pyerr)?;
        Ok(id)
    }

    /// Create/reset a named branch from a snapshot. If name is None, a new name is generated.
    #[pyo3(signature = (snapshot_id, path_name=None))]
    fn diverge_from(&self, snapshot_id: &str, path_name: Option<&str>) -> PyResult<String> {
        let name = path_name
            .map(sanitize_name)
            .unwrap_or_else(|| gen_path_name("replay", snapshot_id));
        self.inner
            .replay_diverge_from(snapshot_id, &name)
            .map_err(pyerr)
    }

    /// Append a new snapshot to a branch. Auto-creates the branch from the last recalled snapshot if missing.
    /// Meta is optional; minimal fields are auto-enriched.
    #[pyo3(signature = (path_name, content, meta=None))]
    fn extend_path(
        &mut self,
        py: Python<'_>,
        path_name: &str,
        content: &str,
        meta: Option<PyObject>,
    ) -> PyResult<String> {
        // ensure branch exists; if not, seed from last recall
        let norm = sanitize_name(path_name);
        if !path_exists_in_refs(&norm).map_err(pyerr)? {
            let base = self.last_recalled.clone().ok_or_else(|| {
                pyerr(anyhow!(
                    "Path '{}' not found and no prior recall to seed from.",
                    path_name
                ))
            })?;
            let _ = self
                .inner
                .replay_diverge_from(&base, &norm)
                .map_err(pyerr)?;
        }

        // meta: safe extract and enrich
        let mut meta_value: Value = if let Some(obj) = meta {
            let bound = obj.bind(py);
            py_to_json(&bound)
        } else {
            json!({})
        };

        let content_hash = blake3::hash(content.as_bytes()).to_hex().to_string();
        let now = Utc::now().to_rfc3339();
        let enrich = json!({
            "op": "extend_path",
            "ts": now,
            "actor": "python",
            "content_hash": content_hash,
            "path_name": path_name,
        });
        meta_value = merge_json(enrich, meta_value);

        // build state and extend
        let state = DagMemoryState {
            content: content.to_string(),
            meta: meta_value,
        };
        let new_id = self.inner.replay_extend_path(&norm, state).map_err(pyerr)?;
        Ok(new_id)
    }

    /// Raw meta for a snapshot (including provenance if set) -> dict
    fn snapshot_meta(&self, py: Python<'_>, snapshot_id: &str) -> PyResult<PyObject> {
        let v = self.inner.dag_snapshot_meta(snapshot_id).map_err(pyerr)?;
        Ok(json_to_py(py, &v))
    }

    /// Walk path newest->oldest (limit). Returns list of dicts with id/hash/ts/lobe/key/source counts.
    #[pyo3(signature = (path_name, limit=50))]
    fn trace_path(&self, py: Python<'_>, path_name: &str, limit: usize) -> PyResult<PyObject> {
        let norm = sanitize_name(path_name);
        let v = self.inner.dag_trace_path(&norm, limit).map_err(pyerr)?;
    Ok(json_array_to_py(py, &v))
    }

    /// Flatten + de-dup provenance sources for a snapshot. Returns list[dict].
    fn cite_sources(&self, py: Python<'_>, snapshot_id: &str) -> PyResult<PyObject> {
        let v = self.inner.dag_cite_sources(snapshot_id).map_err(pyerr)?;
    Ok(json_array_to_py(py, &v))
    }

    // Note: duplicate pruning is automated in the Rust core during writes.

    // -------------------- Convenience helpers --------------------

    /// Return the last snapshot id recalled via recall_snapshot (if any).
    fn last_recalled_id(&self) -> Option<String> {
        self.last_recalled.clone()
    }

    /// Get the newest snapshot hash on a named path (if any).
    fn latest_on_path(&self, path_name: &str) -> PyResult<Option<String>> {
        let norm = sanitize_name(path_name);
        let v = self.inner.dag_trace_path(&norm, 1).map_err(pyerr)?;
        if let Some(first) = v.first() {
            if let Some(hash) = first.get("hash").and_then(|x| x.as_str()) {
                return Ok(Some(hash.to_string()));
            }
        }
        Ok(None)
    }

    /// Recall newest snapshot on a path. Returns dict {content, meta} or None.
    #[pyo3(signature = (path_name, export=None))]
    fn recall_latest_on_path(
        &mut self,
        py: Python<'_>,
        path_name: &str,
        export: Option<&str>,
    ) -> PyResult<Option<PyObject>> {
        match self.latest_on_path(&sanitize_name(path_name))? {
            Some(hash) => {
                let obj = self.recall_snapshot(py, &hash, export)?;
                Ok(Some(obj))
            }
            None => Ok(None),
        }
    }

    fn root(&self) -> PyResult<String> {
        let rep = syn_core::commands::init::ensure_initialized_once().map_err(pyerr)?;
        Ok(rep.root.to_string_lossy().to_string())
    }

    /// Ensure a replay base exists for the given lobe and return its CID (blake3 hex), if any.
    /// Prefers the latest archived CID; otherwise promotes the latest hot row.
    fn seed_base_from_lobe(&self, lobe: &str) -> PyResult<Option<String>> {
        self.inner.replay_base_from_lobe(lobe).map_err(pyerr)
    }

    /// Begin a replay branch from the latest available base in a lobe.
    /// If `path_name` is None, a unique name is generated and returned.
    #[pyo3(signature = (lobe, path_name=None))]
    fn begin_branch(&mut self, lobe: &str, path_name: Option<&str>) -> PyResult<String> {
        let cid = self
            .inner
            .replay_base_from_lobe(lobe)
            .map_err(pyerr)?
            .ok_or_else(|| {
                pyerr(anyhow!(format!(
                    "No base snapshot available for lobe '{lobe}'"
                )))
            })?;
        let name = path_name
            .map(sanitize_name)
            .unwrap_or_else(|| gen_path_name("replay", &cid));
        let id = self.inner.replay_diverge_from(&cid, &name).map_err(pyerr)?;
        self.last_recalled = Some(cid);
        Ok(id)
    }

    // -------------------- New high-level helpers --------------------

    /// Create or reuse a branch at a resolved base (cid|path|lobe). Returns base cid.
    #[pyo3(signature = (path, base=None, lobe=None))]
    fn branch(&self, path: &str, base: Option<&str>, lobe: Option<&str>) -> PyResult<String> {
        let norm = sanitize_name(path);
        self.inner.branch(&norm, base, lobe).map_err(pyerr)
    }

    /// Append content to a path with auto-provenance and Ethos gating. Returns new cid.
    #[pyo3(signature = (path, content, meta=None))]
    fn append(
        &self,
        py: Python<'_>,
        path: &str,
        content: &str,
        meta: Option<PyObject>,
    ) -> PyResult<String> {
        let norm = sanitize_name(path);
        let meta_value = meta
            .as_ref()
            .map(|o| py_to_json(&o.bind(py)))
            .unwrap_or(serde_json::json!({}));
        self.inner
            .append(&norm, content, Some(meta_value))
            .map_err(pyerr)
    }

    /// Fast-forward consolidate: move dst to src head if ancestor. Returns new head.
    #[pyo3(signature = (src_path, dst_path="main"))]
    fn consolidate(&self, src_path: &str, dst_path: &str) -> PyResult<String> {
        let src = sanitize_name(src_path);
        let dst = sanitize_name(dst_path);
        self.inner.consolidate(&src, &dst).map_err(pyerr)
    }

    /// Neuroscience alias: systems consolidation (FF-only).
    #[pyo3(signature = (src_path, dst_path="main"))]
    fn systems_consolidate(&self, src_path: &str, dst_path: &str) -> PyResult<String> {
        let src = sanitize_name(src_path);
        let dst = sanitize_name(dst_path);
        self.inner.systems_consolidate(&src, &dst).map_err(pyerr)
    }

    /// Merge placeholder: errors unless FF is possible (until two-parent nodes exist).
    #[pyo3(signature = (src_path, dst_path="main", note=""))]
    fn merge(&self, src_path: &str, dst_path: &str, note: &str) -> PyResult<String> {
        let src = sanitize_name(src_path);
        let dst = sanitize_name(dst_path);
        self.inner.merge(&src, &dst, note).map_err(pyerr)
    }

    /// Neuroscience alias: reconsolidate paths (merge placeholder; FF-only today).
    #[pyo3(signature = (src_path, dst_path="main", note=""))]
    fn reconsolidate_paths(&self, src_path: &str, dst_path: &str, note: &str) -> PyResult<String> {
        let src = sanitize_name(src_path);
        let dst = sanitize_name(dst_path);
        // Same behavior as merge() today: FF when possible, else error until 2-parent nodes exist.
        self.inner
            .reconsolidate_paths(&dst, &src, note)
            .map_err(pyerr)
    }

    /// Neuroscience alias: sprout a dendrite (normalized, idempotent branch).
    #[pyo3(signature = (path, base=None, lobe=None))]
    fn sprout_dendrite(
        &self,
        path: &str,
        base: Option<&str>,
        lobe: Option<&str>,
    ) -> PyResult<String> {
        let norm = sanitize_name(path);
        self.inner.branch(&norm, base, lobe).map_err(pyerr)
    }

    /// Neuroscience alias: encode an engram (append with ethos + provenance).
    #[pyo3(signature = (path, content, meta=None))]
    fn encode_engram(
        &self,
        py: Python<'_>,
        path: &str,
        content: &str,
        meta: Option<PyObject>,
    ) -> PyResult<String> {
        let norm = sanitize_name(path);
        let meta_value = meta
            .as_ref()
            .map(|o| py_to_json(&o.bind(py)))
            .unwrap_or(serde_json::json!({}));
        self.inner
            .append(&norm, content, Some(meta_value))
            .map_err(pyerr)
    }

    /// Path head hash.
    fn dag_head(&self, path_name: &str) -> PyResult<Option<String>> {
        let norm = sanitize_name(path_name);
        self.inner.dag_head(&norm).map_err(pyerr)
    }

    /// Force set path head to a specific snapshot (creates path if missing).
    fn update_path_head(&self, path_name: &str, snapshot_hash: &str) -> PyResult<()> {
        let norm = sanitize_name(path_name);
        self.inner
            .update_path_head(&norm, snapshot_hash)
            .map_err(pyerr)
    }

    /// Neuroscience alias: latest engram on a path (head hash).
    fn engram_head(&self, path_name: &str) -> PyResult<Option<String>> {
        let norm = sanitize_name(path_name);
        self.inner.dag_head(&norm).map_err(pyerr)
    }

    /// Neuroscience alias: set engram head to a specific snapshot (creates path if missing).
    fn set_engram_head(&self, path_name: &str, snapshot_hash: &str) -> PyResult<()> {
        let norm = sanitize_name(path_name);
        self.inner
            .update_path_head(&norm, snapshot_hash)
            .map_err(pyerr)
    }

    /// Search DAG nodes by content words (case-insensitive), newest-first.
    /// Returns list of dicts [{"hash", "id", "ts"}].
    #[pyo3(signature = (query, limit=50))]
    fn dag_search_content(&self, py: Python<'_>, query: &str, limit: usize) -> PyResult<PyObject> {
        let v = self.inner.dag_search_content(query, limit).map_err(pyerr)?;
    Ok(json_array_to_py(py, &v))
    }
}

#[pyclass(name = "GateDecision")]
struct PyGateDecision {
    #[pyo3(get)]
    kind: &'static str,
    #[pyo3(get)]
    message: Option<String>,
}

#[pyclass(name = "StreamGate")]
struct PyStreamGate {
    #[allow(dead_code)]
    index: Arc<StreamingIndex>,
    gate: CoreStreamGate,
}



#[pymethods]
impl PyStreamGate {
    #[new]
    #[pyo3(
        signature = (contract_json, action, budget_ms=5_000, window_bytes=65_536, fail_closed_on_finalize=true)
    )]
    fn new(
        contract_json: &str,
        action: &str,
        budget_ms: u64,
        window_bytes: usize,
        fail_closed_on_finalize: bool,
    ) -> PyResult<Self> {
        let contract: MoralContract = serde_json::from_str(contract_json)
            .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))?;
        // If StreamingIndex::from_contract_for_action takes &MoralContract, pass &contract
        let index = StreamingIndex::from_contract_for_action(contract, action).map_err(pyerr)?;
        let arc = Arc::new(index);
        let gate = CoreStreamGate::from_index(
            arc.clone(),
            StreamGateConfig {
                budget_ms,
                window_bytes,
                fail_closed_on_finalize,
            },
        );
        Ok(Self { index: arc, gate })
    }

    /// Push a chunk of text to the stream gate. Returns a GateDecision (Pass/Hold/CutAndReplace).
    fn push(&mut self, chunk: &str) -> PyResult<PyGateDecision> {
        Ok(decision_to_py(self.gate.push(chunk)))
    }

    /// Finalize the stream. Returns a GateDecision (Pass/Hold/CutAndReplace).
    fn finalize(&mut self) -> PyResult<PyGateDecision> {
        Ok(decision_to_py(self.gate.finalize()))
    }
}

#[pymodule]
fn synaptik_core(py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    if let Err(e) = syn_core::commands::init::ensure_initialized_once() {
    let cat = py.get_type_bound::<pyo3::exceptions::PyRuntimeWarning>();
    PyErr::warn_bound(py, &cat, &format!("Init warning: {e}"), 0)?;
    }
    m.add_class::<PyCommands>()?;
    m.add_class::<PyStreamGate>()?;
    m.add_class::<PyGateDecision>()?;
    Ok(())
}
