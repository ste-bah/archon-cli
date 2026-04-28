//! Extended reasoning mode engines — 12 specialized reasoning paradigms.
//! Implements REQ-LEARN-011.

pub mod abductive;
pub mod adversarial;
pub mod analogical;
pub mod causal;
pub mod constraint;
pub mod contextual;
pub mod counterfactual;
pub mod decomposition;
pub mod deductive;
pub mod first_principles;
pub mod inductive;
pub mod temporal;

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Common trait for all reasoning engines.
pub trait ReasoningEngine: Send + Sync {
    fn name(&self) -> &str;
    fn reason(&self, request: &ReasoningRequest) -> Result<ReasoningOutput>;
}

/// Input request for extended reasoning modes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasoningRequest {
    pub query: String,
    pub context: Vec<String>,
    pub parameters: std::collections::HashMap<String, String>,
}

/// Structured output from a reasoning engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasoningOutput {
    pub engine_name: String,
    pub result_type: ResultType,
    pub items: Vec<ReasoningItem>,
    pub confidence: f64,
    pub provenance: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ResultType {
    Hypotheses,
    Arguments,
    Mappings,
    Assignments,
    Outcomes,
    Subproblems,
    Axioms,
    TemporalInferences,
    CausalChains,
    ContextualInsights,
    Deductions,
    Generalizations,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasoningItem {
    pub label: String,
    pub description: String,
    pub confidence: f64,
    pub supporting_evidence: Vec<String>,
}

/// Selects the appropriate extended reasoning mode based on query analysis.
pub struct ExtendedModeSelector;

impl ExtendedModeSelector {
    pub fn select(query: &str) -> Option<&'static str> {
        let lower = query.to_lowercase();
        if lower.contains("explain why") || lower.contains("best explanation") {
            return Some("abductive");
        }
        if lower.contains("argue against")
            || lower.contains("counter")
            || lower.contains("devil's advocate")
        {
            return Some("adversarial");
        }
        if lower.contains("analogous") || lower.contains("similar to") || lower.contains("like how")
        {
            return Some("analogical");
        }
        if lower.contains("constraint") || lower.contains("satisfy") || lower.contains("feasible") {
            return Some("constraint");
        }
        if lower.contains("what if")
            || lower.contains("hypothetical")
            || lower.contains("counterfactual")
        {
            return Some("counterfactual");
        }
        if lower.contains("break down")
            || lower.contains("decompose")
            || lower.contains("sub-problem")
        {
            return Some("decomposition");
        }
        if lower.contains("first principles")
            || lower.contains("fundamental")
            || lower.contains("from scratch")
        {
            return Some("first_principles");
        }
        if lower.contains("timeline")
            || lower.contains("before")
            || lower.contains("after")
            || lower.contains("temporal")
        {
            return Some("temporal");
        }
        if lower.contains("deduce")
            || lower.contains("therefore")
            || lower.contains("syllogism")
            || lower.contains("logical")
        {
            return Some("deductive");
        }
        if lower.contains("generalize")
            || lower.contains("induct")
            || lower.contains("pattern from")
            || lower.contains("examples show")
        {
            return Some("inductive");
        }
        if lower.contains("causes ")
            || lower.contains("effect of")
            || lower.contains("causal chain")
            || lower.contains("leads to")
        {
            return Some("causal");
        }
        if lower.contains("depends on context")
            || lower.contains("situational")
            || lower.contains("contextual")
            || lower.contains("in this scenario")
        {
            return Some("contextual");
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use abductive::AbductiveEngine;
    use adversarial::AdversarialEngine;
    use analogical::AnalogicalEngine;
    use causal::CausalEngine;
    use constraint::ConstraintEngine;
    use contextual::ContextualEngine;
    use counterfactual::CounterfactualEngine;
    use decomposition::DecompositionEngine;
    use deductive::DeductiveEngine;
    use first_principles::FirstPrinciplesEngine;
    use inductive::InductiveEngine;
    use temporal::TemporalEngine;

    fn make_request(query: &str, context: Vec<&str>) -> ReasoningRequest {
        ReasoningRequest {
            query: query.to_string(),
            context: context.into_iter().map(String::from).collect(),
            parameters: Default::default(),
        }
    }

    fn make_request_with_params(
        query: &str,
        context: Vec<&str>,
        params: Vec<(&str, &str)>,
    ) -> ReasoningRequest {
        ReasoningRequest {
            query: query.to_string(),
            context: context.into_iter().map(String::from).collect(),
            parameters: params
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    #[test]
    fn test_abductive_returns_hypotheses() {
        let engine = AbductiveEngine::new();
        let req = make_request(
            "explain why the server crashed",
            vec![
                "high memory usage observed",
                "disk full alert triggered",
                "OOM killer active",
            ],
        );
        let output = engine.reason(&req).unwrap();
        assert_eq!(output.result_type, ResultType::Hypotheses);
        assert!(!output.items.is_empty());
        assert!(output.confidence > 0.0);
    }

    #[test]
    fn test_adversarial_returns_arguments() {
        let engine = AdversarialEngine::new();
        let req = make_request(
            "argue against: Rust is the best language for all projects",
            vec![
                "Rust has a steep learning curve",
                "Garbage collected languages are simpler",
            ],
        );
        let output = engine.reason(&req).unwrap();
        assert_eq!(output.result_type, ResultType::Arguments);
        assert!(!output.items.is_empty());
    }

    #[test]
    fn test_analogical_returns_mappings() {
        let engine = AnalogicalEngine::new();
        let req = make_request_with_params(
            "how is a CPU similar to a brain",
            vec!["CPU processes instructions", "brain processes signals"],
            vec![("source_domain", "CPU"), ("target_domain", "brain")],
        );
        let output = engine.reason(&req).unwrap();
        assert_eq!(output.result_type, ResultType::Mappings);
        assert!(!output.items.is_empty());
    }

    #[test]
    fn test_constraint_returns_assignments() {
        let engine = ConstraintEngine::new();
        let req = make_request_with_params(
            "assign tasks to workers",
            vec![
                "var:worker1:task_a,task_b",
                "var:worker2:task_a,task_c",
                "constraint:worker1!=worker2",
            ],
            vec![],
        );
        let output = engine.reason(&req).unwrap();
        assert_eq!(output.result_type, ResultType::Assignments);
        assert!(!output.items.is_empty());
    }

    #[test]
    fn test_counterfactual_returns_outcomes() {
        let engine = CounterfactualEngine::new();
        let req = make_request(
            "what if we had used a cache",
            vec![
                "current latency is 200ms",
                "database queries dominate",
                "cache hit rate would be 80%",
            ],
        );
        let output = engine.reason(&req).unwrap();
        assert_eq!(output.result_type, ResultType::Outcomes);
        assert!(!output.items.is_empty());
    }

    #[test]
    fn test_decomposition_returns_subproblems() {
        let engine = DecompositionEngine::new();
        let req = make_request(
            "build a web application",
            vec![
                "need frontend",
                "need backend API",
                "need database",
                "frontend depends on API",
                "API depends on database",
            ],
        );
        let output = engine.reason(&req).unwrap();
        assert_eq!(output.result_type, ResultType::Subproblems);
        assert!(!output.items.is_empty());
    }

    #[test]
    fn test_first_principles_returns_axioms() {
        let engine = FirstPrinciplesEngine::new();
        let req = make_request(
            "why do we need automated testing",
            vec![
                "axiom: software has bugs",
                "axiom: manual testing is slow",
                "axiom: regressions occur on change",
                "assumption: developers always write correct code",
            ],
        );
        let output = engine.reason(&req).unwrap();
        assert_eq!(output.result_type, ResultType::Axioms);
        assert!(!output.items.is_empty());
    }

    #[test]
    fn test_temporal_returns_inferences() {
        let engine = TemporalEngine::new();
        let req = make_request(
            "analyze deployment timeline",
            vec![
                "event:build:0:10",
                "event:test:10:30",
                "event:deploy:30:35",
                "event:monitor:35:60",
            ],
        );
        let output = engine.reason(&req).unwrap();
        assert_eq!(output.result_type, ResultType::TemporalInferences);
        assert!(!output.items.is_empty());
    }

    #[test]
    fn test_all_engines_implement_trait() {
        let engines: Vec<Box<dyn ReasoningEngine>> = vec![
            Box::new(AbductiveEngine::new()),
            Box::new(AdversarialEngine::new()),
            Box::new(AnalogicalEngine::new()),
            Box::new(CausalEngine::new()),
            Box::new(ConstraintEngine::new()),
            Box::new(ContextualEngine::new()),
            Box::new(CounterfactualEngine::new()),
            Box::new(DecompositionEngine::new()),
            Box::new(DeductiveEngine::new()),
            Box::new(FirstPrinciplesEngine::new()),
            Box::new(InductiveEngine::new()),
            Box::new(TemporalEngine::new()),
        ];
        assert_eq!(engines.len(), 12);
        let names: Vec<&str> = engines.iter().map(|e| e.name()).collect();
        assert!(names.contains(&"abductive"));
        assert!(names.contains(&"adversarial"));
        assert!(names.contains(&"analogical"));
        assert!(names.contains(&"causal"));
        assert!(names.contains(&"constraint"));
        assert!(names.contains(&"contextual"));
        assert!(names.contains(&"counterfactual"));
        assert!(names.contains(&"decomposition"));
        assert!(names.contains(&"deductive"));
        assert!(names.contains(&"first_principles"));
        assert!(names.contains(&"inductive"));
        assert!(names.contains(&"temporal"));
    }

    #[test]
    fn test_extended_mode_selector() {
        assert_eq!(
            ExtendedModeSelector::select("explain why the server failed"),
            Some("abductive")
        );
        assert_eq!(
            ExtendedModeSelector::select("argue against this proposal"),
            Some("adversarial")
        );
        assert_eq!(
            ExtendedModeSelector::select("this is analogous to biology"),
            Some("analogical")
        );
        assert_eq!(
            ExtendedModeSelector::select("satisfy these constraints"),
            Some("constraint")
        );
        assert_eq!(
            ExtendedModeSelector::select("what if we changed the API"),
            Some("counterfactual")
        );
        assert_eq!(
            ExtendedModeSelector::select("break down this problem"),
            Some("decomposition")
        );
        assert_eq!(
            ExtendedModeSelector::select("reason from first principles"),
            Some("first_principles")
        );
        assert_eq!(
            ExtendedModeSelector::select("build a timeline of events"),
            Some("temporal")
        );
    }

    #[test]
    fn test_mode_selector_returns_none_for_generic() {
        assert_eq!(ExtendedModeSelector::select("hello world"), None);
        assert_eq!(ExtendedModeSelector::select("implement a function"), None);
        assert_eq!(ExtendedModeSelector::select(""), None);
    }

    #[test]
    fn test_deductive_returns_deductions() {
        let engine = DeductiveEngine::new();
        let req = make_request(
            "therefore what follows",
            vec![
                "if: it rains then the ground is wet",
                "premise: it rains",
            ],
        );
        let output = engine.reason(&req).unwrap();
        assert_eq!(output.result_type, ResultType::Deductions);
        assert!(!output.items.is_empty());
    }

    #[test]
    fn test_inductive_returns_generalizations() {
        let engine = InductiveEngine::new();
        let req = make_request(
            "generalize from these examples",
            vec![
                "example:bird|wings,feathers,beak",
                "example:bird|wings,feathers,talons",
                "example:bird|wings,beak,talons",
            ],
        );
        let output = engine.reason(&req).unwrap();
        assert_eq!(output.result_type, ResultType::Generalizations);
        assert!(!output.items.is_empty());
    }

    #[test]
    fn test_causal_returns_causal_chains() {
        let engine = CausalEngine::new();
        let req = make_request(
            "what is the chain of events",
            vec![
                "cause:rain -> effect:wet ground",
                "cause:wet ground -> effect:slippery surface",
                "cause:slippery surface -> effect:accident",
            ],
        );
        let output = engine.reason(&req).unwrap();
        assert_eq!(output.result_type, ResultType::CausalChains);
        assert!(!output.items.is_empty());
    }

    #[test]
    fn test_contextual_returns_contextual_insights() {
        let engine = ContextualEngine::new();
        let req = make_request(
            "what should we do in this scenario",
            vec![
                "scenario: production outage",
                "signal: database is down",
                "signal: users reporting errors",
                "context: peak traffic hours",
            ],
        );
        let output = engine.reason(&req).unwrap();
        assert_eq!(output.result_type, ResultType::ContextualInsights);
        assert!(!output.items.is_empty());
    }

    #[test]
    fn test_extended_mode_selector_new_modes() {
        assert_eq!(
            ExtendedModeSelector::select("deduce the logical conclusion"),
            Some("deductive")
        );
        assert_eq!(
            ExtendedModeSelector::select("generalize from examples"),
            Some("inductive")
        );
        assert_eq!(
            ExtendedModeSelector::select("what causes this effect"),
            Some("causal")
        );
        assert_eq!(
            ExtendedModeSelector::select("it depends on context"),
            Some("contextual")
        );
    }
}
