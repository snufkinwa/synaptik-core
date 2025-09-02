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

    // Legacy field — still supported
    #[serde(default)]
    pub contains: Vec<String>,

    // Newer fields — keyword & exact-phrase matching
    #[serde(default)]
    pub contains_any: Option<Vec<String>>,
    #[serde(default)]
    pub matches_any: Option<Vec<String>>,

    // Violation metadata
    pub violation: String,
    pub action_suggestion: Option<String>,
    pub violation_code: Option<String>,

    // "allow" or "allow_with_constraints" for whitelist / soft pass
    #[serde(default)]
    pub effect: Option<String>,

    // "none" | "low" | "medium" | "high" | "critical"
    #[serde(default)]
    pub severity: Option<String>,

    // NEW: optional soft guidance tags (merged into the result)
    #[serde(default)]
    pub constraints: Option<Vec<String>>,
}
