//! Dynamic Phase 8 assembly for the research pipeline.

use anyhow::{Context, Result};

use crate::runner::{
    AgentInfo, PipelineResult, PipelineSession, PipelineType, QualityScore, ToolAccessLevel,
};

use super::chapters::{ChapterDefinition, ChapterStructure, ChapterStructureLoader};
use super::{final_appendix, final_steps};

pub const STATIC_AGENTS_BEFORE_FINAL: usize = 46;
pub const SCANNER_KEY: &str = "final-stage-scanner";
pub const MAPPER_KEY: &str = "final-stage-mapper";
pub const COMBINER_KEY: &str = "final-paper-combiner";
pub const VALIDATOR_KEY: &str = "final-paper-validator";

pub fn is_dynamic_chapter_key(key: &str) -> bool {
    key.starts_with("chapter-writer-")
}

pub fn is_dynamic_final_key(key: &str) -> bool {
    matches!(key, SCANNER_KEY | MAPPER_KEY | COMBINER_KEY | VALIDATOR_KEY)
        || is_dynamic_chapter_key(key)
}

pub fn next_final_stage_agent(session: &PipelineSession) -> Result<Option<AgentInfo>> {
    let Some(structure) = structure_from_session(session).unwrap_or(None) else {
        return Ok(None);
    };
    if !has_result(session, SCANNER_KEY) {
        return Ok(Some(stage_agent(SCANNER_KEY, "Final Stage Scanner", false)));
    }
    if !has_result(session, MAPPER_KEY) {
        return Ok(Some(stage_agent(MAPPER_KEY, "Final Stage Mapper", false)));
    }

    let completed_chapters = session
        .agent_results
        .iter()
        .filter(|(agent, _)| is_dynamic_chapter_key(&agent.key))
        .count();
    if let Some(chapter) = structure.chapters.get(completed_chapters) {
        return Ok(Some(chapter_agent(chapter)));
    }

    if !has_result(session, COMBINER_KEY) {
        return Ok(Some(stage_agent(
            COMBINER_KEY,
            "Final Paper Combiner",
            true,
        )));
    }
    if !has_result(session, VALIDATOR_KEY) {
        return Ok(Some(stage_agent(
            VALIDATOR_KEY,
            "Final Paper Validator",
            true,
        )));
    }
    Ok(None)
}

pub fn build_final_stage_prompt(
    session: &PipelineSession,
    agent_key: &str,
    task: &str,
    style_prompt: Option<&str>,
) -> Result<String> {
    match agent_key {
        SCANNER_KEY => Ok(scanner_prompt(session, task)),
        MAPPER_KEY => mapper_prompt(session, task),
        COMBINER_KEY => combiner_prompt(session, task, style_prompt),
        VALIDATOR_KEY => validator_prompt(session),
        key if is_dynamic_chapter_key(key) => chapter_prompt(session, key, task, style_prompt),
        _ => anyhow::bail!("unknown final-stage agent: {agent_key}"),
    }
}

pub fn score_final_stage_output(
    session: &PipelineSession,
    agent_key: &str,
    output: &str,
) -> QualityScore {
    match agent_key {
        SCANNER_KEY => final_steps::score_report(
            output,
            700,
            &["source inventory", "chapter", "evidence", "gap"],
        ),
        MAPPER_KEY => {
            final_steps::score_report(output, 700, &["mapping", "chapter", "source", "coverage"])
        }
        COMBINER_KEY => final_steps::score_combiner_output(output),
        VALIDATOR_KEY => final_steps::score_validator_output(output),
        key if is_dynamic_chapter_key(key) => {
            let target = structure_from_session(session)
                .ok()
                .flatten()
                .and_then(|structure| {
                    chapter_number_from_key(key).and_then(|number| {
                        structure
                            .chapters
                            .iter()
                            .find(|chapter| chapter.number == number)
                            .map(practical_min_words)
                    })
                })
                .unwrap_or(1_000);
            final_steps::score_chapter_output(target, output)
        }
        _ => final_steps::score_report(output, 400, &["research"]),
    }
}

