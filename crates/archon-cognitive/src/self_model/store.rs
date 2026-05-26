use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use cozo::{DataValue, DbInstance, ScriptMutability};

use crate::cozo_guard::run_script_guarded;
use crate::schema::ensure_cognitive_schema;
use crate::{CognitiveError, self_model::types::*};

pub struct SelfModelStore<'a> {
    db: &'a DbInstance,
}

impl<'a> SelfModelStore<'a> {
    pub fn new(db: &'a DbInstance) -> Result<Self, CognitiveError> {
        ensure_cognitive_schema(db)?;
        Ok(Self { db })
    }

    pub fn write_fact(&self, fact: &SelfModelFact) -> Result<(), CognitiveError> {
        let mut params = BTreeMap::new();
        params.insert("fact_id".into(), DataValue::from(fact.id.as_str()));
        params.insert("domain".into(), DataValue::from(fact.domain.as_str()));
        params.insert("fact_kind".into(), DataValue::from(fact.fact_kind.as_str()));
        params.insert("statement".into(), DataValue::from(fact.statement.as_str()));
        params.insert("confidence".into(), DataValue::from(fact.confidence as f64));
        params.insert(
            "evidence_count".into(),
            DataValue::from(fact.evidence_count as i64),
        );
        params.insert(
            "last_seen_at".into(),
            DataValue::from(fact.last_seen_at.to_rfc3339().as_str()),
        );
        params.insert(
            "expires_at".into(),
            DataValue::from(
                fact.expires_at
                    .map(|value| value.to_rfc3339())
                    .unwrap_or_default()
                    .as_str(),
            ),
        );
        params.insert(
            "created_at".into(),
            DataValue::from(fact.created_at.to_rfc3339().as_str()),
        );
        run_script_guarded(
            self.db,
            "?[fact_id, domain, fact_kind, statement, confidence, evidence_count, last_seen_at, expires_at, created_at] <- \
             [[$fact_id, $domain, $fact_kind, $statement, $confidence, $evidence_count, $last_seen_at, $expires_at, $created_at]]
             :put self_model_facts { fact_id => domain, fact_kind, statement, confidence, evidence_count, last_seen_at, expires_at, created_at }",
            params,
            ScriptMutability::Mutable,
            "write self-model fact",
        )?;
        Ok(())
    }

    pub fn write_facts(&self, facts: &[SelfModelFact]) -> Result<usize, CognitiveError> {
        for fact in facts {
            self.write_fact(fact)?;
        }
        Ok(facts.len())
    }

    pub fn get_domain_trust(&self, domain: &str) -> Result<DomainTrust, CognitiveError> {
        let rows = self.query_domain_rows(domain)?;
        let mut evidence_count = 0_u64;
        let mut weighted = 0.0_f32;
        let mut failures = Vec::new();
        let mut last_correction_at = None;

        for row in rows.rows {
            let kind = row[1].get_str().unwrap_or("");
            let confidence = row[3].get_float().unwrap_or(0.5) as f32;
            let count = row[4].get_int().unwrap_or(0).max(0) as u64;
            evidence_count += count;
            weighted += confidence * count.max(1) as f32;
            if kind == FactKind::FailureCluster.as_str() {
                failures.push(row[0].get_str().unwrap_or("").to_string());
            }
            if kind == FactKind::Correction.as_str() {
                last_correction_at = parse_time(row[5].get_str().unwrap_or(""));
            }
        }

        let trust_score = if evidence_count == 0 {
            0.5
        } else {
            let denominator = evidence_count.max(1) as f32;
            let raw = (weighted / denominator).clamp(0.0, 1.0);
            if evidence_count < 3 {
                raw.clamp(0.4, 0.6)
            } else {
                raw
            }
        };
        Ok(DomainTrust {
            domain: domain.to_owned(),
            trust_score,
            evidence_count,
            last_correction_at,
            failure_cluster_ids: failures,
        })
    }

