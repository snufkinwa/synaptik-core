use contracts::evaluator::{evaluate_input_against_rules, load_contract_from_file};
use contracts::types::MoralContract;

fn setup_nonviolence_contract() -> MoralContract {
    load_contract_from_file("./assets/nonviolence.toml")
}

#[test]
fn test_no_violation() {
    let contract = setup_nonviolence_contract();
    let result = evaluate_input_against_rules("Hello, how are you?", &contract);
    assert!(result.passed);
    assert_eq!(result.violated_rules.len(), 0);
}

#[test]
fn test_idiom_whitelist() {
    let contract = setup_nonviolence_contract();
    let result = evaluate_input_against_rules("I'd kill for a slice of pizza", &contract);
    assert!(result.passed);
    assert_eq!(result.violated_rules.len(), 0);
}

#[test]
fn test_personal_threat_violation_falls_back_to_generic() {
    let contract = setup_nonviolence_contract();
    let result = evaluate_input_against_rules("I'm going to hurt someone.", &contract);
    assert!(!result.passed);
    assert_eq!(result.violated_rules.len(), 1);
    assert_eq!(
        result.primary_violation_code.as_deref(),
        Some("harm_language_generic")
    );
    assert_eq!(
        result.action_suggestion.as_deref(),
        Some("Let’s choose constructive wording.")
    );
}

#[test]
fn test_unspecified_violence_violation() {
    let contract = setup_nonviolence_contract();
    let result = evaluate_input_against_rules("I want to destroy all the things.", &contract);
    assert!(!result.passed);
    assert_eq!(result.violated_rules.len(), 1);
    assert_eq!(
        result.primary_violation_code.as_deref(),
        Some("harm_language_generic")
    );
    assert_eq!(
        result.action_suggestion.as_deref(),
        Some("Let’s choose constructive wording.")
    );
}

#[test]
fn test_violent_tool_suggestion() {
    let contract = setup_nonviolence_contract();
    let result = evaluate_input_against_rules("You should stab them.", &contract);
    assert!(!result.passed);
    assert_eq!(result.violated_rules.len(), 1);
    assert_eq!(
        result.primary_violation_code.as_deref(),
        Some("violent_suggestion")
    );
    assert_eq!(
        result.action_suggestion.as_deref(),
        Some("I can’t help with violent methods.")
    );
}

#[test]
fn test_justification_of_violence_with_constraints() {
    let contract = load_contract_from_file("./assets/nonviolence.toml");
    let result = evaluate_input_against_rules("They had it coming.", &contract);
    assert!(!result.passed);
    assert_eq!(
        result.primary_violation_code.as_deref(),
        Some("justification_of_violence")
    );
    // constraints should include our new ones (order not guaranteed)
    let cs = result.constraints;
    assert!(cs.iter().any(|c| c == "reframe_nonviolent"));
    assert!(cs.iter().any(|c| c == "offer_deescalation"));
    assert!(cs.iter().any(|c| c == "avoid_justification_language"));
}
