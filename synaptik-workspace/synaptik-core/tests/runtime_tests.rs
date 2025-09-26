use synaptik_core::services::{
    EthosContract, FinalizedStatus, LlmClient, Proposal, RuntimeDecision, StreamRuntime,
};
use synaptik_core::services::ConstraintSpec;

// ----------------------- Test stubs -----------------------

#[derive(Clone)]
struct FixedContract {
    spec: Option<ConstraintSpec>,
}

impl EthosContract for FixedContract {
    fn evaluate(&self, _p: &Proposal) -> RuntimeDecision {
        match &self.spec {
            Some(s) => RuntimeDecision::Constrain(s.clone()),
            None => RuntimeDecision::Proceed,
        }
    }
}

struct VecStream {
    idx: usize,
    toks: Vec<String>,
}

impl Iterator for VecStream {
    type Item = String;
    fn next(&mut self) -> Option<Self::Item> {
        if self.idx >= self.toks.len() {
            None
        } else {
            let s = self.toks[self.idx].clone();
            self.idx += 1;
            Some(s)
        }
    }
}

struct VecModel {
    toks: Vec<String>,
}

impl LlmClient for VecModel {
    type Stream = VecStream;
    fn stream(&self, _system_prompt: String) -> Result<Self::Stream, synaptik_core::services::GateError> {
        Ok(VecStream { idx: 0, toks: self.toks.clone() })
    }
}

fn proposal(intent: &str, input: &str) -> Proposal {
    Proposal {
        intent: intent.to_string(),
        input: input.to_string(),
        prior: None,
        tools_requested: vec![],
    }
}

// ----------------------- Tests ----------------------------

#[test]
fn mask_rules_redact_case_insensitive() {
    let spec = ConstraintSpec {
        mask_rules: vec!["kill".into()],
        allow_tools: vec![],
        stop_phrases: vec![],
        max_tokens: 256,
        temperature_cap: 0.7,
    };
    let contract = FixedContract { spec: Some(spec) };
    let model = VecModel { toks: vec!["I will ".into(), "KiLl ".into(), "time".into()] };
    let rt = StreamRuntime { contract, model };

    let res = rt.generate(proposal("memory_storage", "I will kill time")).expect("generate");
    assert_eq!(res.status, FinalizedStatus::Ok);
    assert!(res.text.contains("[masked]"), "masked output should contain [masked]");
    assert!(
        !res.text.to_ascii_lowercase().contains("kill"),
        "masked pattern should not appear"
    );
}

#[test]
fn mask_rules_redact_across_token_boundaries() {
    let spec = ConstraintSpec {
        mask_rules: vec!["secret".into()],
        allow_tools: vec![],
        stop_phrases: vec![],
        max_tokens: 256,
        temperature_cap: 0.7,
    };
    let contract = FixedContract { spec: Some(spec) };
    // Split the sensitive phrase across tokens to ensure cross-boundary masking.
    let model = VecModel { toks: vec![
        "the ".into(),
        "se".into(),
        "Cr".into(),
        "Et ".into(),
        "code".into(),
    ] };
    let rt = StreamRuntime { contract, model };

    let res = rt.generate(proposal("memory_storage", "irrelevant"))
        .expect("generate");
    assert_eq!(res.status, FinalizedStatus::Ok);
    assert!(res.text.contains("[masked]"), "masked output should contain [masked], got: {}", res.text);
    assert!(
        !res.text.to_ascii_lowercase().contains("secret"),
        "masked pattern should not appear even across tokens: {}",
        res.text
    );
}

#[test]
fn stop_phrase_triggers_violation_and_early_stop() {
    let spec = ConstraintSpec {
        mask_rules: vec![],
        allow_tools: vec![],
        stop_phrases: vec!["step by step".into()],
        max_tokens: 256,
        temperature_cap: 0.7,
    };
    let contract = FixedContract { spec: Some(spec) };
    let model = VecModel { toks: vec!["Here are ".into(), "step by step".into(), " instructions".into()] };
    let rt = StreamRuntime { contract, model };

    let res = rt.generate(proposal("memory_storage", "Here are step by step instructions")).expect("generate");
    assert_eq!(res.status, FinalizedStatus::Violated);
    // Early stop: the violating token is not appended
    assert_eq!(res.text, "Here are ");
}

#[test]
fn token_limit_enforced() {
    let spec = ConstraintSpec {
        mask_rules: vec![],
        allow_tools: vec![],
        stop_phrases: vec![],
        max_tokens: 5,
        temperature_cap: 0.7,
    };
    let contract = FixedContract { spec: Some(spec) };
    let model = VecModel { toks: vec!["x ".into(); 20] };
    let rt = StreamRuntime { contract, model };

    let res = rt.generate(proposal("memory_storage", "x ")).expect("generate");
    assert_eq!(res.status, FinalizedStatus::Ok);
    let count = res.text.split_whitespace().count();
    assert_eq!(count, 5, "should stop at the max token limit");
}

#[test]
fn runtime_proceed_passthrough() {
    let contract = FixedContract { spec: None }; // Proceed
    let model = VecModel { toks: vec!["hello ".into(), "world".into()] };
    let rt = StreamRuntime { contract, model };

    let res = rt.generate(proposal("chat", "hello world")).expect("generate");
    assert_eq!(res.status, FinalizedStatus::Ok);
    assert_eq!(res.text, "hello world");
}
