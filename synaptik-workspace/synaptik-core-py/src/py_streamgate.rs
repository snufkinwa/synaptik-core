use pyo3::prelude::*;

use std::sync::Arc;

use contracts::types::MoralContract;
use synaptik_core as syn_core;
use syn_core::services::streamgate::{GateDecision as CoreGateDecision, StreamGate as CoreStreamGate, StreamGateConfig, StreamingIndex};

use crate::py_helpers::pyerr;

#[pyclass(name = "GateDecision")]
pub struct PyGateDecision {
    #[pyo3(get)]
    pub(crate) kind: &'static str,
    #[pyo3(get)]
    pub(crate) message: Option<String>,
}

fn decision_to_py(decision: CoreGateDecision) -> PyGateDecision {
    match decision {
        CoreGateDecision::Pass => PyGateDecision { kind: "Pass", message: None },
        CoreGateDecision::Hold => PyGateDecision { kind: "Hold", message: None },
        CoreGateDecision::CutAndReplace(msg) => PyGateDecision { kind: "CutAndReplace", message: Some(msg) },
    }
}

#[pyclass(name = "StreamGate")]
pub struct PyStreamGate {
    #[allow(dead_code)]
    pub(crate) index: Arc<StreamingIndex>,
    pub(crate) gate: CoreStreamGate,
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
        let index = StreamingIndex::from_contract_for_action(contract, action).map_err(pyerr)?;
        let arc = Arc::new(index);
        let gate = CoreStreamGate::from_index(
            arc.clone(),
            StreamGateConfig { budget_ms, window_bytes, fail_closed_on_finalize },
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

