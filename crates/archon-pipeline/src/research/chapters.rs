//! Chapter structure loading and dynamic writing agent generation.
//!
//! Implements `ChapterStructureLoader` (multi-format parsing with legacy field
//! normalization) and `DynamicAgentGenerator` (one writing agent per chapter).

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChapterDefinition {
    pub number: u32,
    pub title: String,
    pub writer_agent: String,
    pub target_words: u32,
    pub sections: Vec<String>,
    pub output_file: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChapterStructure {
    pub locked: bool,
    pub generated_at: String,
    pub total_chapters: u32,
    pub estimated_total_words: u32,
    pub chapters: Vec<ChapterDefinition>,
    pub writer_mapping: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct DynamicWritingAgent {
    pub agent_key: String,
    pub chapter_number: u32,
    pub chapter_title: String,
    pub sections: Vec<String>,
    pub target_words: u32,
    pub output_path: String,
    pub prompt: String,
    pub tool_access: Vec<String>,
}

#[derive(Debug)]
pub enum ChapterStructureError {
    NotFound { path: String },
    NotLocked,
    InvalidDefinition { index: usize, field: String },
    ParseError(String),
}

// ---------------------------------------------------------------------------
// ChapterStructureLoader
// ---------------------------------------------------------------------------

pub struct ChapterStructureLoader;

impl ChapterStructureLoader {
    pub fn new() -> Self {
        Self
    }

    /// Parse content trying: 1) JSON code block, 2) raw JSON, 3) markdown fallback.
    pub fn parse_structure(content: &str) -> Result<ChapterStructure, ChapterStructureError> {
        let json_block_re = Regex::new(r"```json\s*([\s\S]*?)\s*```").unwrap();

        // 1) Try JSON code block
        if let Some(caps) = json_block_re.captures(content) {
            let json_str = caps.get(1).unwrap().as_str();
            let val: serde_json::Value = serde_json::from_str(json_str)
                .map_err(|e| ChapterStructureError::ParseError(e.to_string()))?;
            return Self::normalize_structure(val);
        }

        // 2) Try raw JSON — find first '{' to last '}'
        if let Some(start) = content.find('{') {
            if let Some(end) = content.rfind('}') {
                if end > start {
                    let json_str = &content[start..=end];
                    if let Ok(val) = serde_json::from_str::<serde_json::Value>(json_str) {
                        return Self::normalize_structure(val);
                    }
                }
            }
        }

        // 3) Markdown fallback
        Self::parse_from_markdown(content)
    }

    /// Normalize a raw `serde_json::Value` with legacy field handling.
    pub fn normalize_structure(
        raw: serde_json::Value,
    ) -> Result<ChapterStructure, ChapterStructureError> {
        let obj = raw
            .as_object()
            .ok_or_else(|| ChapterStructureError::ParseError("expected JSON object".into()))?;

        // Check locked
        let locked = obj
            .get("locked")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if !locked {
            return Err(ChapterStructureError::NotLocked);
        }

        // generated_at with legacy fallbacks
        let generated_at = obj
            .get("generated_at")
            .or_else(|| obj.get("generatedAt"))
            .or_else(|| obj.get("dateLocked"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        // total_chapters
        let total_chapters = obj
            .get("total_chapters")
            .or_else(|| obj.get("totalChapters"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;

        // estimated_total_words
        let estimated_total_words = obj
            .get("estimated_total_words")
            .or_else(|| obj.get("estimatedTotalWords"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;

        // writer_mapping
        let writer_mapping = obj
            .get("writer_mapping")
            .or_else(|| obj.get("writerMapping"))
            .and_then(|v| v.as_object())
            .map(|m| {
                m.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect::<HashMap<String, String>>()
            })
            .unwrap_or_default();

        // chapters
        let chapters_arr = obj
            .get("chapters")
            .and_then(|v| v.as_array())
            .ok_or_else(|| ChapterStructureError::ParseError("missing chapters array".into()))?;

        let mut chapters = Vec::with_capacity(chapters_arr.len());
        for (i, ch_val) in chapters_arr.iter().enumerate() {
            let ch_obj = ch_val.as_object().ok_or_else(|| {
                ChapterStructureError::ParseError(format!("chapter {} is not an object", i))
            })?;

            let number = ch_obj
                .get("number")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u32;

            let title = ch_obj
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            if title.is_empty() {
                return Err(ChapterStructureError::InvalidDefinition {
                    index: i,
                    field: "title".to_string(),
                });
            }

            let writer_agent = ch_obj
                .get("writer_agent")
                .or_else(|| ch_obj.get("writerAgent"))
                .or_else(|| ch_obj.get("assignedAgent"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let target_words = ch_obj
                .get("target_words")
                .or_else(|| ch_obj.get("targetWords"))
                .or_else(|| ch_obj.get("wordTarget"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u32;

            let sections = ch_obj
                .get("sections")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect::<Vec<String>>()
                })
                .unwrap_or_default();

            let output_file = ch_obj
                .get("output_file")
                .or_else(|| ch_obj.get("outputFile"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let def = ChapterDefinition {
                number,
                title,
                writer_agent,
                target_words,
                sections,
                output_file,
            };

            Self::validate_chapter(&def, i)?;
            chapters.push(def);
        }

        Ok(ChapterStructure {
            locked,
            generated_at,
            total_chapters,
            estimated_total_words,
            chapters,
            writer_mapping,
        })
    }

    /// Parse markdown fallback format (`### Chapter N: Title`).
    pub fn parse_from_markdown(
        content: &str,
    ) -> Result<ChapterStructure, ChapterStructureError> {
        let heading_re = Regex::new(r"### Chapter (\d+): (.+)").unwrap();
        let content_re =
            Regex::new(r"(?i)\*\*Content(?:\s+Outline)?:\*\*\s*(.+)").unwrap();
        let words_re = Regex::new(r"(?i)\*\*Word Count Target:\*\*\s*(\d+)").unwrap();

        let lines: Vec<&str> = content.lines().collect();
        let mut chapters: Vec<ChapterDefinition> = Vec::new();

        let mut i = 0;
        while i < lines.len() {
            if let Some(caps) = heading_re.captures(lines[i]) {
                let number: u32 = caps[1].parse().unwrap_or(0);
                let title = caps[2].trim().to_string();

                let mut sections = Vec::new();
                let mut target_words: u32 = 0;

                // Scan subsequent lines for metadata until next heading or EOF
                let mut j = i + 1;
                while j < lines.len() && !lines[j].starts_with("### ") {
                    if let Some(c) = content_re.captures(lines[j]) {
                        sections = c[1]
                            .split(',')
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .collect();
                    }
                    if let Some(w) = words_re.captures(lines[j]) {
                        target_words = w[1].parse().unwrap_or(0);
                    }
                    j += 1;
                }

                let writer_agent = Self::infer_writer_agent(number, &title);
                let output_file = format!("chapter-{:02}.md", number);

                chapters.push(ChapterDefinition {
                    number,
                    title,
                    writer_agent,
                    target_words,
                    sections,
                    output_file,
                });

                i = j;
            } else {
                i += 1;
            }
        }

        if chapters.is_empty() {
            return Err(ChapterStructureError::ParseError(
                "no chapters found in markdown".into(),
            ));
        }

        let total_chapters = chapters.len() as u32;
        let estimated_total_words: u32 = chapters.iter().map(|c| c.target_words).sum();

        Ok(ChapterStructure {
            locked: true,
            generated_at: String::new(),
            total_chapters,
            estimated_total_words,
            chapters,
            writer_mapping: HashMap::new(),
        })
    }

    /// Infer writer agent from chapter title (case-insensitive).
    pub fn infer_writer_agent(number: u32, title: &str) -> String {
        let lower = title.to_lowercase();
        let _ = number; // number unused but part of the API

        if lower.contains("introduction") {
            "introduction-writer".to_string()
        } else if lower.contains("literature") {
            "literature-review-writer".to_string()
        } else if lower.contains("method") {
            "methodology-writer".to_string()
        } else if lower.contains("result") || lower.contains("finding") {
            "results-writer".to_string()
        } else if lower.contains("discussion") {
            "discussion-writer".to_string()
        } else if lower.contains("conclusion") {
            "conclusion-writer".to_string()
        } else if lower.contains("abstract") {
            "abstract-writer".to_string()
        } else {
            "chapter-synthesizer".to_string()
        }
    }

    /// Validate a chapter definition.
    pub fn validate_chapter(
        chapter: &ChapterDefinition,
        index: usize,
    ) -> Result<(), ChapterStructureError> {
        if chapter.title.is_empty() {
            return Err(ChapterStructureError::InvalidDefinition {
                index,
                field: "title".to_string(),
            });
        }
        if chapter.output_file.is_empty() {
            return Err(ChapterStructureError::InvalidDefinition {
                index,
                field: "output_file".to_string(),
            });
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// DynamicAgentGenerator
// ---------------------------------------------------------------------------

pub struct DynamicAgentGenerator;

impl DynamicAgentGenerator {
    pub fn new() -> Self {
        Self
    }

    /// Generate one `DynamicWritingAgent` per chapter in the structure.
    pub fn generate_writing_agents(structure: &ChapterStructure) -> Vec<DynamicWritingAgent> {
        structure
            .chapters
            .iter()
            .map(|ch| DynamicWritingAgent {
                agent_key: ch.writer_agent.clone(),
                chapter_number: ch.number,
                chapter_title: ch.title.clone(),
                sections: ch.sections.clone(),
                target_words: ch.target_words,
                output_path: ch.output_file.clone(),
                prompt: format!(
                    "Write Chapter {}: {}\n\nSections: {}\nTarget: {} words\nOutput: {}",
                    ch.number,
                    ch.title,
                    ch.sections.join(", "),
                    ch.target_words,
                    ch.output_file,
                ),
                tool_access: vec![
                    "Write".to_string(),
                    "Read".to_string(),
                    "Glob".to_string(),
                    "Grep".to_string(),
                    "WebSearch".to_string(),
                    "WebFetch".to_string(),
                ],
            })
            .collect()
    }
}
