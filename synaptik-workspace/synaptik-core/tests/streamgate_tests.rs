use contracts::assets::NONVIOLENCE_TOML;
use contracts::types::MoralContract;
use synaptik_core::services::{GateDecision, StreamGate, StreamGateConfig, StreamingIndex};

fn default_contract() -> MoralContract {
    toml::from_str(NONVIOLENCE_TOML).expect("parse embedded nonviolence contract")
}

fn gate_from_contract(contract: MoralContract, action: &str) -> StreamGate {
    let index = StreamingIndex::from_contract_for_action(contract, action).unwrap();
    StreamGate::from_index(
        std::sync::Arc::new(index),
        StreamGateConfig {
            budget_ms: 50,
            window_bytes: 4096,
            fail_closed_on_finalize: true,
        },
    )
}

#[test]
fn idioms_pass() {
    let contract = default_contract();
    let mut g = gate_from_contract(contract, "say");
    assert!(matches!(g.push("I'm killing it today"), GateDecision::Pass));

    let contract2 = default_contract();
    let mut g2 = gate_from_contract(contract2, "say");
    assert!(matches!(
        g2.push("I'd kill for a slice of pizza"),
        GateDecision::Pass
    ));
}

#[test]
fn personal_threat_cuts() {
    let contract = default_contract();
    let mut g = gate_from_contract(contract, "say");
    match g.push("I'm going to kill him") {
        GateDecision::CutAndReplace(_) => {}
        _ => panic!("should cut for personal threat"),
    }
}

#[test]
fn generic_harm_holds_then_cuts_on_finalize() {
    let contract = default_contract();
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
    let contract = default_contract();
    let index = StreamingIndex::from_contract_for_action(contract, "say").unwrap();

    // Test the evaluation directly
    let evaluation = index.evaluate_chunk("I want to kill");
    println!("Evaluation passed: {}", evaluation.passed);
    println!("Violated rules count: {}", evaluation.violated_rules.len());

    for (i, rule) in evaluation.violated_rules.iter().enumerate() {
        println!(
            "Rule {}: {} (severity: {:?})",
            i, rule.violation, rule.severity
        );
    }

    // Test pronoun detection
    let has_pronouns = index.has_pronouns("I want to kill");
    println!("Has pronouns: {}", has_pronouns);

    assert!(!evaluation.passed, "Should detect violation");
    assert!(
        !evaluation.violated_rules.is_empty(),
        "Should have violated rules"
    );
}
