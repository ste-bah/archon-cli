use crate::coding::rlm::RlmStore;
use crate::prompt_cap::count_tokens;
use crate::runner::{AgentInfo, PipelineSession};

use super::agents::{ResearchAgent, get_agent_by_key};

#[derive(Clone, Debug)]
pub struct ResearchRlmEntry {
    pub ordinal: usize,
    pub agent_key: String,
    pub phase: u8,
    pub memory_keys: Vec<String>,
    pub output_artifacts: Vec<String>,
    pub token_count: usize,
    pub output: String,
}

#[derive(Debug, Default)]
pub struct ResearchRlm {
    store: RlmStore,
    entries: Vec<ResearchRlmEntry>,
}

impl ResearchRlm {
    pub fn new() -> Self {
        Self {
            store: RlmStore::new(),
            entries: Vec::new(),
        }
    }

    pub fn write_agent_output(&mut self, agent: &ResearchAgent, ordinal: usize, output: &str) {
        for key in agent.memory_keys {
            self.store.write(key, output);
        }
        self.store
            .write(&format!("research/outputs/{}", agent.key), output);
        for artifact in agent.output_artifacts {
            self.store
                .write(&format!("research/artifacts/{artifact}"), output);
        }

        self.entries.push(ResearchRlmEntry {
            ordinal,
            agent_key: agent.key.to_string(),
            phase: agent.phase,
            memory_keys: agent
                .memory_keys
                .iter()
                .map(|key| (*key).to_string())
                .collect(),
            output_artifacts: agent
                .output_artifacts
                .iter()
                .map(|artifact| (*artifact).to_string())
                .collect(),
            token_count: count_tokens(output),
            output: output.to_string(),
        });
    }

