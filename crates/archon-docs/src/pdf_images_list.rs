//! Helpers for parsing `pdfimages -list` output and filtering extracted images.
//!
//! Split out of `pdf.rs` to keep both files under the 500-line gate.

use std::fs;
use std::path::{Path, PathBuf};

use archon_policy::PdfPolicy;

use super::PdfImagesListEntry;
use crate::errors::DocsError;

pub fn parse_pdfimages_list(output: &str) -> Vec<PdfImagesListEntry> {
    let mut entries = Vec::new();
    for line in output.lines() {
        let cols = line.split_whitespace().collect::<Vec<_>>();
        if cols.len() < 5 {
            continue;
        }
        let Ok(page) = cols[0].parse::<u32>() else {
            continue;
        };
        let Ok(width) = cols[3].parse::<u32>() else {
            continue;
        };
        let Ok(height) = cols[4].parse::<u32>() else {
            continue;
        };
        let object_key = if cols.len() > 11 {
            Some(format!("{}:{}", cols[10], cols[11]))
        } else {
            None
        };
        // `pdfimages -list` columns: … object(10) ID(11) x-ppi(12) y-ppi(13) size(14) ratio(15).
        // Keep the ppi (dropped historically) for the coverage classifier; tolerate short rows.
        let x_ppi = cols.get(12).and_then(|c| c.parse::<u32>().ok());
        let y_ppi = cols.get(13).and_then(|c| c.parse::<u32>().ok());
        let bytes = cols.get(14).and_then(|c| parse_pdfimages_size(c));
        entries.push(PdfImagesListEntry {
            source_page: page,
            source_pages: vec![page],
            width,
            height,
            object_key: object_key.clone(),
            xobject_name: object_key,
            x_ppi,
            y_ppi,
            bytes,
        });
    }
    entries
}

/// Parse a `pdfimages -list` `size` cell (`1620B`, `479K`, `3.9M`, or a bare number) into bytes.
pub(crate) fn parse_pdfimages_size(cell: &str) -> Option<u64> {
    let cell = cell.trim();
    let last = cell.chars().last()?;
    let (num, mult) = match last {
        'B' | 'b' => (&cell[..cell.len() - 1], 1.0),
        'K' | 'k' => (&cell[..cell.len() - 1], 1024.0),
        'M' | 'm' => (&cell[..cell.len() - 1], 1024.0 * 1024.0),
        'G' | 'g' => (&cell[..cell.len() - 1], 1024.0 * 1024.0 * 1024.0),
        d if d.is_ascii_digit() => (cell, 1.0),
        _ => return None,
    };
    let value: f64 = num.parse().ok()?;
    if value < 0.0 {
        return None;
    }
    Some((value * mult) as u64)
}

pub(crate) fn dedupe_entries_by_object(
    entries: Vec<PdfImagesListEntry>,
) -> Vec<PdfImagesListEntry> {
    let mut deduped = Vec::<PdfImagesListEntry>::new();
    for entry in entries {
        let key = entry.object_key.clone();
        if let Some(key) = key
            && let Some(existing) = deduped
                .iter_mut()
                .find(|candidate| candidate.object_key.as_ref() == Some(&key))
        {
            for page in entry.source_pages {
                if !existing.source_pages.contains(&page) {
                    existing.source_pages.push(page);
                }
            }
            continue;
        }
        deduped.push(entry);
    }
    deduped
}

pub fn image_survives_filter(width: u32, height: u32, bytes: u64, policy: &PdfPolicy) -> bool {
    width.max(height) >= policy.min_image_dimension && bytes >= policy.min_image_bytes
}

pub(crate) fn list_supported_image_files(dir: &Path) -> Result<Vec<PathBuf>, DocsError> {
    let mut files = fs::read_dir(dir)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| mime_from_path(path).is_some())
        .collect::<Vec<_>>();
    files.sort();
    Ok(files)
}

pub(crate) fn mime_from_path(path: &Path) -> Option<&'static str> {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some(ext) if ext.eq_ignore_ascii_case("png") => Some("image/png"),
        Some(ext) if ext.eq_ignore_ascii_case("jpg") || ext.eq_ignore_ascii_case("jpeg") => {
            Some("image/jpeg")
        }
        _ => None,
    }
}

pub(crate) fn image_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() >= 24 && bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        let width = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
        let height = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
        return Some((width, height));
    }
    None
}
