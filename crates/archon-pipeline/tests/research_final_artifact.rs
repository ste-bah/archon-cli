use std::fs;

use archon_pipeline::research::final_artifact::{
    ResearchPaper, artifact_paths, write_final_research_artifacts,
};

fn sample_paper() -> &'static str {
    r#"# GKB Match Scoring and Research Disposition Algorithms

## Abstract

This paper examines match scoring architecture, disposition algorithm design,
and governance concerns for proprietary financial-crime screening systems.

## 1. Introduction

Screening platforms require explainable matching, reliable operations, and
governed disposition logic (Smith, 2024). The architecture must support
configurable risk controls while preserving auditability.

## References

Smith, J. (2024). Screening systems and financial crime controls. Journal of Applied Compliance, 12(2), 10-22.

## 2. Architecture

Match scoring services should separate candidate generation from scoring and
case disposition. This separation improves maintainability and operational
control (Adams, 2023).

## References

Adams, R. (2023). Maintainable screening architectures. Compliance Engineering Review, 8(1), 44-61.

## Appendix A: Source Material

The ingested HLD PDF is treated as the primary architecture source.
"#
}

#[test]
fn final_artifact_normalises_references_and_appendices() {
    let paper = ResearchPaper::parse(sample_paper()).unwrap();
    let markdown = paper.to_markdown();

    assert_eq!(markdown.matches("## References").count(), 1);
    assert!(markdown.contains("## Appendix A: Source Material"));
    let references = markdown.find("## References").unwrap();
    let appendix = markdown.find("## Appendix A").unwrap();
    assert!(references < appendix);
    assert!(markdown.contains("Adams, R. (2023)."));
    assert!(markdown.contains("Smith, J. (2024)."));
}

#[test]
fn final_artifact_writes_markdown_and_pdf() {
    let tmp = tempfile::tempdir().unwrap();
    let artifacts = write_final_research_artifacts(tmp.path(), sample_paper()).unwrap();
    let (markdown_path, pdf_path) = artifact_paths(tmp.path());

    assert_eq!(artifacts.markdown_path, markdown_path);
    assert_eq!(artifacts.pdf_path, pdf_path);
    assert!(
        fs::read_to_string(markdown_path)
            .unwrap()
            .contains("## References")
    );
    let pdf = fs::read(pdf_path).unwrap();
    assert!(pdf.starts_with(b"%PDF-1.4"));
    assert!(pdf.len() > 1000);
}

#[test]
fn final_artifact_splits_line_separated_references() {
    let paper = r#"# Title

## Abstract

This paper has an abstract.

## Introduction

The body cites two sources (Smith, 2024; Adams, 2023).

## References

Smith, J. (2024). Screening systems. Journal of Compliance, 12(2), 10-22.
Adams, R. (2023). Maintainable screening architectures. Compliance Engineering Review, 8(1), 44-61.
"#;
    let paper = ResearchPaper::parse(paper).unwrap();
    assert_eq!(paper.references.len(), 2);
    assert!(paper.to_markdown().contains("Adams, R. (2023)."));
}

#[test]
fn final_artifact_requires_references() {
    let paper = r#"# Title

## Abstract

Abstract text.

## Introduction

Body text.
"#;
    let err = ResearchPaper::parse(paper).unwrap_err().to_string();
    assert!(err.contains("References"));
}

#[test]
fn final_artifact_rejects_chatty_preamble() {
    let paper = format!("Here is the paper:\n\n{}", sample_paper());
    let err = ResearchPaper::parse(&paper).unwrap_err().to_string();
    assert!(err.contains("title"));
}
