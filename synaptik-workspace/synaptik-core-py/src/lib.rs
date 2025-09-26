use pyo3::prelude::*;
use ::synaptik_core::commands::init;

mod py_helpers;
mod py_streamgate;
mod py_commands;

pub use py_commands::PyCommands;
pub use py_streamgate::{PyGateDecision, PyStreamGate};

#[pymodule]
fn synaptik_core(py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    if let Err(e) = init::ensure_initialized_once() {
        let cat = py.get_type_bound::<pyo3::exceptions::PyRuntimeWarning>();
        PyErr::warn_bound(py, &cat, &format!("Init warning: {e}"), 0)?;
    }
    m.add_class::<PyCommands>()?;
    m.add_class::<PyStreamGate>()?;
    m.add_class::<PyGateDecision>()?;
    Ok(())
}
