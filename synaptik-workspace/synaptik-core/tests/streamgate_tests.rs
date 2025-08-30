use synaptik_core::services::{StreamGate, StreamingIndex, StreamGateConfig, GateDecision};
use contracts::types::{MoralContract, ContractRule};

fn create_test_contract() -> MoralContract {
    MoralContract {
        name: "Test Contract".to_string(),
        version: "1.0".to_string(),
        description: Some("Test contract for streaming gate".to_string()),
        rules: vec![
            // Allow rule for idioms
            ContractRule {
                action: "say".to_string(),
                contains: vec![], // Legacy field
                contains_any: None,
                matches_any: Some(vec![
                    "kill time".to_string(),
                    "killing it".to_string(),
                    "i'd kill for a slice of pizza".to_string()
                ]),
                violation: "idiom_usage".to_string(),
                action_suggestion: None,
                violation_code: Some("IDIOM_ALLOW".to_string()),
                effect: Some("allow".to_string()),
                severity: Some("none".to_string()),
            },
            // High severity personal threats - only matches when pronouns present
            ContractRule {
                action: "say".to_string(),
                contains: vec![], // Legacy field
                contains_any: Some(vec!["kill him".to_string(), "kill her".to_string(), "hurt them".to_string()]),
                matches_any: None,
                violation: "personal_harm_threat".to_string(),
                action_suggestion: Some("I don't want to harm anyone. Let's talk about what's bothering me.".to_string()),
                violation_code: Some("HARM_THREAT".to_string()),
                effect: None,
                severity: Some("high".to_string()),
            },
            // Medium severity general harm - matches general patterns
            ContractRule {
                action: "say".to_string(),
                contains: vec![], // Legacy field  
                contains_any: Some(vec!["kill".to_string(), "destroy".to_string()]),
                matches_any: None,
                violation: "general_harm_content".to_string(),
                action_suggestion: Some("Let's discuss this in a constructive way.".to_string()),
                violation_code: Some("GENERAL_HARM".to_string()),
                effect: None,
                severity: Some("medium".to_string()),
            },
        ]
    }
}

fn gate_from_contract(contract: MoralContract, action: &str) -> StreamGate {
    let index = StreamingIndex::from_contract_for_action(contract, action).unwrap();
    StreamGate::from_index(
        std::sync::Arc::new(index),
        StreamGateConfig { 
            budget_ms: 50, 
            window_bytes: 4096, 
            fail_closed_on_finalize: true 
        },
    )
}

#[test]
fn idioms_pass() {
    let contract = create_test_contract();
    let mut g = gate_from_contract(contract, "say");
    assert!(matches!(g.push("I'm killing it today"), GateDecision::Pass));
    
    let contract2 = create_test_contract();
    let mut g2 = gate_from_contract(contract2, "say");
    assert!(matches!(g2.push("I'd kill for a slice of pizza"), GateDecision::Pass));
}

#[test]
fn personal_threat_cuts() {
    let contract = create_test_contract();
    let mut g = gate_from_contract(contract, "say");
    match g.push("I'm going to kill him") {
        GateDecision::CutAndReplace(_) => {}
        _ => panic!("should cut for personal threat"),
    }
}

#[test]
fn generic_harm_holds_then_cuts_on_finalize() {
    let contract = create_test_contract();
    let mut g = gate_from_contract(contract, "say");
    
    // This should match the medium severity "general_harm_content" rule
    match g.push("I want to kill") {
        GateDecision::Hold => println!("✓ Correctly held for medium severity"),
        other => panic!("Expected Hold, got {:?}", other),
    }
    
    match g.finalize() {
        GateDecision::CutAndReplace(_) => println!("✓ Correctly cut at finalize"),
        other => panic!("Expected CutAndReplace at finalize, got {:?}", other),
    }
}

#[test]
fn debug_generic_harm() {
    // Debug test to see what's happening
    let contract = create_test_contract();
    let index = StreamingIndex::from_contract_for_action(contract, "say").unwrap();
    
    // Test the evaluation directly
    let evaluation = index.evaluate_chunk("I want to kill");
    println!("Evaluation passed: {}", evaluation.passed);
    println!("Violated rules count: {}", evaluation.violated_rules.len());
    
    for (i, rule) in evaluation.violated_rules.iter().enumerate() {
        println!("Rule {}: {} (severity: {:?})", i, rule.violation, rule.severity);
    }
    
    // Test pronoun detection
    let has_pronouns = index.has_pronouns("I want to kill");
    println!("Has pronouns: {}", has_pronouns);
    
    assert!(!evaluation.passed, "Should detect violation");
    assert!(!evaluation.violated_rules.is_empty(), "Should have violated rules");
}