    pub fn build_context(&self, session: &PipelineSession, agent: &ResearchAgent) -> String {
        if self.entries.is_empty() && session.agent_results.is_empty() {
            return String::new();
        }

        let fallback_entries;
        let entries = if self.entries.is_empty() {
            fallback_entries = entries_from_session(session);
            fallback_entries.as_slice()
        } else {
            self.entries.as_slice()
        };

        let mut parts = vec![
            self.identity_context(session, agent, entries.len()),
            self.manifest_context(session, entries),
            self.namespace_context(agent),
        ];

        if let Some(pinned) = self.pinned_context(agent, entries) {
            parts.push(pinned);
        }
        if let Some(active) = self.rolling_context(agent, entries) {
            parts.push(active);
        }
        if let Some(prescan) = self.consistency_prescan(agent, entries) {
            parts.push(prescan);
        }

        parts
            .into_iter()
            .filter(|part| !part.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n\n---\n\n")
    }

    fn identity_context(
        &self,
        session: &PipelineSession,
        agent: &ResearchAgent,
        completed_count: usize,
    ) -> String {
        format!(
            "## Research RLM Identity\n\
             Session: {}\n\
             Target agent: {}\n\
             Phase: {}\n\
             Completed agents: {}\n\
             Contract: Archon injects persisted run-level memory and rolling context. \
             Do not claim filesystem or memory-store access is unavailable when the \
             required evidence is present in this context. Logical `research/...` \
             namespaces are memory keys, not project-root file paths. If you need \
             to read a prior artifact, use only the concrete `.archon/pipelines/...` \
             paths listed in the manifest. Return the full Markdown deliverable, \
             not a status note saying artifacts or memory were created.",
            session.id, agent.key, agent.phase, completed_count
        )
    }

    fn manifest_context(&self, session: &PipelineSession, entries: &[ResearchRlmEntry]) -> String {
        let rows = entries
            .iter()
            .map(|entry| {
                format!(
                    "- `{ordinal:03}-{key}` phase {phase}, tokens approx {tokens}, memory: {memory}, artifacts: {artifacts}\n  files: {files}",
                    ordinal = entry.ordinal,
                    key = entry.agent_key,
                    phase = entry.phase,
                    tokens = entry.token_count,
                    memory = entry.memory_keys.join(", "),
                    artifacts = entry.output_artifacts.join(", "),
                    files = manifest_files(session, entry).join("; ")
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        format!("## Accepted Output Manifest\n\n{rows}")
    }

    fn namespace_context(&self, agent: &ResearchAgent) -> String {
        let mut sections = Vec::new();
        for key in agent.memory_keys {
            if let Some(content) = self.store.read(key) {
                sections.push(format!(
                    "### RLM Namespace `{key}`\n\n{}",
                    truncate_chars(&content, namespace_budget(agent))
                ));
            }
        }
        sections.join("\n\n")
    }

    fn pinned_context(
        &self,
        agent: &ResearchAgent,
        entries: &[ResearchRlmEntry],
    ) -> Option<String> {
        let wanted = pinned_agent_keys(agent);
        if wanted.is_empty() {
            return None;
        }
        let mut sections = Vec::new();
        for key in wanted {
            if let Some(entry) = entries.iter().find(|entry| entry.agent_key == key) {
                sections.push(format!(
                    "### Pinned Output `{ordinal:03}-{key}`\n\n{}",
                    truncate_chars(&entry.output, pinned_budget(agent)),
                    ordinal = entry.ordinal,
                    key = entry.agent_key
                ));
            }
        }
        if sections.is_empty() {
            None
        } else {
            Some(format!(
                "## Pinned Research Context\n\n{}",
                sections.join("\n\n")
            ))
        }
    }

    fn rolling_context(
        &self,
        agent: &ResearchAgent,
        entries: &[ResearchRlmEntry],
    ) -> Option<String> {
        let window = rolling_window_size(agent);
        let mut entries: Vec<&ResearchRlmEntry> = entries.iter().rev().take(window).collect();
        entries.reverse();
        if entries.is_empty() {
            return None;
        }
        let mut sections = Vec::new();
        for entry in entries {
            sections.push(format!(
                "### Rolling Output `{ordinal:03}-{key}`\n\n{}",
                truncate_chars(&entry.output, rolling_budget(agent)),
                ordinal = entry.ordinal,
                key = entry.agent_key
            ));
        }
        Some(format!(
            "## Rolling Context Window\n\
             Phase-aware active window containing the most relevant recent accepted outputs.\n\n{}",
            sections.join("\n\n")
        ))
    }

    fn consistency_prescan(
        &self,
        agent: &ResearchAgent,
        entries: &[ResearchRlmEntry],
    ) -> Option<String> {
        if agent.key != "consistency-validator" {
            return None;
        }

        let locked_chapters = entries
            .iter()
            .find(|entry| entry.agent_key == "dissertation-architect")
            .and_then(|entry| extract_total_chapters(&entry.output))
            .unwrap_or(0);

        let mut scanned = 0usize;
        let mut references = 0usize;
        let mut invalid = Vec::new();

        for entry in entries {
            if !is_writer(&entry.agent_key) {
                continue;
            }
            scanned += 1;
            for chapter in chapter_references(&entry.output) {
                references += 1;
                if locked_chapters > 0 && chapter > locked_chapters {
                    invalid.push(format!("{} references Chapter {chapter}", entry.agent_key));
                }
            }
        }

        Some(format!(
            "## Deterministic Consistency Pre-Scan\n\
             Source: Archon RLM accepted outputs for this run.\n\
             Locked chapter count detected: {locked_chapters}\n\
             Documents scanned: {scanned}\n\
             Chapter references found: {references}\n\
             Invalid references found: {}\n\
             Invalid reference details: {}\n",
            invalid.len(),
            if invalid.is_empty() {
                "none".to_string()
            } else {
                invalid.join("; ")
            }
        ))
    }
}

fn pinned_agent_keys(agent: &ResearchAgent) -> Vec<&'static str> {
    let mut keys = vec!["dissertation-architect"];
    if agent.phase >= 6 {
        keys.extend([
            "evidence-synthesizer",
            "methodology-writer",
            "apa-citation-specialist",
        ]);
    }
    if agent.phase >= 8 {
        keys.extend([
            "introduction-writer",
            "literature-review-writer",
            "results-writer",
            "discussion-writer",
            "conclusion-writer",
            "abstract-writer",
        ]);
    }
    keys
}

fn rolling_window_size(agent: &ResearchAgent) -> usize {
    match agent.phase {
        0..=3 => 6,
        4..=5 => 10,
        6..=7 => 14,
        _ => 24,
    }
}

fn namespace_budget(agent: &ResearchAgent) -> usize {
    if agent.phase >= 8 { 40_000 } else { 10_000 }
}

fn pinned_budget(agent: &ResearchAgent) -> usize {
    if agent.phase >= 8 { 60_000 } else { 16_000 }
}

fn rolling_budget(agent: &ResearchAgent) -> usize {
    if agent.phase >= 8 { 40_000 } else { 8_000 }
}

pub fn research_output_namespaces(agent: &ResearchAgent) -> Vec<String> {
    let mut namespaces = agent
        .memory_keys
        .iter()
        .map(|key| (*key).to_string())
        .collect::<Vec<_>>();
    namespaces.push(format!("research/outputs/{}", agent.key));
    for artifact in agent.output_artifacts {
        namespaces.push(format!("research/artifacts/{artifact}"));
    }
    namespaces
}

pub fn primary_memory_key(agent: &AgentInfo) -> &'static str {
    get_agent_by_key(&agent.key)
        .and_then(|research_agent| research_agent.memory_keys.first().copied())
        .unwrap_or("research/output")
}

fn entries_from_session(session: &PipelineSession) -> Vec<ResearchRlmEntry> {
    session
        .agent_results
        .iter()
        .enumerate()
        .map(|(ordinal, (agent, result))| {
            let research_agent = get_agent_by_key(&agent.key);
            let memory_keys = research_agent
                .map(|agent| {
                    agent
                        .memory_keys
                        .iter()
                        .map(|key| (*key).to_string())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_else(|| vec![primary_memory_key(agent).to_string()]);
            let output_artifacts = research_agent
                .map(|agent| {
                    agent
                        .output_artifacts
                        .iter()
                        .map(|artifact| (*artifact).to_string())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            ResearchRlmEntry {
                ordinal,
                agent_key: agent.key.clone(),
                phase: agent.phase as u8,
                memory_keys,
                output_artifacts,
                token_count: count_tokens(&result.output),
                output: result.output.clone(),
            }
        })
        .collect()
}

fn manifest_files(session: &PipelineSession, entry: &ResearchRlmEntry) -> Vec<String> {
    let prefix = format!(".archon/pipelines/{}/outputs", session.id);
    let stem = format!("{:03}-{}", entry.ordinal, entry.agent_key);
    let mut files = vec![format!("{prefix}/markdown/{stem}.md")];
    for key in &entry.memory_keys {
        files.push(format!("{prefix}/rlm/{}.md", safe_path_segment(key)));
    }
    for artifact in &entry.output_artifacts {
        files.push(format!("{prefix}/artifacts/{stem}/{artifact}"));
        files.push(format!("{prefix}/rlm/research/artifacts/{artifact}.md"));
    }
    files
}

fn safe_path_segment(value: &str) -> String {
    value
        .split('/')
        .filter(|segment| !segment.is_empty() && *segment != "." && *segment != "..")
        .collect::<Vec<_>>()
        .join("/")
}

fn is_writer(agent_key: &str) -> bool {
    matches!(
        agent_key,
        "introduction-writer"
            | "literature-review-writer"
            | "methodology-writer"
            | "results-writer"
            | "discussion-writer"
            | "conclusion-writer"
            | "abstract-writer"
    )
}

fn extract_total_chapters(text: &str) -> Option<usize> {
    let total_re = regex::Regex::new(r"(?i)total\s+chapters\D+(\d+)").ok()?;
    if let Some(caps) = total_re.captures(text) {
        return caps.get(1)?.as_str().parse().ok();
    }
    let chapter_re = regex::Regex::new(r"(?i)chapter\s+(\d+)").ok()?;
    chapter_re
        .captures_iter(text)
        .filter_map(|caps| caps.get(1)?.as_str().parse::<usize>().ok())
        .max()
}

fn chapter_references(text: &str) -> Vec<usize> {
    let Ok(re) = regex::Regex::new(r"(?i)chapter\s+(\d+)") else {
        return Vec::new();
    };
    let mut in_code = false;
    let mut refs = Vec::new();
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if in_code || line.trim_start().starts_with("<!--") || line.trim_start().starts_with("//") {
            continue;
        }
        refs.extend(
            re.captures_iter(line)
                .filter_map(|caps| caps.get(1)?.as_str().parse::<usize>().ok()),
        );
    }
    refs
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max_chars).collect();
    out.push_str("\n... [truncated by Archon research RLM budget]");
    out
}
