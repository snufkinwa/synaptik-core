use crate::types::{MoralContract, ContractRule};
use serde::Serialize;
use std::fs;
use toml;

#[derive(Debug, Clone, Serialize)]
pub struct EvaluationResult {
    pub passed: bool,
    pub violated_rules: Vec<ContractRule>,
    pub reason: String,
    pub primary_violation_code: Option<String>,
    pub action_suggestion: Option<String>,
}

pub fn load_contract_from_file(path: &str) -> MoralContract {
    let content = fs::read_to_string(path).expect("Failed to read contract file");
    toml::from_str(&content).expect("Failed to parse TOML")
}

pub fn evaluate_input_against_rules(input: &str, contract: &MoralContract) -> EvaluationResult {
    let normalized_input = input.to_lowercase();
    let mut violations = Vec::new();
    
    // First pass: check for allow/whitelist rules (these take precedence)
    for rule in &contract.rules {
        // Check if this rule has an "allow" effect or severity "none"
        let is_allow_rule = rule.effect.as_deref() == Some("allow") || 
                           rule.severity.as_deref() == Some("none");
        
        if is_allow_rule {
            let mut found_match = false;
            
            // Check for exact phrase matches first (more specific)
            if let Some(matches) = &rule.matches_any {
                for phrase in matches {
                    if normalized_input.contains(&phrase.to_lowercase()) {
                        found_match = true;
                        break;
                    }
                }
            }
            
            // Check for keyword matches
            if !found_match {
                if let Some(keywords) = &rule.contains_any {
                    for keyword in keywords {
                        if normalized_input.contains(&keyword.to_lowercase()) {
                            found_match = true;
                            break;
                        }
                    }
                }
            }
            
            // If this allow rule matches, the input passes
            if found_match {
                return EvaluationResult {
                    passed: true,
                    violated_rules: vec![],
                    reason: "Input matches allowlisted pattern.".into(),
                    primary_violation_code: None,
                    action_suggestion: None,
                };
            }
        }
    }
    
    // Second pass: check for violation rules
    for rule in &contract.rules {
        // Skip allow rules - we already processed them
        let is_allow_rule = rule.effect.as_deref() == Some("allow") || 
                           rule.severity.as_deref() == Some("none");
        if is_allow_rule {
            continue;
        }
        
        let mut found_match = false;
        
        // Check for keyword matches in 'contains_any'
        if let Some(keywords) = &rule.contains_any {
            for keyword in keywords {
                if normalized_input.contains(&keyword.to_lowercase()) {
                    found_match = true;
                    break;
                }
            }
        }
        
        // Check for exact phrase matches in 'matches_any'
        if !found_match {
            if let Some(matches) = &rule.matches_any {
                for phrase in matches {
                    if normalized_input.contains(&phrase.to_lowercase()) {
                        found_match = true;
                        break;
                    }
                }
            }
        }
        
        // Add rule to violations if a match was found
        if found_match {
            violations.push(rule.clone());
        }
    }
    
    if violations.is_empty() {
        EvaluationResult {
            passed: true,
            violated_rules: vec![],
            reason: "No violations detected.".into(),
            primary_violation_code: None,
            action_suggestion: None,
        }
    } else {
        let first = &violations[0];
        EvaluationResult {
            passed: false,
            violated_rules: violations.clone(),
            reason: format!("Violated {} rule(s).", violations.len()),
            primary_violation_code: first.violation_code.clone(),
            action_suggestion: first.action_suggestion.clone(),
        }
    }
}