    pub fn query_failure_clusters(
        &self,
        domain: &str,
        limit: usize,
    ) -> Result<Vec<FailureCluster>, CognitiveError> {
        let mut clusters: Vec<_> = self
            .query_domain_rows(domain)?
            .rows
            .into_iter()
            .filter(|row| row[1].get_str() == Some(FactKind::FailureCluster.as_str()))
            .map(|row| FailureCluster {
                cluster_id: row[0].get_str().unwrap_or("").to_string(),
                domain: domain.to_owned(),
                pattern_summary: row[2].get_str().unwrap_or("").to_string(),
                occurrence_count: row[4].get_int().unwrap_or(0).max(0) as u64,
                first_seen_at: parse_time(row[6].get_str().unwrap_or("")).unwrap_or_else(Utc::now),
                last_seen_at: parse_time(row[5].get_str().unwrap_or("")).unwrap_or_else(Utc::now),
                recommended_caution: None,
            })
            .collect();
        clusters.sort_by(|a, b| b.last_seen_at.cmp(&a.last_seen_at));
        clusters.truncate(limit);
        Ok(clusters)
    }

    pub fn read_profile(&self, domains: &[String]) -> Result<SelfModelProfile, CognitiveError> {
        let mut domain_trust = Vec::with_capacity(domains.len());
        let mut active_failure_clusters = Vec::new();
        for domain in domains {
            domain_trust.push(self.get_domain_trust(domain)?);
            active_failure_clusters.extend(self.query_failure_clusters(domain, 5)?);
        }
        Ok(SelfModelProfile {
            caution_rules: self.caution_rules()?,
            domain_trust,
            active_failure_clusters,
            confidence_calibration: ConfidenceCalibration::default(),
            generated_at: Utc::now(),
        })
    }

    pub fn read_memory_context(&self, domain: &str) -> Result<MemoryContext, CognitiveError> {
        let rows = self.query_domain_rows(domain)?;
        let fact_refs = rows
            .rows
            .iter()
            .map(|row| row[0].get_str().unwrap_or("").to_string())
            .collect();
        Ok(MemoryContext {
            fact_refs,
            prior_correction_ids: Vec::new(),
            failure_pattern_labels: Vec::new(),
            context_refs: Vec::new(),
        })
    }

    pub fn export_briefing(&self) -> Result<SelfModelBriefing, CognitiveError> {
        let rows = self.query_all_rows()?;
        let mut briefing = SelfModelBriefing {
            fact_count: rows.rows.len(),
            ..SelfModelBriefing::default()
        };
        for row in rows.rows {
            match row[1].get_str().unwrap_or("") {
                "failure_cluster" => briefing.active_failure_clusters += 1,
                "caution_rule" => briefing
                    .caution_rules
                    .push(row[2].get_str().unwrap_or("").to_string()),
                _ => {}
            }
        }
        Ok(briefing)
    }

    fn caution_rules(&self) -> Result<Vec<String>, CognitiveError> {
        Ok(self
            .query_all_rows()?
            .rows
            .into_iter()
            .filter(|row| row[1].get_str() == Some(FactKind::CautionRule.as_str()))
            .map(|row| row[2].get_str().unwrap_or("").to_string())
            .collect())
    }

    fn query_domain_rows(&self, domain: &str) -> Result<cozo::NamedRows, CognitiveError> {
        let mut params = BTreeMap::new();
        params.insert("domain".into(), DataValue::from(domain));
        run_script_guarded(
            self.db,
            "?[fact_id, fact_kind, statement, confidence, evidence_count, last_seen_at, created_at] := \
             *self_model_facts{fact_id, domain: $domain, fact_kind, statement, confidence, evidence_count, last_seen_at, created_at}",
            params,
            ScriptMutability::Immutable,
            "query self-model facts by domain",
        )
    }

    fn query_all_rows(&self) -> Result<cozo::NamedRows, CognitiveError> {
        run_script_guarded(
            self.db,
            "?[fact_id, fact_kind, statement] := *self_model_facts{fact_id, fact_kind, statement}",
            Default::default(),
            ScriptMutability::Immutable,
            "query self-model facts",
        )
    }
}

fn parse_time(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|value| value.with_timezone(&Utc))
}
