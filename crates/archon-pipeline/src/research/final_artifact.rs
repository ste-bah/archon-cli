use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

use super::{final_steps, pdf::write_research_pdf};

#[derive(Clone, Debug)]
pub struct ResearchPaperArtifacts {
    pub markdown_path: PathBuf,
    pub pdf_path: PathBuf,
    pub chapter_paths: Vec<PathBuf>,
    pub markdown_hash: String,
    pub pdf_hash: String,
}

#[derive(Clone, Debug)]
pub struct ResearchPaper {
    pub title: String,
    pub abstract_text: String,
    pub body_markdown: String,
    pub references: Vec<String>,
    pub appendices: Vec<Appendix>,
}

#[derive(Clone, Debug)]
pub struct Appendix {
    pub title: String,
    pub body: String,
}

#[derive(Clone, Debug)]
struct Section {
    title: String,
    body: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SectionKind {
    Abstract,
    References,
    Appendix,
    Body,
}

pub fn artifact_paths(bundle_dir: &Path) -> (PathBuf, PathBuf) {
    let exports = bundle_dir.join("exports");
    (
        exports.join("final-paper.md"),
        exports.join("final-paper.pdf"),
    )
}

pub fn write_final_research_artifacts(
    bundle_dir: &Path,
    final_output: &str,
) -> Result<ResearchPaperArtifacts> {
    let mut paper = ResearchPaper::parse(final_output)?;
    if let Some(refs) = final_steps::bundle_reference_section(bundle_dir) {
        paper.references = normalise_references(&refs);
    }
    let markdown = paper.to_markdown();
    let (markdown_path, pdf_path) = artifact_paths(bundle_dir);
    if let Some(parent) = markdown_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&markdown_path, markdown.as_bytes())
        .with_context(|| format!("write {}", markdown_path.display()))?;
    let chapter_paths = write_chapter_files(bundle_dir, &paper)?;
    write_research_pdf(&pdf_path, &paper)
        .with_context(|| format!("write {}", pdf_path.display()))?;

    let markdown_hash = sha256_hex(markdown.as_bytes());
    let pdf_hash = sha256_hex(&fs::read(&pdf_path)?);
    Ok(ResearchPaperArtifacts {
        markdown_path,
        pdf_path,
        chapter_paths,
        markdown_hash,
        pdf_hash,
    })
}

impl ResearchPaper {
    pub fn parse(raw: &str) -> Result<Self> {
        let input = strip_outer_markdown_fence(raw);
        let input = input.trim();
        let title = extract_title(input).context("final research paper is missing a title")?;
        let sections = parse_sections(input);
        let mut abstract_parts = Vec::new();
        let mut body_sections = Vec::new();
        let mut reference_parts = Vec::new();
        let mut appendices = Vec::new();

        for section in sections {
            match classify_heading(&section.title) {
                SectionKind::Abstract => abstract_parts.push(section.body),
                SectionKind::References => reference_parts.push(section.body),
                SectionKind::Appendix => appendices.push(Appendix {
                    title: normalise_appendix_title(&section.title),
                    body: section.body.trim().to_string(),
                }),
                SectionKind::Body => {
                    if !is_title_heading(&section.title, &title) {
                        body_sections.push(format!(
                            "## {}\n\n{}",
                            section.title.trim(),
                            section.body.trim()
                        ));
                    }
                }
            }
        }

        let abstract_text = abstract_parts.join("\n\n").trim().to_string();
        if abstract_text.is_empty() {
            bail!("final research paper must include an Abstract section");
        }

        let body_markdown = body_sections.join("\n\n").trim().to_string();
        if body_markdown.is_empty() {
            bail!("final research paper must include body sections before References");
        }
        if !has_introduction_section(&body_sections) {
            bail!("final research paper must include an Introduction section");
        }

        let references = normalise_references(&reference_parts.join("\n\n"));
        if references.is_empty() {
            bail!("final research paper must include a non-empty References section");
        }

        Ok(Self {
            title,
            abstract_text,
            body_markdown,
            references,
            appendices,
        })
    }

    pub fn to_markdown(&self) -> String {
        let mut out = String::new();
        out.push_str("# ");
        out.push_str(self.title.trim());
        out.push_str("\n\n## Abstract\n\n");
        out.push_str(self.abstract_text.trim());
        out.push_str("\n\n");
        out.push_str(self.body_markdown.trim());
        out.push_str("\n\n## References\n\n");
        for entry in &self.references {
            out.push_str(entry.trim());
            out.push_str("\n\n");
        }
        for appendix in &self.appendices {
            out.push_str("## ");
            out.push_str(appendix.title.trim());
            out.push_str("\n\n");
            out.push_str(appendix.body.trim());
            out.push_str("\n\n");
        }
        out
    }
}

