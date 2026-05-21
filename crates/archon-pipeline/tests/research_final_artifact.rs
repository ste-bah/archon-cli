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
    assert!(!artifacts.chapter_paths.is_empty());
    assert!(artifacts.chapter_paths[0].exists());
}

#[test]
fn final_artifact_accepts_numbered_chapter_introduction() {
    let paper = r#"# Title

## Abstract

This paper has an abstract.

# Chapter 1: Introduction

The introduction is emitted as a numbered chapter heading.

# References

Smith, J. (2024). Screening systems. Journal of Compliance, 12(2), 10-22.
"#;
    let paper = ResearchPaper::parse(paper).unwrap();
    assert!(paper.body_markdown.contains("Chapter 1: Introduction"));
}

#[test]
fn final_artifact_groups_numbered_chapter_exports() {
    let tmp = tempfile::tempdir().unwrap();
    let stale_dir = tmp.path().join("exports").join("chapters");
    fs::create_dir_all(&stale_dir).unwrap();
    fs::write(stale_dir.join("stale.md"), "old").unwrap();
    let paper = r#"# Title

## Abstract

This paper has an abstract.

# 1. Introduction

## 1.1 Background

Introductory background.

# 2. Architecture

## 2.1 Components

Architectural components.

# References

Smith, J. (2024). Screening systems. Journal of Compliance, 12(2), 10-22.
"#;
    let artifacts = write_final_research_artifacts(tmp.path(), paper).unwrap();
    assert_eq!(artifacts.chapter_paths.len(), 2);
    assert!(!stale_dir.join("stale.md").exists());
    let chapter = fs::read_to_string(&artifacts.chapter_paths[0]).unwrap();
    assert!(chapter.contains("### 1.1 Background"));
}

#[test]
fn final_artifact_uses_bundle_master_references_when_sparse() {
    let tmp = tempfile::tempdir().unwrap();
    let outputs = tmp.path().join("outputs");
    fs::create_dir_all(&outputs).unwrap();
    fs::write(
        outputs.join("041-citation-reconciler.txt"),
        r#"## Canonical Citation Rules

- Reconcile all citations before export.

## Master Reference List

Smith, J. (2024). Screening systems.

Adams, R. (2023). Architecture controls.

## Removed or Downgraded Citations

| Citation | Action |
|---|---|
| Weak source | Removed |
"#,
    )
    .unwrap();
    let paper = r#"# Title

## Abstract

This paper has an abstract.

## Introduction

Body text.

## References

GSS / GKB Architecture Team. (2020). *HLD - Match Scoring* [Internal high-level design document]. Global Screening / GKB.
"#;
    write_final_research_artifacts(tmp.path(), paper).unwrap();
    let markdown = fs::read_to_string(tmp.path().join("exports/final-paper.md")).unwrap();
    assert!(markdown.contains("Adams, R. (2023). Architecture controls."));
    assert!(markdown.contains("Smith, J. (2024). Screening systems."));
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