pub fn assemble_result(session: PipelineSession) -> Result<PipelineResult> {
    let final_output = assemble_final_paper(&session)
        .or_else(|| {
            session
                .agent_results
                .iter()
                .find(|(agent, _)| agent.key == COMBINER_KEY)
                .map(|(_, result)| result.output.clone())
        })
        .or_else(|| {
            session
                .agent_results
                .last()
                .map(|(_, result)| result.output.clone())
        })
        .unwrap_or_else(|| "No agent output produced.".to_string());
    let total_cost = session.agent_results.iter().map(|(_, r)| r.cost_usd).sum();
    Ok(PipelineResult {
        session_id: session.id,
        pipeline_type: PipelineType::Research,
        agent_results: session.agent_results,
        total_cost_usd: total_cost,
        duration: session.started_at.elapsed(),
        final_output,
    })
}

pub fn assemble_final_paper(session: &PipelineSession) -> Option<String> {
    let structure = structure_from_session(session).ok().flatten()?;
    let mut chapter_outputs: Vec<_> = session
        .agent_results
        .iter()
        .filter(|(agent, _)| is_dynamic_chapter_key(&agent.key))
        .filter_map(|(agent, result)| chapter_number_from_key(&agent.key).map(|n| (n, result)))
        .collect();
    if chapter_outputs.is_empty() {
        return None;
    }
    chapter_outputs.sort_by_key(|(number, _)| *number);

    let mut out = format!("# {}\n\n", paper_title(session, &structure));
    out.push_str("## Abstract\n\n");
    out.push_str(&abstract_context(session));
    out.push('\n');
    for (number, result) in chapter_outputs {
        if let Some(chapter) = structure
            .chapters
            .iter()
            .find(|chapter| chapter.number == number)
        {
            out.push_str(&format!("\n## {}. {}\n\n", chapter.number, chapter.title));
            out.push_str(&final_steps::clean_chapter_body(&result.output));
            out.push('\n');
        }
    }
    out.push_str("\n## References\n\n");
    out.push_str(&reference_context(session));
    out.push_str("\n\n");
    out.push_str(&final_appendix::appendices(session, &structure));
    Some(out)
}

fn scanner_prompt(session: &PipelineSession, task: &str) -> String {
    format!(
        "# Final Stage Scanner\n\nResearch topic: {task}\n\n\
         Build a source inventory for final paper assembly from the accepted outputs below. \
         Group evidence by chapter relevance, identify missing material, and flag outputs \
         that must not be treated as chapter prose.\n\n{}\n\n\
         Required headings: Source Inventory, Chapter Evidence Coverage, Missing Evidence, \
         Material Excluded From Final Prose.",
        accepted_output_manifest(session, 4_000)
    )
}

fn mapper_prompt(session: &PipelineSession, task: &str) -> Result<String> {
    let structure =
        structure_from_session(session)?.context("missing dissertation architecture")?;
    Ok(format!(
        "# Final Stage Mapper\n\nResearch topic: {task}\n\n\
         Locked chapter architecture:\n{}\n\n\
         Scanner output:\n{}\n\n\
         Create a chapter-by-chapter source map. For each chapter list the accepted outputs \
         to use, key claims to carry forward, citation needs, and unresolved gaps. \
         Do not write chapter prose yet.",
        chapter_plan(&structure),
        result_text(session, SCANNER_KEY).unwrap_or("Scanner output unavailable."),
    ))
}

fn chapter_prompt(
    session: &PipelineSession,
    agent_key: &str,
    task: &str,
    style_prompt: Option<&str>,
) -> Result<String> {
    let structure =
        structure_from_session(session)?.context("missing dissertation architecture")?;
    let number = chapter_number_from_key(agent_key).context("invalid dynamic chapter key")?;
    let chapter = structure
        .chapters
        .iter()
        .find(|chapter| chapter.number == number)
        .context("chapter definition not found")?;
    let style = style_prompt
        .unwrap_or("Use UK English, formal academic prose, APA 7 citations, and no contractions.");

    Ok(format!(
        "# Chapter Writer Task\n\nResearch topic: {task}\n\n\
         Locked paper title: {}\n\
         Chapter {}: {}\n\
         Target length: {} words, with a practical minimum of {} words for this pipeline pass.\n\n\
         Required sections:\n{}\n\nStyle requirements:\n{style}\n\n\
         Final-stage mapping:\n{}\n\nSource material:\n{}\n\nReferences:\n{}\n\n\
         Write only this chapter in Markdown. Start with `# Chapter {}: {}`. \
         Produce sustained academic prose, not notes. Do not include a References section; \
         the combiner will add exactly one consolidated References section. \
         Do not include pipeline status, memory notes, file paths, task summaries, or QA verdicts.",
        paper_title(session, &structure),
        chapter.number,
        chapter.title,
        chapter.target_words,
        practical_min_words(chapter),
        chapter_sections(chapter),
        result_text(session, MAPPER_KEY).unwrap_or("No final-stage mapping output available."),
        source_context_for_chapter(session, chapter),
        reference_context(session),
        chapter.number,
        chapter.title,
    ))
}

