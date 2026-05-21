use std::collections::HashMap;

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
    for line in output.lines() {
        let trimmed = line.trim();
        let lower = trimmed.to_ascii_lowercase();
        if lower.starts_with("# artifact:") || lower.starts_with("artifacts created:") {
            continue;
        }
        if lower == "## references" || lower == "# references" {
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

pub fn extract_reference_section(output: &str) -> String {
    let mut collecting = false;
    let mut refs = Vec::new();
    for line in output.lines() {
        let lower = line.to_ascii_lowercase();
        if lower.contains("master reference list") || lower.trim() == "## references" {
            collecting = true;
            continue;
        }
        if collecting && line.starts_with("## ") {
            break;
        }
        if collecting {
            let trimmed = line.trim();
            if !trimmed.is_empty() && !trimmed.starts_with('|') {
                refs.push(trimmed.trim_start_matches("- ").to_string());
            }
        }
    }
    refs.join("\n\n")
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
