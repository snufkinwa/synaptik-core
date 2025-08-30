// synaptik-core-py/src/lib.rs
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};
use pyo3::exceptions::PyRuntimeWarning;
use synaptik_core as core; 

#[pyfunction]
fn list_memories(limit: Option<usize>, py: Python<'_>) -> PyResult<PyObject> {
    // Ensure the core is initialized (idempotent)
    let _ = core::commands::init::ensure_initialized_once();

    let items = core::commands::reflect::list_memories(limit)
        .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;

    let out = PyList::empty(py);
    for it in items {
        let d = PyDict::new(py);
        d.set_item("id", it.id)?;
        d.set_item("path", it.path.to_string_lossy().to_string())?;
        d.set_item("timestamp", it.timestamp)?;
        d.set_item("preview", it.preview)?;
        out.append(d)?;
    }
    Ok(out.into())
}

#[pyfunction]
fn read_memory(id_or_path: &str) -> PyResult<String> {
    // Optional: ensure init here too
    let _ = core::commands::init::ensure_initialized_once();

    core::commands::reflect::read_memory(id_or_path)
        .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))
}

#[pymodule]
fn synaptik_core_py(py: Python<'_>, m: &PyModule) -> PyResult<()> {
    // Auto-init once on import; warn (don’t fail) if it has issues
    if let Err(e) = core::commands::init::ensure_initialized_once() {
        PyRuntimeWarning::warn(py, &format!("Init warning: {}", e), 0)?;
    }
    m.add_function(wrap_pyfunction!(list_memories, m)?)?;
    m.add_function(wrap_pyfunction!(read_memory, m)?)?;
    Ok(())
}
