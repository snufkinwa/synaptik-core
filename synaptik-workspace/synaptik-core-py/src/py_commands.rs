use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyList, PyMemoryView};

use anyhow::anyhow;
use blake3;
use chrono::Utc;
use serde_json::{json, Value};

use pyo3::buffer::PyBuffer;
use syn_core::commands::{Commands, EthosReport, HitSource, Prefer};
use syn_core::memory::dag::MemoryState as DagMemoryState;
use syn_core::services::ethos::{ContractsDecider, Proposal};
use syn_core::services::{FinalizedStatus, LlmClient, StreamRuntime};
use syn_core::utils::pons::ObjectMetadata as PonsMetadata;
use synaptik_core as syn_core;

use crate::py_helpers::{
    bind_json, gen_path_name, json_array_to_py, json_to_py, metadata_to_py, object_ref_to_py,
    path_exists_in_refs, py_to_json, pyerr, sanitize_name,
};

#[pyclass]
pub struct PyCommands {
    pub(crate) inner: Commands,
    pub(crate) last_recalled: Option<String>,
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
    #[pyo3(signature = (intent, input))]
    fn govern_text(&self, py: Python<'_>, intent: &str, input: &str) -> PyResult<PyObject> {
        struct EchoStream {
            yielded: bool,
            text: String,
        }
        impl Iterator for EchoStream {
            type Item = String;
            fn next(&mut self) -> Option<Self::Item> {
                if self.yielded {
                    None
                } else {
                    self.yielded = true;
                    Some(self.text.clone())
                }
            }
        }
        struct EchoModel {
            text: String,
        }
        impl LlmClient for EchoModel {
            type Stream = EchoStream;
            fn stream(
                &self,
                _system_prompt: String,
            ) -> Result<Self::Stream, syn_core::services::GateError> {
                Ok(EchoStream {
                    yielded: false,
                    text: self.text.clone(),
                })
            }
        }

        let proposal = Proposal {
            intent: intent.to_string(),
            input: input.to_string(),
            prior: None,
            tools_requested: vec![],
        };
        let contract = ContractsDecider;
        let model = EchoModel {
            text: input.to_string(),
        };
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
        extra: Option<PyObject>,
    ) -> PyResult<PyObject> {
        let mv: PyBuffer<u8> = PyBuffer::get_bound(data)?;
        let slice =
            unsafe { std::slice::from_raw_parts(mv.buf_ptr() as *const u8, mv.item_count()) };
        let bytes = slice.to_vec();
        let extra_v = extra.as_ref().map(|o| {
            // Attempt structured conversion; fallback to Null to preserve type expectations.
            let bound = o.bind(py);
            match py_to_json(&bound) {
                v @ Value::Null => v, // already Null
                v => v,
            }
        });
        let obj = self
            .inner
            .pons_put_object(pons, key, &bytes, media_type, extra_v)
            .map_err(pyerr)?;
        Ok(crate::py_helpers::json_to_py(
            py,
            &json!({
                "pons": obj.pons,
                "key": obj.key,
                "version": obj.version,
                "etag": obj.etag,
                "size_bytes": obj.size_bytes,
            }),
        ))
    }

    fn pons_get_latest_bytes(&self, py: Python<'_>, pons: &str, key: &str) -> PyResult<PyObject> {
        let bytes = self.inner.pons_get_latest_bytes(pons, key).map_err(pyerr)?;
        let pybytes = PyBytes::new_bound(py, &bytes);
        Ok(PyMemoryView::from_bound(&pybytes.as_any())?.into_py(py))
    }

    fn pons_get_latest_ref(&self, py: Python<'_>, pons: &str, key: &str) -> PyResult<PyObject> {
        let r = self.inner.pons_get_latest_ref(pons, key).map_err(pyerr)?;
        Ok(object_ref_to_py(py, &r))
    }

