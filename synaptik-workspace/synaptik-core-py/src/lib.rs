use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};
use pyo3::PyTypeInfo;

use ::synaptik_core as core;
use core::commands::{Commands, EthosReport};

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
        core::commands::init::ensure_initialized_once().map_err(pyerr)?;
        let inner = Commands::new("ignored", None).map_err(pyerr)?;
        Ok(Self { inner })
    }

    fn precheck_text(&self, py: Python<'_>, text: &str, purpose: &str) -> PyResult<PyObject> {
        let rep: EthosReport = self.inner.precheck_text(text, purpose).map_err(pyerr)?;
        let d = PyDict::new_bound(py);
        d.set_item("decision", rep.decision)?;
        d.set_item("reason", rep.reason)?;
        d.set_item("risk", rep.risk)?;
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

    fn recall(&self, memory_id: &str) -> PyResult<Option<String>> {
        self.inner.recall(memory_id).map_err(pyerr)
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

    fn root(&self) -> PyResult<String> {
        let rep = core::commands::init::ensure_initialized_once().map_err(pyerr)?;
        Ok(rep.root.to_string_lossy().to_string())
    }
}

#[pymodule]
fn synaptik_core(py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    if let Err(e) = core::commands::init::ensure_initialized_once() {
        let cat = pyo3::exceptions::PyRuntimeWarning::type_object_bound(py);
        PyErr::warn_bound(py, &cat, &format!("Init warning: {e}"), 0)?;
    }
    m.add_class::<PyCommands>()?;
    Ok(())
}
