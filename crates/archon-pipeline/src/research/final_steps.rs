use std::collections::HashMap;
use std::fs;
use std::path::Path;

use crate::runner::QualityScore;

pub fn score_report(output: &str, min_words: usize, required: &[&str]) -> QualityScore {
    let words = count_words(output);
    let lower = output.to_ascii_lowercase();
    let required_hits = required
        .iter()
        .filter(|term| lower.contains(&term.to_ascii_lowercase()))
        .count();
    let mut score: f64 = 0.20;
    if words >= min_words {
        score += 0.35;
    } else if words >= min_words / 2 {
        score += 0.15;
    }
    score += 0.30 * (required_hits as f64 / required.len().max(1) as f64);
    if !contains_status_junk(&lower) {
        score += 0.10;
    }

    let mut dimensions = HashMap::new();
    dimensions.insert(
        "word_count".to_string(),
        (words as f64 / min_words.max(1) as f64).min(1.0),
    );
    dimensions.insert(
        "required_terms".to_string(),
        required_hits as f64 / required.len().max(1) as f64,
    );
    dimensions.insert(
        "artifact_hygiene".to_string(),
        if contains_status_junk(&lower) {
            0.0
        } else {
            1.0
        },
    );
    QualityScore {
        overall: score.min(0.95),
        dimensions,
    }
}

pub fn score_chapter_output(target: usize, output: &str) -> QualityScore {
    let words = count_words(output);
    let lower = output.to_ascii_lowercase();
    let has_heading = lower.contains("# chapter") || lower.contains("## chapter");
    let has_citation = output.contains(", 20") || output.contains("(20") || output.contains("n.d.");
    let no_reference_section = !lower.contains("## references") && !lower.contains("# references");
    let no_artifacts = !contains_status_junk(&lower);

    let mut score: f64 = 0.15;
    if words >= target {
        score += 0.45;
    } else if words >= target / 2 {
        score += 0.20;
    }
    if has_heading {
        score += 0.10;
    }
    if has_citation {
        score += 0.15;
    }
    if no_reference_section {
        score += 0.05;
    }
    if no_artifacts {
        score += 0.05;
    }

    let mut dimensions = HashMap::new();
    dimensions.insert(
        "word_count".to_string(),
        (words as f64 / target.max(1) as f64).min(1.0),
    );
    dimensions.insert(
        "chapter_structure".to_string(),
        if has_heading { 1.0 } else { 0.0 },
    );
    dimensions.insert(
        "citation_use".to_string(),
        if has_citation { 1.0 } else { 0.0 },
    );
    dimensions.insert(
        "artifact_hygiene".to_string(),
        if no_artifacts { 1.0 } else { 0.0 },
    );
    QualityScore {
        overall: score.min(0.95),
        dimensions,
    }
}

pub fn score_combiner_output(output: &str) -> QualityScore {
    let words = count_words(output);
    let lower = output.to_ascii_lowercase();
    let checks = [
        lower.contains("## abstract"),
        lower.contains("introduction"),
        lower.contains("## references"),
        lower.contains("appendix"),
        lower.matches("## ").count() >= 5,
        !contains_status_junk(&lower),
    ];
    let hits = checks.iter().filter(|hit| **hit).count();
    let mut score: f64 = 0.10 + 0.45 * (hits as f64 / checks.len() as f64);
    if words >= 6_000 {
        score += 0.35;
    } else if words >= 3_000 {
        score += 0.15;
    }

    let mut dimensions = HashMap::new();
    dimensions.insert("word_count".to_string(), (words as f64 / 6_000.0).min(1.0));
    dimensions.insert(
        "paper_structure".to_string(),
        hits as f64 / checks.len() as f64,
    );
    QualityScore {
        overall: score.min(0.95),
        dimensions,
    }
}

pub fn score_validator_output(output: &str) -> QualityScore {
    let lower = output.to_ascii_lowercase();
    let verdict_lines = lower
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(12)
        .collect::<Vec<_>>();
    let explicit_fail = verdict_lines.iter().any(|line| {
        line.starts_with("fail")
            || line.starts_with("verdict: fail")
            || line.starts_with("qa outcome: fail")
            || line.contains("blocking issue")
    });
    let explicit_pass = verdict_lines.iter().any(|line| {
        line.starts_with("pass")
            || line.starts_with("verdict: pass")
            || line.starts_with("qa outcome: pass")
            || line.contains("approved")
    });
    let score = if explicit_fail {
        0.0
    } else if explicit_pass {
        0.85
    } else {
        score_report(
            output,
            400,
            &["citations", "references", "appendix", "chapter"],
        )
        .overall
    };
    let mut dimensions = HashMap::new();
    dimensions.insert(
        "verdict".to_string(),
        if explicit_fail { 0.0 } else { score.min(1.0) },
    );
    QualityScore {
        overall: score.min(0.95),
        dimensions,
    }
}

pub fn clean_chapter_body(output: &str) -> String {
    let mut body = Vec::new();
    let mut skip_refs = false;
    let mut skip_noise = false;
    for line in output.lines() {
        let trimmed = line.trim();
        let lower = trimmed.to_ascii_lowercase();
        if is_noise_heading(&lower) {
            skip_noise = true;
            continue;
        }
        if markdown_heading_level(trimmed).is_some() {
            skip_noise = false;
        }
        if skip_noise || lower.starts_with("# artifact:") {
            continue;
        }
        if matches!(
            lower.as_str(),
            "## references" | "# references" | "references"
        ) {
            skip_refs = true;
            continue;
        }
        if skip_refs {
            continue;
        }
        if trimmed.starts_with("# ") {
            continue;
        }
        body.push(line);
    }
    body.join("\n").trim().to_string()
}