    fn pons_get_version_with_meta(
        &self,
        py: Python<'_>,
        pons: &str,
        key: &str,
        version: &str,
    ) -> PyResult<(PyObject, PyObject)> {
        let pons_s = pons.to_string();
        let key_s = key.to_string();
        let ver_s = version.to_string();
        // Perform only pure Rust/domain work while the GIL is released. No PyErr creation here.
        let domain_result = py.allow_threads(move || -> anyhow::Result<(Vec<u8>, PonsMetadata)> {
            let report = syn_core::commands::init::ensure_initialized_once()?;
            let store = syn_core::utils::pons::PonsStore::open(&report.root)?;
            let (bytes, meta) = store.get_object_version_with_meta(&pons_s, &key_s, &ver_s)?;
            Ok((bytes, meta))
        });
        let (bytes, meta) = domain_result.map_err(pyerr)?;
        let pybytes = PyBytes::new_bound(py, &bytes);
        let mv = PyMemoryView::from_bound(&pybytes.as_any())?;
        Ok((mv.into_py(py), metadata_to_py(py, &meta)))
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

    #[pyo3(signature = (lobe=None))]
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

    #[pyo3(signature = (snapshot_id, path_name=None))]
    fn diverge_from(&self, snapshot_id: &str, path_name: Option<&str>) -> PyResult<String> {
        let name = path_name
            .map(sanitize_name)
            .unwrap_or_else(|| gen_path_name("replay", snapshot_id));
        self.inner
            .replay_diverge_from(snapshot_id, &name)
            .map_err(pyerr)
    }

    #[pyo3(signature = (path_name, content, meta=None))]
    fn extend_path(
        &mut self,
        py: Python<'_>,
        path_name: &str,
        content: &str,
        meta: Option<PyObject>,
    ) -> PyResult<String> {
        let norm = sanitize_name(path_name);
        if !path_exists_in_refs(&norm).map_err(pyerr)? {
            let base = self.last_recalled.clone().ok_or_else(|| {
                pyerr(anyhow!(format!(
                    "Path '{}' not found and no prior recall to seed from.",
                    path_name
                )))
            })?;
            let _ = self
                .inner
                .replay_diverge_from(&base, &norm)
                .map_err(pyerr)?;
        }
        let mut meta_value: Value = if let Some(obj) = meta {
            let bound = obj.bind(py);
            py_to_json(&bound)
        } else {
            json!({})
        };
        let content_hash = blake3::hash(content.as_bytes()).to_hex().to_string();
        let now = Utc::now().to_rfc3339();
        let enrich = json!({ "op": "extend_path", "ts": now, "actor": "python", "content_hash": content_hash, "path_name": path_name });
        meta_value = bind_json(enrich, meta_value);
        let state = DagMemoryState {
            content: content.to_string(),
            meta: meta_value,
        };
        let new_id = self.inner.replay_extend_path(&norm, state).map_err(pyerr)?;
        Ok(new_id)
    }

    fn snapshot_meta(&self, py: Python<'_>, snapshot_id: &str) -> PyResult<PyObject> {
        Ok(json_to_py(
            py,
            &self.inner.dag_snapshot_meta(snapshot_id).map_err(pyerr)?,
        ))
    }

    #[pyo3(signature = (path_name, limit=50))]
    fn trace_path(&self, py: Python<'_>, path_name: &str, limit: usize) -> PyResult<PyObject> {
        let norm = sanitize_name(path_name);
        let v = self.inner.dag_trace_path(&norm, limit).map_err(pyerr)?;
        Ok(json_array_to_py(py, &v))
    }

    fn cite_sources(&self, py: Python<'_>, snapshot_id: &str) -> PyResult<PyObject> {
        let v = self.inner.dag_cite_sources(snapshot_id).map_err(pyerr)?;
        Ok(json_array_to_py(py, &v))
    }

    fn last_recalled_id(&self) -> Option<String> {
        self.last_recalled.clone()
    }

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

    fn seed_base_from_lobe(&self, lobe: &str) -> PyResult<Option<String>> {
        self.inner.replay_base_from_lobe(lobe).map_err(pyerr)
    }

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

    #[pyo3(signature = (path, base=None, lobe=None))]
    fn branch(&self, path: &str, base: Option<&str>, lobe: Option<&str>) -> PyResult<String> {
        let norm = sanitize_name(path);
        self.inner.branch(&norm, base, lobe).map_err(pyerr)
    }

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

    fn dag_head(&self, path_name: &str) -> PyResult<Option<String>> {
        self.inner
            .dag_head(&sanitize_name(path_name))
            .map_err(pyerr)
    }

    fn update_path_head(&self, path_name: &str, snapshot_hash: &str) -> PyResult<()> {
        self.inner
            .update_path_head(&sanitize_name(path_name), snapshot_hash)
            .map_err(pyerr)
    }

    fn engram_head(&self, path_name: &str) -> PyResult<Option<String>> {
        self.inner
            .dag_head(&sanitize_name(path_name))
            .map_err(pyerr)
    }

    fn set_engram_head(&self, path_name: &str, snapshot_hash: &str) -> PyResult<()> {
        self.inner
            .update_path_head(&sanitize_name(path_name), snapshot_hash)
            .map_err(pyerr)
    }

    #[pyo3(signature = (query, limit=50))]
    fn dag_search_content(&self, py: Python<'_>, query: &str, limit: usize) -> PyResult<PyObject> {
        let v = self.inner.dag_search_content(query, limit).map_err(pyerr)?;
        Ok(json_array_to_py(py, &v))
    }
}
