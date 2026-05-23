use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::errors::VideoError;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TranscriptSegment {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    pub confidence: Option<f32>,
    pub speaker: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedTranscript {
    pub segments: Vec<TranscriptSegment>,
    pub warnings: Vec<String>,
    pub format: TranscriptFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranscriptFormat {
    Srt,
    Vtt,
    Ttml,
    Json,
    PlainText,
}

pub fn parse_transcript(
    content: &[u8],
    hint: Option<TranscriptFormat>,
) -> Result<ParsedTranscript, VideoError> {
    let format = hint.unwrap_or_else(|| sniff_format(content));
    match format {
        TranscriptFormat::Srt => parse_cue_text(content, TranscriptFormat::Srt),
        TranscriptFormat::Vtt => parse_cue_text(content, TranscriptFormat::Vtt),
        TranscriptFormat::Ttml => parse_ttml(content),
        TranscriptFormat::Json => parse_json(content),
        TranscriptFormat::PlainText => parse_plain(content),
    }
}

pub fn sniff_format(content: &[u8]) -> TranscriptFormat {
    let text = String::from_utf8_lossy(content);
    let trimmed = text.trim_start();
    if trimmed.starts_with("WEBVTT") {
        TranscriptFormat::Vtt
    } else if trimmed.starts_with('[') || trimmed.starts_with('{') {
        TranscriptFormat::Json
    } else if trimmed.contains("<tt") || trimmed.contains("<p ") {
        TranscriptFormat::Ttml
    } else if trimmed.contains("-->") && trimmed.contains(',') {
        TranscriptFormat::Srt
    } else if trimmed.contains("-->") {
        TranscriptFormat::Vtt
    } else {
        TranscriptFormat::PlainText
    }
}

fn parse_cue_text(
    content: &[u8],
    format: TranscriptFormat,
) -> Result<ParsedTranscript, VideoError> {
    let text = String::from_utf8_lossy(content);
    let mut segments = Vec::new();
    let mut warnings = Vec::new();
    for block in text.split("\n\n") {
        let mut lines = block.lines().map(str::trim).filter(|line| !line.is_empty());
        let Some(first) = lines.next() else {
            continue;
        };
        if first == "WEBVTT" || first.starts_with("NOTE") {
            continue;
        }
        let timestamp_line = if first.contains("-->") {
            first
        } else {
            lines.next().unwrap_or("")
        };
        if !timestamp_line.contains("-->") {
            warnings.push(format!("skipped cue without timestamp: {first}"));
            continue;
        }
        let text_lines: Vec<&str> = lines.collect();
        let parts: Vec<&str> = timestamp_line.split("-->").collect();
        let start = parse_timestamp_ms(parts[0].trim());
        let end = parse_timestamp_ms(parts.get(1).copied().unwrap_or("").trim());
        match (start, end) {
            (Some(start_ms), end_ms) => segments.push(TranscriptSegment {
                start_ms,
                end_ms: end_ms.unwrap_or(0),
                text: strip_tags(&text_lines.join("\n")),
                confidence: None,
                speaker: None,
            }),
            _ => warnings.push(format!("skipped malformed cue timestamp: {timestamp_line}")),
        }
    }
    finish_segments(segments, warnings, format)
}

fn parse_ttml(content: &[u8]) -> Result<ParsedTranscript, VideoError> {
    let text = String::from_utf8_lossy(content);
    let mut segments = Vec::new();
    let mut warnings = Vec::new();
    for part in text.split("<p").skip(1) {
        let Some((attrs, rest)) = part.split_once('>') else {
            warnings.push("skipped malformed TTML paragraph".into());
            continue;
        };
        let body = rest.split("</p>").next().unwrap_or("");
        let start = attr_value(attrs, "begin").and_then(|value| parse_timestamp_ms(&value));
        let end = attr_value(attrs, "end").and_then(|value| parse_timestamp_ms(&value));
        match start {
            Some(start_ms) => segments.push(TranscriptSegment {
                start_ms,
                end_ms: end.unwrap_or(0),
                text: strip_tags(body),
                confidence: None,
                speaker: attr_value(attrs, "speaker"),
            }),
            None => warnings.push("skipped TTML paragraph with malformed begin time".into()),
        }
    }
    finish_segments(segments, warnings, TranscriptFormat::Ttml)
}

fn parse_json(content: &[u8]) -> Result<ParsedTranscript, VideoError> {
    let root: Value =
        serde_json::from_slice(content).map_err(|e| VideoError::AcquisitionFailed {
            message: format!("parse transcript JSON: {e}"),
        })?;
    let array = root
        .as_array()
        .or_else(|| root.get("segments").and_then(Value::as_array))
        .ok_or_else(|| VideoError::AcquisitionFailed {
            message: "transcript JSON must be an array or {segments:[...]}".into(),
        })?;
    let mut segments = Vec::new();
    let mut warnings = Vec::new();
    for (index, item) in array.iter().enumerate() {
        let start = json_time(item, &["start_ms", "start", "start_time"]);
        let end = json_time(item, &["end_ms", "end", "end_time"]);
        let text = item
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        if text.is_empty() {
            warnings.push(format!("skipped JSON segment {index} with empty text"));
            continue;
        }
        match start {
            Some(start_ms) => segments.push(TranscriptSegment {
                start_ms,
                end_ms: end.unwrap_or(0),
                text: text.to_string(),
                confidence: item
                    .get("confidence")
                    .and_then(Value::as_f64)
                    .map(|value| value as f32),
                speaker: item
                    .get("speaker")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            }),
            None => warnings.push(format!("skipped JSON segment {index} with malformed start")),
        }
    }
    finish_segments(segments, warnings, TranscriptFormat::Json)
}

fn parse_plain(content: &[u8]) -> Result<ParsedTranscript, VideoError> {
    let text = String::from_utf8_lossy(content);
    let mut segments = Vec::new();
    let mut untimed = Vec::new();
    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        if let Some((time, body)) = split_plain_timestamp(line) {
            if let Some(start_ms) = parse_timestamp_ms(time) {
                segments.push(TranscriptSegment {
                    start_ms,
                    end_ms: 0,
                    text: body.trim().to_string(),
                    confidence: None,
                    speaker: None,
                });
            } else {
                untimed.push(line.to_string());
            }
        } else {
            untimed.push(line.to_string());
        }
    }
    if segments.is_empty() && !untimed.is_empty() {
        segments.push(TranscriptSegment {
            start_ms: 0,
            end_ms: 5000,
            text: untimed.join("\n"),
            confidence: None,
            speaker: None,
        });
    }
    finish_segments(segments, Vec::new(), TranscriptFormat::PlainText)
}

fn finish_segments(
    mut segments: Vec<TranscriptSegment>,
    mut warnings: Vec<String>,
    format: TranscriptFormat,
) -> Result<ParsedTranscript, VideoError> {
    infer_missing_end_ms(&mut segments, &mut warnings);
    warnings.extend(overlap_warnings(&segments));
    Ok(ParsedTranscript {
        segments,
        warnings,
        format,
    })
}

fn infer_missing_end_ms(segments: &mut [TranscriptSegment], warnings: &mut Vec<String>) {
    for index in 0..segments.len() {
        if segments[index].end_ms != 0 {
            continue;
        }
        let inferred = segments
            .get(index + 1)
            .map(|next| next.start_ms)
            .unwrap_or(segments[index].start_ms + 5000);
        segments[index].end_ms = inferred;
        warnings.push(format!("inferred end_ms for segment {index}"));
    }
}

fn overlap_warnings(segments: &[TranscriptSegment]) -> Vec<String> {
    segments
        .windows(2)
        .enumerate()
        .filter(|(_, pair)| pair[0].end_ms > pair[1].start_ms)
        .map(|(index, _)| {
            format!(
                "overlapping timestamps between segments {index} and {}",
                index + 1
            )
        })
        .collect()
}

pub fn export_to_vtt(segments: &[TranscriptSegment]) -> String {
    let mut out = String::from("WEBVTT\n\n");
    for segment in segments {
        out.push_str(&format!(
            "{} --> {}\n{}\n\n",
            format_timestamp(segment.start_ms, '.'),
            format_timestamp(segment.end_ms, '.'),
            segment.text
        ));
    }
    out
}

pub fn export_to_srt(segments: &[TranscriptSegment]) -> String {
    let mut out = String::new();
    for (index, segment) in segments.iter().enumerate() {
        out.push_str(&format!(
            "{}\n{} --> {}\n{}\n\n",
            index + 1,
            format_timestamp(segment.start_ms, ','),
            format_timestamp(segment.end_ms, ','),
            segment.text
        ));
    }
    out
}

pub fn export_to_txt(segments: &[TranscriptSegment]) -> String {
    segments
        .iter()
        .map(|segment| segment.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn parse_timestamp_ms(raw: &str) -> Option<u64> {
    let cleaned = raw.trim().trim_end_matches('s').replace(',', ".");
    let parts: Vec<&str> = cleaned.split(':').collect();
    let (hours, minutes, seconds) = match parts.as_slice() {
        [h, m, s] => (h.parse::<u64>().ok()?, m.parse::<u64>().ok()?, *s),
        [m, s] => (0, m.parse::<u64>().ok()?, *s),
        [s] => (0, 0, *s),
        _ => return None,
    };
    let (secs, millis) = seconds
        .split_once('.')
        .map(|(s, ms)| (s, ms))
        .unwrap_or((seconds, "0"));
    let seconds = secs.parse::<u64>().ok()?;
    let millis = format!("{millis:0<3}")
        .chars()
        .take(3)
        .collect::<String>()
        .parse::<u64>()
        .ok()?;
    Some(hours * 3_600_000 + minutes * 60_000 + seconds * 1000 + millis)
}

fn format_timestamp(ms: u64, separator: char) -> String {
    let hours = ms / 3_600_000;
    let minutes = (ms / 60_000) % 60;
    let seconds = (ms / 1000) % 60;
    let millis = ms % 1000;
    format!("{hours:02}:{minutes:02}:{seconds:02}{separator}{millis:03}")
}

fn split_plain_timestamp(line: &str) -> Option<(&str, &str)> {
    let trimmed = line.trim_start();
    let close = match trimmed.chars().next()? {
        '[' => ']',
        '(' => ')',
        _ => return None,
    };
    let end = trimmed.find(close)?;
    Some((&trimmed[1..end], &trimmed[end + 1..]))
}

fn attr_value(attrs: &str, name: &str) -> Option<String> {
    let needle = format!("{name}=\"");
    let start = attrs.find(&needle)? + needle.len();
    let end = attrs[start..].find('"')?;
    Some(attrs[start..start + end].to_string())
}

fn json_time(item: &Value, keys: &[&str]) -> Option<u64> {
    for key in keys {
        if let Some(value) = item.get(*key) {
            if let Some(ms) = value.as_u64() {
                return Some(ms);
            }
            if let Some(seconds) = value.as_f64() {
                return Some((seconds * 1000.0).round() as u64);
            }
            if let Some(text) = value.as_str().and_then(parse_timestamp_ms) {
                return Some(text);
            }
        }
    }
    None
}

fn strip_tags(text: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for ch in text.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out.trim().to_string()
}