fn combiner_prompt(
    session: &PipelineSession,
    task: &str,
    style_prompt: Option<&str>,
) -> Result<String> {
    let structure =
        structure_from_session(session)?.context("missing dissertation architecture")?;
    Ok(format!(
        "# Final Paper Combiner\n\nResearch topic: {task}\n\n\
         Compose a complete university-standard research paper from the accepted chapter outputs. \
         Use APA 7 style, one title, one Abstract, ordered chapters, exactly one References \
         section at the end, and appendices after References. Do not include memory notes, \
         pipeline status, artifact lists, QA verdicts, or file paths.\n\n\
         Style override:\n{}\n\nLocked architecture:\n{}\n\nChapter outputs:\n{}\n\n\
         Authoritative references:\n{}\n\nAppendix requirement: include Appendix A for the primary HLD architecture source.",
        style_prompt.unwrap_or("UK English academic prose."),
        chapter_plan(&structure),
        dynamic_chapter_outputs(session),
        reference_context(session),
    ))
}

fn validator_prompt(session: &PipelineSession) -> Result<String> {
    Ok(format!(
        "# Final Paper Validator\n\n\
         Validate the final combined paper below. Return PASS only if it has a title, Abstract, \
         ordered chapters, one References section, appendix material, coherent citations, and \
         no pipeline/status garbage. If there is a blocking issue, return FAIL with exact fixes.\n\n{}",
        assemble_final_paper(session)
            .as_deref()
            .or_else(|| result_text(session, COMBINER_KEY))
            .unwrap_or("Final combiner output unavailable."),
    ))
}

fn stage_agent(key: &str, display_name: &str, critical: bool) -> AgentInfo {
    AgentInfo {
        key: key.to_string(),
        display_name: display_name.to_string(),
        model: "sonnet".to_string(),
        phase: 8,
        critical,
        parallelizable: false,
        quality_threshold: 0.50,
        tool_access_level: ToolAccessLevel::Full,
    }
}

fn chapter_agent(chapter: &ChapterDefinition) -> AgentInfo {
    let slug = final_steps::slug(&chapter.title);
    let key = format!("chapter-writer-{:03}-{slug}", chapter.number);
    let display = format!("Chapter {} Writer: {}", chapter.number, chapter.title);
    stage_agent(&key, &display, true)
}

fn structure_from_session(session: &PipelineSession) -> Result<Option<ChapterStructure>> {
    let Some((_, result)) = session
        .agent_results
        .iter()
        .find(|(agent, _)| agent.key == "dissertation-architect")
    else {
        return Ok(None);
    };
    Ok(Some(
        ChapterStructureLoader::parse_structure(&result.output)
            .map_err(|e| anyhow::anyhow!("failed to parse dissertation architecture: {e:?}"))?,
    ))
}

