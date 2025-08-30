use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct MoralContract {
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub rules: Vec<ContractRule>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ContractRule {
    pub action: String,
    #[serde(default)]
    pub contains: Vec<String>,  // Legacy field for backward compatibility
    #[serde(default)]
    pub contains_any: Option<Vec<String>>,  // New field for keywords
    #[serde(default)]
    pub matches_any: Option<Vec<String>>,   // New field for exact phrases
    pub violation: String,
    pub action_suggestion: Option<String>,
    pub violation_code: Option<String>,
    #[serde(default)]
    pub effect: Option<String>,  // "allow" for whitelist rules
    #[serde(default)]
    pub severity: Option<String>,  // "none", "low", "medium", "high"
}