//! Temporal reasoning engine — time-based reasoning.
//!
//! Handles temporal relationships (before, after, during, overlaps),
//! builds a timeline from context events, applies Allen's interval algebra
//! for constraint propagation, and returns temporal inferences.

use std::collections::HashMap;

use anyhow::Result;

use super::{ReasoningEngine, ReasoningItem, ReasoningOutput, ReasoningRequest, ResultType};

/// Temporal reasoning: Allen's interval algebra and timeline analysis.
pub struct TemporalEngine;

impl Default for TemporalEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl TemporalEngine {
    pub fn new() -> Self {
        Self
    }

    /// Parse events from context.
    /// Format: "event:name:start:end" where start/end are numeric.
    fn parse_events(context: &[String]) -> Vec<Event> {
        let mut events = Vec::new();
        for line in context {
            let trimmed = line.trim();
            if let Some(rest) = trimmed.strip_prefix("event:") {
                let parts: Vec<&str> = rest.split(':').collect();
                if parts.len() >= 3 {
                    let name = parts[0].trim().to_string();
                    let start = parts[1].trim().parse::<f64>().unwrap_or(0.0);
                    let end = if parts.len() >= 4 {
                        parts[2].trim().parse::<f64>().unwrap_or(start + 1.0)
                    } else {
                        // Point event — make it a small interval.
                        // parts[2] was the end value.
                        parts[2].trim().parse::<f64>().unwrap_or(start + 1.0)
                    };
                    events.push(Event { name, start, end });
                }
            } else if !trimmed.is_empty() {
                // Try to parse free-form temporal statements.
                let lower = trimmed.to_lowercase();
                if lower.contains("before") || lower.contains("after") || lower.contains("during") {
                    events.push(Event {
                        name: trimmed.to_string(),
                        start: 0.0,
                        end: 1.0,
                    });
                }
            }
        }

        // Sort by start time.
        events.sort_by(|a, b| {
            a.start
                .partial_cmp(&b.start)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        events
    }

    /// Determine Allen's interval relation between two events.
    fn allen_relation(a: &Event, b: &Event) -> AllenRelation {
        // Allen's 13 basic relations (using 7 + inverses).
        if a.end < b.start {
            AllenRelation::Before
        } else if a.end == b.start {
            AllenRelation::Meets
        } else if a.start < b.start && a.end > b.start && a.end < b.end {
            AllenRelation::Overlaps
        } else if a.start == b.start && a.end < b.end {
            AllenRelation::Starts
        } else if a.start > b.start && a.end < b.end {
            AllenRelation::During
        } else if a.start > b.start && a.end == b.end {
            AllenRelation::Finishes
        } else if a.start == b.start && a.end == b.end {
            AllenRelation::Equal
        } else if a.start < b.start && a.end > b.end {
            AllenRelation::Contains
        } else if a.start < b.start && a.end == b.end {
            AllenRelation::FinishedBy
        } else if a.start == b.start && a.end > b.end {
            AllenRelation::StartedBy
        } else if a.start > b.start && a.start < b.end && a.end > b.end {
            AllenRelation::OverlappedBy
        } else if a.start == b.end {
            AllenRelation::MetBy
        } else {
            AllenRelation::After
        }
    }

    /// Build a relation matrix for all event pairs.
    fn build_relation_matrix(events: &[Event]) -> HashMap<(usize, usize), AllenRelation> {
        let mut matrix = HashMap::new();
        for i in 0..events.len() {
            for j in (i + 1)..events.len() {
                let rel = Self::allen_relation(&events[i], &events[j]);
                matrix.insert((i, j), rel);
            }
        }
        matrix
    }

    /// Generate temporal inferences from the relation matrix.
    fn generate_inferences(
        events: &[Event],
        matrix: &HashMap<(usize, usize), AllenRelation>,
    ) -> Vec<TemporalInference> {
        let mut inferences = Vec::new();

        // Direct pairwise inferences.
        for (&(i, j), rel) in matrix {
            let desc = format!(
                "'{}' {} '{}'",
                events[i].name,
                rel.description(),
                events[j].name
            );
            inferences.push(TemporalInference {
                description: desc,
                confidence: 0.95,
                events_involved: vec![events[i].name.clone(), events[j].name.clone()],
            });
        }

        // Transitive inferences: if A before B and B before C, then A before C.
        for i in 0..events.len() {
            for j in (i + 1)..events.len() {
                for k in (j + 1)..events.len() {
                    let r_ij = matrix.get(&(i, j));
                    let r_jk = matrix.get(&(j, k));
                    if let (Some(AllenRelation::Before), Some(AllenRelation::Before)) = (r_ij, r_jk)
                    {
                        inferences.push(TemporalInference {
                            description: format!(
                                "By transitivity: '{}' before '{}' (via '{}')",
                                events[i].name, events[k].name, events[j].name
                            ),
                            confidence: 0.9,
                            events_involved: vec![
                                events[i].name.clone(),
                                events[j].name.clone(),
                                events[k].name.clone(),
                            ],
                        });
                    }
                }
            }
        }

        // Timeline summary.
        if events.len() >= 2 {
            let total_span = events.last().map(|e| e.end).unwrap_or(0.0)
                - events.first().map(|e| e.start).unwrap_or(0.0);
            inferences.push(TemporalInference {
                description: format!(
                    "Timeline spans {:.1} units across {} events",
                    total_span,
                    events.len()
                ),
                confidence: 1.0,
                events_involved: events.iter().map(|e| e.name.clone()).collect(),
            });
        }

        inferences
    }
}

struct Event {
    name: String,
    start: f64,
    end: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum AllenRelation {
    Before,
    Meets,
    Overlaps,
    Starts,
    During,
    Finishes,
    Equal,
    Contains,
    FinishedBy,
    StartedBy,
    OverlappedBy,
    MetBy,
    After,
}

impl AllenRelation {
    fn description(&self) -> &str {
        match self {
            Self::Before => "is before",
            Self::Meets => "meets",
            Self::Overlaps => "overlaps with",
            Self::Starts => "starts with",
            Self::During => "is during",
            Self::Finishes => "finishes with",
            Self::Equal => "equals",
            Self::Contains => "contains",
            Self::FinishedBy => "is finished by",
            Self::StartedBy => "is started by",
            Self::OverlappedBy => "is overlapped by",
            Self::MetBy => "is met by",
            Self::After => "is after",
        }
    }
}

struct TemporalInference {
    description: String,
    confidence: f64,
    events_involved: Vec<String>,
}

impl ReasoningEngine for TemporalEngine {
    fn name(&self) -> &str {
        "temporal"
    }

    fn reason(&self, request: &ReasoningRequest) -> Result<ReasoningOutput> {
        let events = Self::parse_events(&request.context);

        if events.is_empty() {
            return Ok(ReasoningOutput {
                engine_name: "temporal".to_string(),
                result_type: ResultType::TemporalInferences,
                items: vec![ReasoningItem {
                    label: "No Events".to_string(),
                    description: "No temporal events could be parsed from context".to_string(),
                    confidence: 0.0,
                    supporting_evidence: vec![],
                }],
                confidence: 0.0,
                provenance: vec![format!("query: {}", request.query)],
            });
        }

        let matrix = Self::build_relation_matrix(&events);
        let inferences = Self::generate_inferences(&events, &matrix);

        let items: Vec<ReasoningItem> = inferences
            .iter()
            .enumerate()
            .map(|(i, inf)| ReasoningItem {
                label: format!("T{}", i + 1),
                description: inf.description.clone(),
                confidence: inf.confidence,
                supporting_evidence: inf.events_involved.clone(),
            })
            .collect();

        let overall_confidence = if items.is_empty() {
            0.0
        } else {
            items.iter().map(|i| i.confidence).sum::<f64>() / items.len() as f64
        };

        Ok(ReasoningOutput {
            engine_name: "temporal".to_string(),
            result_type: ResultType::TemporalInferences,
            items,
            confidence: overall_confidence,
            provenance: vec![
                format!("query: {}", request.query),
                format!("event_count: {}", events.len()),
            ],
        })
    }
}
