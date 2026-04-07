//! Semantic mapper — maps agent outputs to chapters using keyword heuristics.

use std::collections::HashMap;
use super::scanner::AgentOutput;

/// A single agent output mapped to a chapter with a relevance score.
#[derive(Debug, Clone)]
pub struct MappedAgentOutput {
    /// The agent key that produced this output.
    pub agent_key: String,
    /// The text content from the agent output.
    pub content: String,
    /// Heuristic relevance score (higher = better match).
    pub relevance: f64,
}

/// The result of mapping agent outputs to numbered chapters.
#[derive(Debug, Clone)]
pub struct ChapterMapping {
    /// Maps chapter number to the list of agent outputs assigned to it.
    pub mappings: HashMap<u32, Vec<MappedAgentOutput>>,
}

/// Map agent outputs to chapters using keyword matching (heuristic semantic mapping).
///
/// Each agent output is assigned to the chapter whose title best matches the agent
/// key and content. When no chapter matches, the output falls back to the first chapter.
pub fn map_to_chapters(outputs: Vec<AgentOutput>, chapters: &[(u32, String)]) -> ChapterMapping {
    let mut mappings: HashMap<u32, Vec<MappedAgentOutput>> = HashMap::new();

    for output in &outputs {
        let lower_key = output.agent_key.to_lowercase();
        let lower_content = output.content.to_lowercase();

        let mut best_chapter: Option<u32> = None;
        let mut best_score: f64 = 0.0;

        for (num, title) in chapters {
            let lower_title = title.to_lowercase();
            let mut score = 0.0;

            // Check if agent key or content contains words from chapter title
            for word in lower_title.split_whitespace() {
                if word.len() >= 3 && lower_key.contains(word) {
                    score += 2.0;
                }
                if word.len() >= 3 && lower_content.contains(word) {
                    score += 1.0;
                }
            }

            if score > best_score {
                best_score = score;
                best_chapter = Some(*num);
            }
        }

        // Heuristic fallback: if no match, assign to first chapter
        let target = best_chapter.unwrap_or_else(|| {
            chapters.first().map(|(n, _)| *n).unwrap_or(1)
        });

        mappings
            .entry(target)
            .or_default()
            .push(MappedAgentOutput {
                agent_key: output.agent_key.clone(),
                content: output.content.clone(),
                relevance: best_score,
            });
    }

    ChapterMapping { mappings }
}
