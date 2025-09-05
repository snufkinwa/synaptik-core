use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};
use pyo3::PyTypeInfo;

extern crate synaptik_core as syn_core;
use syn_core::commands::{Commands, EthosReport, Prefer, HitSource};

fn pyerr<E: std::fmt::Display>(e: E) -> PyErr {
    pyo3::exceptions::PyRuntimeError::new_err(e.to_string())
}

#[pyclass]
struct PyCommands {
    inner: Commands,
}

#[pymethods]
impl PyCommands {
    #[new]
    fn new() -> PyResult<Self> {
        syn_core::commands::init::ensure_initialized_once().map_err(pyerr)?;
        let inner = Commands::new("ignored", None).map_err(pyerr)?;
        Ok(Self { inner })
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
    fn recall(&self, py: Python<'_>, memory_id: &str, prefer: Option<&str>) -> PyResult<Option<PyObject>> {
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
                let src = match hit.source { HitSource::Hot => "hot", HitSource::Archive => "archive", HitSource::Dag => "dag" };
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
            let src = match r.source { HitSource::Hot => "hot", HitSource::Archive => "archive", HitSource::Dag => "dag" };
            d.set_item("source", src)?;
            out.append(d)?;
        }
        Ok(out.into_any().into_py(py))
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

    // Note: duplicate pruning is automated in the Rust core during writes.

   
    fn root(&self) -> PyResult<String> {
        let rep = syn_core::commands::init::ensure_initialized_once().map_err(pyerr)?;
        Ok(rep.root.to_string_lossy().to_string())
    }

}

#[pymodule]
fn synaptik_core(py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    if let Err(e) = syn_core::commands::init::ensure_initialized_once() {
        let cat = pyo3::exceptions::PyRuntimeWarning::type_object_bound(py);
        PyErr::warn_bound(py, &cat, &format!("Init warning: {e}"), 0)?;
    }
    m.add_class::<PyCommands>()?;
    Ok(())
}