fn source_context_for_chapter(session: &PipelineSession, chapter: &ChapterDefinition) -> String {
    let title = chapter.title.to_ascii_lowercase();
    let mut scored = session
        .agent_results
        .iter()
        .filter(|(agent, _)| !is_dynamic_chapter_key(&agent.key))
        .map(|(agent, result)| {
            let haystack = format!("{} {}", agent.key, result.output).to_ascii_lowercase();
            let score = title
                .split_whitespace()
                .filter(|word| word.len() > 3 && haystack.contains(*word))
                .count();
            (score, agent.key.as_str(), result.output.as_str())
        })
        .collect::<Vec<_>>();
    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored
        .into_iter()
        .filter(|(score, key, _)| {
            *score > 0 || key.contains("writer") || *key == "citation-reconciler"
        })
        .take(10)
        .map(|(_, key, output)| {
            format!(
                "### {key}\n\n{}",
                final_steps::truncate_chars(output, 8_000)
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n---\n\n")
}

fn reference_context(session: &PipelineSession) -> String {
    let outputs = session
        .agent_results
        .iter()
        .map(|(agent, result)| (agent.key.as_str(), result.output.as_str()))
        .collect::<Vec<_>>();
    final_steps::best_reference_section(&outputs)
        .unwrap_or_else(final_steps::fallback_hld_reference)
}

fn abstract_context(session: &PipelineSession) -> String {
    session
        .agent_results
        .iter()
        .find(|(agent, _)| agent.key == "abstract-writer")
        .and_then(|(_, result)| final_steps::extract_abstract_section(&result.output))
        .unwrap_or_else(|| {
            "This paper examines GKB match scoring and proprietary disposition algorithm design."
                .into()
        })
}

fn dynamic_chapter_outputs(session: &PipelineSession) -> String {
    let mut chapters = session
        .agent_results
        .iter()
        .filter(|(agent, _)| is_dynamic_chapter_key(&agent.key))
        .filter_map(|(agent, result)| chapter_number_from_key(&agent.key).map(|n| (n, result)))
        .collect::<Vec<_>>();
    chapters.sort_by_key(|(number, _)| *number);
    chapters
        .into_iter()
        .map(|(number, result)| format!("## Chapter {number}\n\n{}", result.output))
        .collect::<Vec<_>>()
        .join("\n\n---\n\n")
}

fn accepted_output_manifest(session: &PipelineSession, max_each: usize) -> String {
    session
        .agent_results
        .iter()
        .map(|(agent, result)| {
            format!(
                "### {}\n\n{}",
                agent.key,
                final_steps::truncate_chars(&result.output, max_each)
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n---\n\n")
}

fn chapter_plan(structure: &ChapterStructure) -> String {
    structure
        .chapters
        .iter()
        .map(|chapter| {
            format!(
                "- Chapter {}: {} ({} words)\n{}",
                chapter.number,
                chapter.title,
                chapter.target_words,
                chapter_sections(chapter)
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn chapter_sections(chapter: &ChapterDefinition) -> String {
    if chapter.sections.is_empty() {
        "- Develop the chapter according to the locked dissertation architecture.".into()
    } else {
        chapter
            .sections
            .iter()
            .map(|section| format!("  - {section}"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn result_text<'a>(session: &'a PipelineSession, key: &str) -> Option<&'a str> {
    session
        .agent_results
        .iter()
        .find(|(agent, _)| agent.key == key)
        .map(|(_, result)| result.output.as_str())
}

fn has_result(session: &PipelineSession, key: &str) -> bool {
    session
        .agent_results
        .iter()
        .any(|(agent, _)| agent.key == key)
}

fn paper_title(session: &PipelineSession, _structure: &ChapterStructure) -> String {
    if let Some(title) = architect_title(session) {
        return title;
    }
    let task = session.task.trim();
    let title = task
        .strip_prefix("Write a research paper about ")
        .or_else(|| task.strip_prefix("write a research paper about "))
        .unwrap_or(task)
        .split('.')
        .next()
        .unwrap_or(task)
        .trim();
    if !title.is_empty() {
        return final_steps::truncate_chars(title, 140).replace('\n', " ");
    }
    "Research Paper".to_string()
}

fn architect_title(session: &PipelineSession) -> Option<String> {
    let output = session
        .agent_results
        .iter()
        .find(|(agent, _)| agent.key == "dissertation-architect")
        .map(|(_, result)| result.output.as_str())?;
    for line in output.lines().take(80) {
        let trimmed = line.trim();
        for label in [
            "**Research Title**:",
            "**Paper Title**:",
            "Research Title:",
            "Paper Title:",
        ] {
            if let Some(title) = trimmed.strip_prefix(label) {
                let title = title.trim().trim_matches('*').trim();
                if !title.is_empty() {
                    return Some(title.to_string());
                }
            }
        }
    }
    None
}

fn practical_min_words(chapter: &ChapterDefinition) -> usize {
    ((chapter.target_words as usize) / 3).clamp(1_000, 2_000)
}

fn chapter_number_from_key(key: &str) -> Option<u32> {
    key.strip_prefix("chapter-writer-")?.get(..3)?.parse().ok()
}
