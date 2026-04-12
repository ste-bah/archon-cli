//! TASK-AGS-503: BrokerPattern — capability-based agent selection.
//!
//! Selects the best agent from a candidate list based on availability,
//! capabilities, cost, or a custom selector function, then delegates
//! to `TaskServiceHandle`.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashMap;
use serde_json::Value;

use super::{BrokerConfig, BrokerSelector, Pattern, PatternCtx, PatternError, PatternKind, PatternRegistry};

// ---------------------------------------------------------------------------
// AgentRegistryHandle — slim trait wrapping phase-3 AgentRegistry
// ---------------------------------------------------------------------------

/// Slim abstraction over the phase-3 `AgentRegistry` for candidate lookup.
pub trait AgentRegistryHandle: Send + Sync {
    fn lookup_candidates(&self, names: &[String]) -> Vec<Candidate>;
}

/// A candidate agent returned by the registry.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub name: String,
    pub capabilities: Vec<String>,
    pub available: bool,
    pub cost: f64,
}

/// Type alias for custom selector functions.
pub type CustomSelectorFn =
    Arc<dyn Fn(&[Candidate], &Value) -> Option<usize> + Send + Sync>;

// ---------------------------------------------------------------------------
// BrokerPattern
// ---------------------------------------------------------------------------

/// Capability-based agent selection pattern.
pub struct BrokerPattern {
    registry: Arc<dyn AgentRegistryHandle>,
    selectors: DashMap<String, CustomSelectorFn>,
    round_robin_counter: AtomicUsize,
    config: BrokerConfig,
}

impl BrokerPattern {
    pub fn new(
        registry: Arc<dyn AgentRegistryHandle>,
        config: BrokerConfig,
    ) -> Self {
        Self {
            registry,
            selectors: DashMap::new(),
            round_robin_counter: AtomicUsize::new(0),
            config,
        }
    }

    /// Register a custom selector function (NFR-ARCH-001 extensibility).
    pub fn register_custom_selector(&self, name: &str, f: CustomSelectorFn) {
        self.selectors.insert(name.to_owned(), f);
    }

    fn select_candidate(
        &self,
        alive: &[Candidate],
        input: &Value,
    ) -> Result<usize, PatternError> {
        match &self.config.selector {
            BrokerSelector::RoundRobin => {
                if alive.is_empty() {
                    return Err(PatternError::BrokerNoCandidate {
                        reasons: vec!["no alive candidates".into()],
                    });
                }
                let idx = self.round_robin_counter.fetch_add(1, Ordering::SeqCst)
                    % alive.len();
                Ok(idx)
            }
            BrokerSelector::Capability => {
                let required: Vec<String> = input
                    .get("required_caps")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default();

                for (i, c) in alive.iter().enumerate() {
                    let has_all = required
                        .iter()
                        .all(|req| c.capabilities.contains(req));
                    if has_all {
                        return Ok(i);
                    }
                }

                let reasons: Vec<String> = alive
                    .iter()
                    .map(|c| {
                        let missing: Vec<&String> = required
                            .iter()
                            .filter(|r| !c.capabilities.contains(r))
                            .collect();
                        format!("{}: missing capabilities {:?}", c.name, missing)
                    })
                    .collect();

                Err(PatternError::BrokerNoCandidate { reasons })
            }
            BrokerSelector::Cost => {
                alive
                    .iter()
                    .enumerate()
                    .min_by(|(_, a), (_, b)| a.cost.total_cmp(&b.cost))
                    .map(|(i, _)| i)
                    .ok_or_else(|| PatternError::BrokerNoCandidate {
                        reasons: vec!["no alive candidates for cost selection".into()],
                    })
            }
            BrokerSelector::Custom(name) => {
                let selector = self.selectors.get(name).ok_or_else(|| {
                    PatternError::Execution(format!(
                        "custom selector '{name}' not registered"
                    ))
                })?;
                selector(alive, input).ok_or_else(|| {
                    PatternError::BrokerNoCandidate {
                        reasons: vec![format!(
                            "custom selector '{name}' returned None"
                        )],
                    }
                })
            }
        }
    }
}

#[async_trait]
impl Pattern for BrokerPattern {
    fn kind(&self) -> PatternKind {
        PatternKind::Broker
    }

    async fn execute(
        &self,
        input: Value,
        ctx: PatternCtx,
    ) -> Result<Value, PatternError> {
        let all_candidates = self
            .registry
            .lookup_candidates(&self.config.candidates);

        // Filter available, collecting rejection reasons for unavailable.
        let mut alive: Vec<Candidate> = Vec::new();
        let mut reasons: Vec<String> = Vec::new();

        for c in &all_candidates {
            if c.available {
                alive.push(c.clone());
            } else {
                reasons.push(format!("{}: unavailable", c.name));
            }
        }

        if alive.is_empty() {
            return Err(PatternError::BrokerNoCandidate { reasons });
        }

        let idx = self.select_candidate(&alive, &input)?;
        let chosen = &alive[idx];

        ctx.task_service.submit(&chosen.name, input).await
    }
}

