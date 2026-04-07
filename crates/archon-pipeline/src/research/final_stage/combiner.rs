//! Paper combiner — merges chapters into a final paper with Table of Contents.

/// Represents the content of a single chapter ready for combination.
#[derive(Debug, Clone)]
pub struct ChapterContent {
    /// Chapter number (1-based).
    pub number: u32,
    /// Human-readable chapter title.
    pub title: String,
    /// The full body text for this chapter.
    pub content: String,
}

/// Combine chapters into a final paper with a title page and Table of Contents.
///
/// Chapters are emitted in the order they appear in the input slice. Each chapter
/// is separated by a horizontal rule.
pub fn combine_chapters(chapters: &[ChapterContent]) -> String {
    let mut paper = String::new();

    // Title page
    paper.push_str("# Final Research Paper\n\n");

    // Table of Contents
    paper.push_str("## Table of Contents\n\n");
    for ch in chapters {
        paper.push_str(&format!(
            "- [Chapter {}: {}](#chapter-{})\n",
            ch.number, ch.title, ch.number
        ));
    }
    paper.push_str("\n---\n\n");

    // Chapters in order
    for ch in chapters {
        paper.push_str(&format!("## Chapter {}: {}\n\n", ch.number, ch.title));
        paper.push_str(&ch.content);
        paper.push_str("\n\n---\n\n");
    }

    paper
}
