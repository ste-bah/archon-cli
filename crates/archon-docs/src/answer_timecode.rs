use crate::answer::Citation;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimecodeMs {
    pub start_ms: i64,
}

pub fn format_ms_as_timecode(ms: i64) -> String {
    let clamped = ms.max(0);
    let total_secs = clamped / 1000;
    if clamped >= 3_600_000 {
        let hours = total_secs / 3600;
        let minutes = (total_secs % 3600) / 60;
        let seconds = total_secs % 60;
        format!("{hours:02}:{minutes:02}:{seconds:02}")
    } else {
        let minutes = total_secs / 60;
        let seconds = total_secs % 60;
        format!("{minutes:02}:{seconds:02}")
    }
}

pub fn format_citation(
    index: usize,
    citation: &Citation,
    score: f64,
    timeref: Option<TimecodeMs>,
) -> String {
    if let Some(timeref) = timeref {
        format_citation_video(index, citation, score, timeref.start_ms)
    } else {
        format_citation_page(index, citation, score)
    }
}

pub fn format_citation_page(index: usize, citation: &Citation, score: f64) -> String {
    format!(
        "\n[{}] (pages {}-{}, score {:.2}):\n{}\n",
        index, citation.page_start, citation.page_end, score, citation.snippet
    )
}

pub fn format_citation_video(
    index: usize,
    citation: &Citation,
    score: f64,
    start_ms: i64,
) -> String {
    format!(
        "\n[{}] (video@{}, score {:.2}):\n{}\n",
        index,
        format_ms_as_timecode(start_ms),
        score,
        citation.snippet
    )
}