/// Register a BrokerPattern into the registry under name "broker".
pub fn register(
    reg: &PatternRegistry,
    agent_registry: Arc<dyn AgentRegistryHandle>,
    config: BrokerConfig,
) {
    reg.register(
        "broker",
        Arc::new(BrokerPattern::new(agent_registry, config)),
    );
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use async_trait::async_trait;
    use serde_json::json;

    use crate::patterns::{PatternRegistry, TaskServiceHandle};

    // Stub registry: returns fixed candidates.
    struct StubAgentRegistry {
        candidates: Vec<Candidate>,
    }

    impl AgentRegistryHandle for StubAgentRegistry {
        fn lookup_candidates(&self, names: &[String]) -> Vec<Candidate> {
            self.candidates
                .iter()
                .filter(|c| names.contains(&c.name))
                .cloned()
                .collect()
        }
    }

    // Stub task service: returns agent name in output.
    struct StubTaskService;

    #[async_trait]
    impl TaskServiceHandle for StubTaskService {
        async fn submit(
            &self,
            agent: &str,
            _input: Value,
        ) -> Result<Value, PatternError> {
            Ok(json!({"chosen": agent}))
        }
    }

    fn make_ctx() -> PatternCtx {
        PatternCtx {
            task_service: Arc::new(StubTaskService),
            registry: Arc::new(PatternRegistry::new()),
            trace_id: "test".into(),
            deadline: None,
        }
    }

    fn three_candidates() -> Vec<Candidate> {
        vec![
            Candidate {
                name: "a".into(),
                capabilities: vec!["rust".into()],
                available: true,
                cost: 1.0,
            },
            Candidate {
                name: "b".into(),
                capabilities: vec!["rust".into(), "async".into()],
                available: true,
                cost: 2.0,
            },
            Candidate {
                name: "c".into(),
                capabilities: vec!["python".into()],
                available: true,
                cost: 0.5,
            },
        ]
    }

    #[tokio::test]
    async fn test_broker_round_robin_cycles_candidates() {
        let reg = Arc::new(StubAgentRegistry {
            candidates: three_candidates(),
        });
        let config = BrokerConfig {
            candidates: vec!["a".into(), "b".into(), "c".into()],
            selector: BrokerSelector::RoundRobin,
        };
        let pattern = BrokerPattern::new(reg, config);

        let mut chosen_names = Vec::new();
        for _ in 0..6 {
            let ctx = make_ctx();
            let result = pattern.execute(json!({}), ctx).await.unwrap();
            chosen_names.push(
                result["chosen"].as_str().unwrap().to_string(),
            );
        }

        // Each of the 3 candidates should be hit exactly twice.
        let count_a = chosen_names.iter().filter(|n| *n == "a").count();
        let count_b = chosen_names.iter().filter(|n| *n == "b").count();
        let count_c = chosen_names.iter().filter(|n| *n == "c").count();
        assert_eq!(count_a, 2);
        assert_eq!(count_b, 2);
        assert_eq!(count_c, 2);
    }

    #[tokio::test]
    async fn test_broker_capability_selects_superset() {
        let reg = Arc::new(StubAgentRegistry {
            candidates: three_candidates(),
        });
        let config = BrokerConfig {
            candidates: vec!["a".into(), "b".into(), "c".into()],
            selector: BrokerSelector::Capability,
        };
        let pattern = BrokerPattern::new(reg, config);
        let ctx = make_ctx();

        // Require ["rust", "async"] — only "b" has both.
        let input = json!({"required_caps": ["rust", "async"]});
        let result = pattern.execute(input, ctx).await.unwrap();
        assert_eq!(result["chosen"].as_str().unwrap(), "b");
    }

    #[tokio::test]
    async fn test_broker_cost_picks_minimum() {
        let reg = Arc::new(StubAgentRegistry {
            candidates: three_candidates(),
        });
        let config = BrokerConfig {
            candidates: vec!["a".into(), "b".into(), "c".into()],
            selector: BrokerSelector::Cost,
        };
        let pattern = BrokerPattern::new(reg, config);
        let ctx = make_ctx();

        // Costs: a=1.0, b=2.0, c=0.5 → c chosen.
        let result = pattern.execute(json!({}), ctx).await.unwrap();
        assert_eq!(result["chosen"].as_str().unwrap(), "c");
    }

    #[tokio::test]
    async fn test_broker_no_suitable_returns_rejections() {
        // EC-ARCH-004: all unavailable.
        let candidates = vec![
            Candidate {
                name: "a".into(),
                capabilities: vec![],
                available: false,
                cost: 0.0,
            },
            Candidate {
                name: "b".into(),
                capabilities: vec![],
                available: false,
                cost: 0.0,
            },
            Candidate {
                name: "c".into(),
                capabilities: vec![],
                available: false,
                cost: 0.0,
            },
        ];
        let reg = Arc::new(StubAgentRegistry { candidates });
        let config = BrokerConfig {
            candidates: vec!["a".into(), "b".into(), "c".into()],
            selector: BrokerSelector::RoundRobin,
        };
        let pattern = BrokerPattern::new(reg, config);
        let ctx = make_ctx();

        let err = pattern.execute(json!({}), ctx).await.unwrap_err();
        match err {
            PatternError::BrokerNoCandidate { reasons } => {
                assert_eq!(reasons.len(), 3, "one reason per candidate");
                for r in &reasons {
                    assert!(
                        r.contains("unavailable"),
                        "reason must explain why: {r}"
                    );
                }
            }
            other => panic!("expected BrokerNoCandidate, got: {other}"),
        }
    }

    #[tokio::test]
    async fn test_broker_custom_selector_registered() {
        let reg = Arc::new(StubAgentRegistry {
            candidates: three_candidates(),
        });
        let config = BrokerConfig {
            candidates: vec!["a".into(), "b".into(), "c".into()],
            selector: BrokerSelector::Custom("weighted".into()),
        };
        let pattern = BrokerPattern::new(reg, config);

        // Register custom selector that always picks index 2 ("c").
        pattern.register_custom_selector(
            "weighted",
            Arc::new(|_candidates, _input| Some(2)),
        );

        let ctx = make_ctx();
        let result = pattern.execute(json!({}), ctx).await.unwrap();
        assert_eq!(result["chosen"].as_str().unwrap(), "c");
    }
}
