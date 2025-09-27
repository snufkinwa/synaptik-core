// Public modules so synaptik-core can use them
pub mod api;
#[cfg(not(target_arch = "wasm32"))]
pub mod assets;
pub mod capsule;
pub mod evaluator;
pub mod normalize;
pub mod patch;
#[cfg(not(target_arch = "wasm32"))]
pub mod store;
pub mod types;

pub use api::{CapsAnnot, CapsId, Contract, Denied, PatchId, Purpose, Verdict};
#[cfg(not(target_arch = "wasm32"))]
pub use assets::{default_contract_text, write_default_contracts};
pub use capsule::{ArtifactRef, CapsuleMeta, CapsuleSource, SimCapsule};
pub use evaluator::{evaluate_input_against_rules, EvaluationResult};
#[cfg(not(target_arch = "wasm32"))]
pub use evaluator::load_contract_from_file;
pub use normalize::for_rules as normalize_for_rules;
pub use patch::{PatchOp, PatchPlan};
#[cfg(not(target_arch = "wasm32"))]
pub use store::{CapsHandle, ContractsStore};
pub use types::MoralContract;

// Experimental host-side sandbox runner (host-only)
#[cfg(feature = "wasm_host")]
pub mod sandbox;
#[cfg(feature = "wasm_host")]
pub use sandbox::run_wasm_contract;

/// --- Pure Rust API for synaptik-core ---
pub fn evaluate_contract_json(
    json_contract: &str,
    message: &str,
) -> Result<EvaluationResult, serde_json::Error> {
    let contract: MoralContract = serde_json::from_str(json_contract)?;
    Ok(evaluate_input_against_rules(message, &contract))
}

/// --- WASM entrypoint (guest side; compiled for wasm32 when wasm_guest enabled) ---
#[cfg(all(feature = "wasm_guest", target_arch = "wasm32"))]
mod wasm_api {
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen]
    pub fn evaluate_contract(json_contract: &str, message: &str) -> String {
        match super::evaluate_contract_json(json_contract, message) {
            Ok(res) => {
                serde_json::to_string(&res).unwrap_or_else(|_| "Serialization failed".to_string())
            }
            Err(_) => "Invalid contract JSON".to_string(),
        }
    }
}
