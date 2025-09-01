use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};
use pyo3::exceptions::PyRuntimeError;
use pyo3::PyTypeInfo;

use ::synaptik_core as core;
use core::commands::Commands;

fn pyerr<E: std::fmt::Display>(e: E) -> PyErr {
    PyRuntimeError::new_err(e.to_string())
}

#[pyclass]
struct PyCommands {
    inner: Commands,
}

#[pymethods]
impl PyCommands {
    /// Construct using canonical .cogniv paths 
    #[new]
    fn new() -> PyResult<Self> {
        // Ensure the core is initialized (idempotent)
        core::commands::init::ensure_initialized_once().map_err(pyerr)?;
        let inner = Commands::new("ignored", None).map_err(pyerr)?;
        Ok(Self { inner })
    }

    #[pyo3(signature = (lobe, content, key=None))]
    fn remember(&self, lobe: &str, content: &str, key: Option<&str>) -> PyResult<String> {
        self.inner.remember(lobe, key, content).map_err(pyerr)
    }

    fn reflect(&self, lobe: &str, window: usize) -> PyResult<String> {
        self.inner.reflect(lobe, window).map_err(pyerr)
    }

    // Optional arg also needs explicit signature when other non-Python params exist
    #[pyo3(signature = (lobe=None))]
    fn stats(&self, lobe: Option<&str>, py: Python<'_>) -> PyResult<PyObject> {
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
    // Optional: warn if init fails (PyO3 0.22 warning API)
    if let Err(e) = core::commands::init::ensure_initialized_once() {
        use pyo3::exceptions::PyRuntimeWarning;
        let cat = PyRuntimeWarning::type_object_bound(py);
        PyErr::warn_bound(py, &cat, &format!("Init warning: {e}"), 0)?;
    }

    m.add_class::<PyCommands>()?;
    Ok(())
}