fn parse_sections(input: &str) -> Vec<Section> {
    let mut sections = Vec::new();
    let mut current_title: Option<String> = None;
    let mut current_body = String::new();

    for line in input.lines() {
        if let Some(title) = markdown_heading_title(line) {
            if let Some(title) = current_title.replace(title) {
                sections.push(Section {
                    title,
                    body: current_body.trim().to_string(),
                });
                current_body.clear();
            }
        } else if current_title.is_some() {
            current_body.push_str(line);
            current_body.push('\n');
        }
    }

    if let Some(title) = current_title {
        sections.push(Section {
            title,
            body: current_body.trim().to_string(),
        });
    }
    sections
}

fn extract_title(input: &str) -> Option<String> {
    for line in input.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("```") {
            continue;
        }
        if let Some(title) = markdown_heading_title(line)
            && !title.is_empty()
        {
            return Some(title);
        }
        return None;
    }
    None
}

fn markdown_heading_title(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    let hashes = trimmed.chars().take_while(|&c| c == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = trimmed[hashes..].trim();
    if rest.is_empty() {
        None
    } else {
        Some(rest.trim_matches('#').trim().to_string())
    }
}

fn classify_heading(title: &str) -> SectionKind {
    let normal = normalise_heading(title);
    if normal == "abstract" {
        SectionKind::Abstract
    } else if matches!(
        normal.as_str(),
        "references" | "reference list" | "bibliography" | "works cited"
    ) {
        SectionKind::References
    } else if normal.starts_with("appendix") || normal.starts_with("appendices") {
        SectionKind::Appendix
    } else {
        SectionKind::Body
    }
}

fn normalise_heading(title: &str) -> String {
    let mut s = title.trim().to_ascii_lowercase();
    while let Some(first) = s.chars().next() {
        if first.is_ascii_digit() || matches!(first, '.' | ')' | ':' | '-' | ' ') {
            s.remove(0);
        } else {
            break;
        }
    }
    s.trim().to_string()
}

fn normalise_appendix_title(title: &str) -> String {
    let trimmed = title.trim();
    if trimmed.to_ascii_lowercase().starts_with("appendix") {
        trimmed.to_string()
    } else {
        format!("Appendix: {trimmed}")
    }
}

fn is_title_heading(title: &str, paper_title: &str) -> bool {
    title.trim().eq_ignore_ascii_case(paper_title.trim())
}

fn write_chapter_files(bundle_dir: &Path, paper: &ResearchPaper) -> Result<Vec<PathBuf>> {
    let chapters_dir = bundle_dir.join("exports").join("chapters");
    if chapters_dir.exists() {
        fs::remove_dir_all(&chapters_dir)?;
    }
    fs::create_dir_all(&chapters_dir)?;
    let mut paths = Vec::new();
    let sections = parse_sections(&paper.body_markdown)
        .into_iter()
        .filter(|section| classify_heading(&section.title) == SectionKind::Body)
        .collect::<Vec<_>>();
    for (idx, section) in group_chapter_sections(sections).into_iter().enumerate() {
        let file_name = format!("{:02}-{}.md", idx + 1, slug(&section.title));
        let path = chapters_dir.join(file_name);
        let content = format!("## {}\n\n{}\n", section.title.trim(), section.body.trim());
        fs::write(&path, content.as_bytes())
            .with_context(|| format!("write {}", path.display()))?;
        paths.push(path);
    }
    Ok(paths)
}

fn group_chapter_sections(sections: Vec<Section>) -> Vec<Section> {
    if !sections
        .iter()
        .any(|section| is_numbered_chapter_heading(&section.title))
    {
        return sections;
    }
    let mut grouped = Vec::new();
    let mut current: Option<Section> = None;
    for section in sections {
        if is_numbered_chapter_heading(&section.title) {
            if let Some(chapter) = current.replace(section) {
                grouped.push(chapter);
            }
        } else if let Some(chapter) = current.as_mut() {
            if !chapter.body.trim().is_empty() {
                chapter.body.push_str("\n\n");
            }
            chapter.body.push_str("### ");
            chapter.body.push_str(section.title.trim());
            chapter.body.push_str("\n\n");
            chapter.body.push_str(section.body.trim());
        } else {
            grouped.push(section);
        }
    }
    if let Some(chapter) = current {
        grouped.push(chapter);
    }
    grouped
}

fn is_numbered_chapter_heading(title: &str) -> bool {
    let trimmed = title.trim();
    if let Some((prefix, rest)) = trimmed.split_once('.') {
        return prefix.chars().all(|c| c.is_ascii_digit())
            && rest
                .trim_start()
                .chars()
                .next()
                .is_some_and(|c| !c.is_ascii_digit());
    }
    let normal = normalise_heading(title);
    normal
        .strip_prefix("chapter")
        .and_then(|rest| rest.trim_start().chars().next())
        .is_some_and(|c| c.is_ascii_digit())
}

fn slug(title: &str) -> String {
    let mut out = title
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    while out.contains("--") {
        out = out.replace("--", "-");
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        "chapter".to_string()
    } else {
        trimmed.to_string()
    }
}

fn has_introduction_section(body_sections: &[String]) -> bool {
    body_sections.iter().any(|section| {
        section
            .lines()
            .next()
            .map(|line| is_introduction_heading(line.trim_start_matches('#')))
            .unwrap_or(false)
    })
}

fn is_introduction_heading(title: &str) -> bool {
    let normal = normalise_heading(title);
    if normal.starts_with("introduction") {
        return true;
    }
    let Some(stripped) = normal.strip_prefix("chapter") else {
        return false;
    };
    let stripped = stripped.trim_start().trim_start_matches(|c: char| {
        c.is_ascii_digit() || matches!(c, '.' | ')' | ':' | '-' | ' ')
    });
    stripped.trim().starts_with("introduction")
}

fn normalise_references(input: &str) -> Vec<String> {
    let mut entries = split_reference_entries(input);
    let mut seen = HashSet::new();
    entries.retain(|entry| seen.insert(entry.to_ascii_lowercase()));
    entries.sort_by_key(|entry| entry.to_ascii_lowercase());
    entries
}

fn split_reference_entries(input: &str) -> Vec<String> {
    let mut entries = Vec::new();
    let mut current = String::new();
    for line in input.lines() {
        if markdown_heading_title(line).is_some() {
            continue;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            push_reference_entry(&mut entries, &mut current);
        } else {
            let cleaned = strip_reference_marker(trimmed);
            if !current.is_empty() && starts_new_reference(line, cleaned) {
                push_reference_entry(&mut entries, &mut current);
            }
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(cleaned);
        }
    }
    push_reference_entry(&mut entries, &mut current);
    entries
}

fn strip_reference_marker(line: &str) -> &str {
    let stripped = line
        .trim_start_matches("- ")
        .trim_start_matches("* ")
        .trim();
    let Some((prefix, rest)) = stripped.split_once(". ") else {
        return stripped;
    };
    if prefix.chars().all(|c| c.is_ascii_digit()) {
        rest.trim()
    } else {
        stripped
    }
}

fn starts_new_reference(raw_line: &str, cleaned: &str) -> bool {
    let trimmed_start = raw_line.trim_start();
    if trimmed_start.starts_with("- ") || trimmed_start.starts_with("* ") {
        return true;
    }
    if raw_line
        .chars()
        .next()
        .map(|c| c.is_whitespace())
        .unwrap_or(false)
    {
        return false;
    }
    looks_like_reference_start(cleaned)
}

fn looks_like_reference_start(entry: &str) -> bool {
    let prefix: String = entry.chars().take(160).collect();
    prefix.contains("(19")
        || prefix.contains("(20")
        || prefix.contains("(n.d.)")
        || prefix.contains("(in press)")
}

fn push_reference_entry(entries: &mut Vec<String>, current: &mut String) {
    let entry = current.trim();
    if !entry.is_empty() {
        entries.push(entry.to_string());
    }
    current.clear();
}

fn strip_outer_markdown_fence(input: &str) -> String {
    let trimmed = input.trim();
    if !trimmed.starts_with("```") {
        return input.to_string();
    }
    let mut lines = trimmed.lines();
    let first = lines.next().unwrap_or_default();
    if !first.starts_with("```") {
        return input.to_string();
    }
    let body = lines.collect::<Vec<_>>().join("\n");
    if let Some(stripped) = body.strip_suffix("```") {
        stripped.trim().to_string()
    } else {
        input.to_string()
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}
