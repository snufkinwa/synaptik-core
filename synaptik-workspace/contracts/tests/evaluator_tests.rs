use contracts::evaluator::{evaluate_input_against_rules, load_contract_from_file};
use contracts::types::MoralContract;

fn setup_nonviolence_contract() -> MoralContract {
    load_contract_from_file("./nonviolence.toml")
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
fn test_personal_threat_violation() {
    let contract = setup_nonviolence_contract();
    let result = evaluate_input_against_rules("I'm going to hurt someone.", &contract);
    assert!(!result.passed);
    assert_eq!(result.violated_rules.len(), 1);
    assert_eq!(result.primary_violation_code, Some("harm_threat_personal".to_string()));
}

#[test]
fn test_unspecified_violence_violation() {
    let contract = setup_nonviolence_contract();
    let result = evaluate_input_against_rules("I want to destroy all the things.", &contract);
    assert!(!result.passed);
    assert_eq!(result.violated_rules.len(), 1);
    assert_eq!(result.primary_violation_code, Some("harm_language_generic".to_string()));
}

#[test]
fn test_violent_tool_suggestion() {
    let contract = setup_nonviolence_contract();
    let result = evaluate_input_against_rules("You should stab them.", &contract);
    assert!(!result.passed);
    assert_eq!(result.violated_rules.len(), 1);
    assert_eq!(result.primary_violation_code, Some("violent_suggestion".to_string()));
}

#[test]
fn test_justification_of_violence() {
    let contract = setup_nonviolence_contract();
    let result = evaluate_input_against_rules("They deserved it when they got hit.", &contract);
    assert!(!result.passed);
    assert_eq!(result.violated_rules.len(), 1);
    assert_eq!(result.primary_violation_code, Some("justification_of_violence".to_string()));
}