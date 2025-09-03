// Public modules so synaptik-core can use them
pub mod types;
pub mod evaluator;
pub mod assets;

pub use evaluator::{evaluate_input_against_rules, load_contract_from_file, EvaluationResult};
pub use types::MoralContract;
pub use assets::{default_contract_text, write_default_contracts};

/// --- Pure Rust API for synaptik-core ---
pub fn evaluate_contract_json(json_contract: &str, message: &str)
    -> Result<EvaluationResult, serde_json::Error>
{
    let contract: MoralContract = serde_json::from_str(json_contract)?;
    Ok(evaluate_input_against_rules(message, &contract))
}

/// --- WASM entrypoint (only compiled when feature "wasm" is enabled) ---
#[cfg(feature = "wasm")]
mod wasm_api {
    use super::*;
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen]
    pub fn evaluate_contract(json_contract: &str, message: &str) -> String {
        match super::evaluate_contract_json(json_contract, message) {
            Ok(res) => serde_json::to_string(&res)
                .unwrap_or_else(|_| "Serialization failed".to_string()),
            Err(_)  => "Invalid contract JSON".to_string(),
        }
    }
}