pub fn extract_abstract_section(output: &str) -> Option<String> {
    let mut collecting = false;
    let mut lines = Vec::new();
    for line in output.lines() {
        let trimmed = line.trim();
        let lower = trimmed.to_ascii_lowercase();
        if markdown_heading_level(trimmed).is_some() {
            if collecting {
                break;
            }
            if lower == "## abstract" || lower == "# abstract" {
                collecting = true;
            }
            continue;
        }
        if collecting {
            lines.push(line);
        }
    }
    let abstract_text = lines.join("\n").trim().to_string();
    (!abstract_text.is_empty()).then_some(abstract_text)
}

pub fn best_reference_section(outputs: &[(&str, &str)]) -> Option<String> {
    outputs
        .iter()
        .filter(|(key, _)| *key == "citation-reconciler")
        .filter_map(|(_, output)| reference_section_candidate(output, true))
        .max_by_key(|refs| reference_entry_count(refs))
        .or_else(|| {
            outputs
                .iter()
                .filter(|(key, _)| key.contains("citation") || key.contains("reference"))
                .filter_map(|(_, output)| reference_section_candidate(output, false))
                .max_by_key(|refs| reference_entry_count(refs))
        })
}

fn reference_section_candidate(output: &str, master_only: bool) -> Option<String> {
    let refs = extract_reference_section_with_mode(output, master_only);
    (reference_entry_count(&refs) > 0).then_some(refs)
}

pub fn extract_reference_section(output: &str) -> String {
    extract_reference_section_with_mode(output, false)
}

fn extract_reference_section_with_mode(output: &str, master_only: bool) -> String {
    collect_reference_section(output, true).unwrap_or_else(|| {
        if master_only {
            String::new()
        } else {
            collect_reference_section(output, false).unwrap_or_default()
        }
    })
}

fn collect_reference_section(output: &str, master: bool) -> Option<String> {
    let mut collecting = false;
    let mut refs = Vec::new();
    for line in output.lines() {
        let trimmed = line.trim();
        let starts_target = reference_heading_matches(trimmed, master);
        if starts_target {
            collecting = true;
            continue;
        }
        if collecting && markdown_heading_title(trimmed).is_some() {
            break;
        }
        if collecting {
            if !trimmed.is_empty() && !trimmed.starts_with('|') {
                refs.push(trimmed.trim_start_matches("- ").to_string());
            }
        }
    }
    let refs = refs.join("\n\n");
    (!refs.trim().is_empty()).then_some(refs)
}

fn reference_heading_matches(line: &str, master: bool) -> bool {
    let Some(title) = markdown_heading_title(line) else {
        return false;
    };
    let normal = title.to_ascii_lowercase();
    if master {
        normal.contains("master reference list")
    } else {
        matches!(
            normal.as_str(),
            "references" | "reference list" | "bibliography" | "works cited"
        )
    }
}

fn markdown_heading_title(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    let hashes = trimmed.chars().take_while(|&c| c == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let title = trimmed[hashes..].trim();
    (!title.is_empty()).then_some(title)
}

fn reference_entry_count(refs: &str) -> usize {
    refs.lines()
        .filter(|line| {
            let line = line.trim();
            !line.starts_with("**")
                && (line.contains("(19") || line.contains("(20") || line.contains("(n.d.)"))
        })
        .count()
}

pub fn fallback_hld_reference() -> String {
    "GSS / GKB Architecture Team. (2020). *HLD - Match Scoring* \
     [Internal high-level design document]. Global Screening / GKB."
        .to_string()
}

pub fn bundle_reference_section(bundle_dir: &Path) -> Option<String> {
    [
        "outputs/041-citation-reconciler.txt",
        "outputs/markdown/041-citation-reconciler.md",
    ]
    .iter()
    .filter_map(|rel| fs::read_to_string(bundle_dir.join(rel)).ok())
    .filter_map(|output| reference_section_candidate(&output, true))
    .max_by_key(|refs| reference_entry_count(refs))
}

pub fn count_words(output: &str) -> usize {
    output
        .split_whitespace()
        .filter(|word| word.chars().any(char::is_alphanumeric))
        .count()
}

pub fn truncate_chars(input: &str, max: usize) -> String {
    if input.chars().count() <= max {
        return input.to_string();
    }
    let byte = input
        .char_indices()
        .nth(max)
        .map(|(idx, _)| idx)
        .unwrap_or(input.len());
    format!("{}\n[truncated]", &input[..byte])
}

pub fn slug(title: &str) -> String {
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
    out.trim_matches('-').to_string()
}

fn contains_status_junk(lower: &str) -> bool {
    lower.contains("artifacts created:") || lower.contains("memory stored:")
}

fn is_noise_heading(lower: &str) -> bool {
    matches!(
        lower.trim_matches('#').trim(),
        "abstract quality check"
            | "keyword justification"
            | "journal compliance"
            | "quality gate"
            | "executive summary"
            | "artifacts created:"
            | "memory stored:"
    ) || lower.starts_with("completed ") && lower.contains(" output")
}

fn markdown_heading_level(line: &str) -> Option<usize> {
    let hashes = line.chars().take_while(|&c| c == '#').count();
    (hashes > 0 && hashes <= 6).then_some(hashes)
}
