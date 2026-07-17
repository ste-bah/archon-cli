use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};

use regex::Regex;

const SOURCE_EXTENSIONS: &[&str] = &["rs", "ts", "tsx", "js", "jsx", "py", "go"];

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CandidateReferences {
    pub(crate) candidate: String,
    pub(crate) references: Vec<String>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct OrphanReferenceScan {
    pub(crate) references: Vec<CandidateReferences>,
    pub(crate) errors: Vec<String>,
}

struct CandidatePattern {
    candidate: String,
    path: PathBuf,
    pattern: Regex,
}

pub(crate) fn orphan_result(
    new_files: &[PathBuf],
    project_root: &Path,
    timestamp: String,
) -> super::gates::GateResultRecord {
    if new_files.is_empty() {
        return empty_orphan_result(timestamp);
    }

    let scan = scan_orphan_references(new_files, project_root, || {}, |_| {});
    if !scan.errors.is_empty() {
        return scan_error_result(scan.errors, timestamp);
    }

    scan_success_result(scan.references, timestamp)
}

fn empty_orphan_result(timestamp: String) -> super::gates::GateResultRecord {
    super::gates::GateResultRecord {
        gate_name: "orphan-detection".into(),
        gate_passed: true,
        evidence: "No new files to check.".into(),
        failures: Vec::new(),
        timestamp,
    }
}

fn scan_error_result(errors: Vec<String>, timestamp: String) -> super::gates::GateResultRecord {
    let failures = errors
        .iter()
        .map(|error| super::gates::GateFailure {
            description: "Orphan detection scan failed".into(),
            file: None,
            details: error.clone(),
        })
        .collect();
    super::gates::GateResultRecord {
        gate_name: "orphan-detection".into(),
        gate_passed: false,
        evidence: errors.join("\n"),
        failures,
        timestamp,
    }
}

fn scan_success_result(
    references: Vec<CandidateReferences>,
    timestamp: String,
) -> super::gates::GateResultRecord {
    let mut failures = Vec::new();
    let evidence = references
        .into_iter()
        .map(|candidate| evidence_for_candidate(candidate, &mut failures))
        .collect::<Vec<_>>();
    super::gates::GateResultRecord {
        gate_name: "orphan-detection".into(),
        gate_passed: failures.is_empty(),
        evidence: evidence.join("\n"),
        failures,
        timestamp,
    }
}

fn evidence_for_candidate(
    candidate: CandidateReferences,
    failures: &mut Vec<super::gates::GateFailure>,
) -> String {
    if candidate.references.is_empty() {
        failures.push(super::gates::GateFailure {
            description: format!("Orphaned file: {}", candidate.candidate),
            file: Some(candidate.candidate.clone()),
            details: format!(
                "No mod/use/import/require references to '{}' found in project",
                candidate_stem(&candidate.candidate)
            ),
        });
        return format!("ORPHAN: {} — zero references", candidate.candidate);
    }

    format!(
        "OK: {} — referenced by: {}",
        candidate.candidate,
        candidate.references.join(", ")
    )
}

fn candidate_stem(candidate: &str) -> &str {
    Path::new(candidate)
        .file_stem()
        .and_then(OsStr::to_str)
        .unwrap_or(candidate)
}

pub(crate) fn scan_orphan_references<OnWalk, OnRead>(
    new_files: &[PathBuf],
    project_root: &Path,
    mut on_walk: OnWalk,
    mut on_read: OnRead,
) -> OrphanReferenceScan
where
    OnWalk: FnMut(),
    OnRead: FnMut(&Path),
{
    let project_root = normalize_path(project_root);
    let (candidates, mut errors) = candidate_patterns(new_files, &project_root);
    if has_ambiguous_stem_error(&errors) {
        return finish_scan(Vec::new(), errors);
    }
    let mut source_paths = Vec::new();
    on_walk();
    collect_source_paths(&project_root, &mut source_paths, &mut errors);
    source_paths.sort();
    let references = scan_sources(
        &candidates,
        source_paths,
        &project_root,
        &mut errors,
        &mut on_read,
    );
    finish_scan(references, errors)
}

fn scan_sources<OnRead>(
    candidates: &[CandidatePattern],
    source_paths: Vec<PathBuf>,
    project_root: &Path,
    errors: &mut Vec<String>,
    on_read: &mut OnRead,
) -> Vec<CandidateReferences>
where
    OnRead: FnMut(&Path),
{
    let mut references = reference_rows(candidates);
    for source_path in source_paths {
        scan_source(
            candidates,
            &mut references,
            project_root,
            source_path,
            errors,
            on_read,
        );
    }
    references
}

fn reference_rows(candidates: &[CandidatePattern]) -> Vec<CandidateReferences> {
    candidates
        .iter()
        .map(|candidate| CandidateReferences {
            candidate: candidate.candidate.clone(),
            references: Vec::new(),
        })
        .collect()
}

fn scan_source<OnRead>(
    candidates: &[CandidatePattern],
    references: &mut [CandidateReferences],
    project_root: &Path,
    source_path: PathBuf,
    errors: &mut Vec<String>,
    on_read: &mut OnRead,
) where
    OnRead: FnMut(&Path),
{
    let source_display = display_path(project_root, &source_path);
    if source_path.to_str().is_none() {
        errors.push(format!("Non-UTF8 source path: {source_display}"));
        return;
    }

    on_read(&source_path);
    let content = match read_source(&source_path, &source_display, errors) {
        Some(content) => content,
        None => return,
    };
    record_matches(
        candidates,
        references,
        &source_path,
        source_display,
        &content,
    );
}

fn read_source(path: &Path, display: &str, errors: &mut Vec<String>) -> Option<String> {
    match std::fs::read_to_string(path) {
        Ok(content) => Some(content),
        Err(error) if error.kind() == std::io::ErrorKind::InvalidData => {
            errors.push(format!("Non-UTF8 source content: {display}"));
            None
        }
        Err(error) => {
            errors.push(format!("Unable to read source file {display}: {error}"));
            None
        }
    }
}

fn record_matches(
    candidates: &[CandidatePattern],
    references: &mut [CandidateReferences],
    source_path: &Path,
    source_display: String,
    content: &str,
) {
    for (candidate, reference) in candidates.iter().zip(references.iter_mut()) {
        if source_path != candidate.path && candidate.pattern.is_match(content) {
            reference.references.push(source_display.clone());
        }
    }
}

fn finish_scan(
    mut references: Vec<CandidateReferences>,
    mut errors: Vec<String>,
) -> OrphanReferenceScan {
    errors.sort();
    if errors.is_empty() {
        for candidate in &mut references {
            candidate.references.sort();
        }
    } else {
        references.clear();
    }
    OrphanReferenceScan { references, errors }
}

fn has_ambiguous_stem_error(errors: &[String]) -> bool {
    errors
        .iter()
        .any(|error| error.starts_with("Ambiguous candidate stem '"))
}

fn candidate_patterns(
    new_files: &[PathBuf],
    project_root: &Path,
) -> (Vec<CandidatePattern>, Vec<String>) {
    let mut candidates = Vec::new();
    let mut errors = Vec::new();
    for path in new_files {
        match candidate_pattern(path, project_root) {
            Ok(candidate) => candidates.push(candidate),
            Err(error) => errors.push(error),
        }
    }
    candidates.sort_by(|left, right| left.candidate.cmp(&right.candidate));
    candidates.dedup_by(|left, right| left.path == right.path);
    errors.extend(ambiguous_stem_errors(&candidates));
    errors.sort();
    (candidates, errors)
}

fn ambiguous_stem_errors(candidates: &[CandidatePattern]) -> Vec<String> {
    let mut by_stem = std::collections::BTreeMap::<&str, Vec<&str>>::new();
    for candidate in candidates {
        let stem = candidate_stem(&candidate.candidate);
        by_stem.entry(stem).or_default().push(&candidate.candidate);
    }
    by_stem
        .into_iter()
        .filter_map(|(stem, paths)| {
            (paths.len() > 1)
                .then(|| format!("Ambiguous candidate stem '{stem}': {}", paths.join(", ")))
        })
        .collect()
}

fn candidate_pattern(path: &Path, project_root: &Path) -> Result<CandidatePattern, String> {
    let normalized = normalize_root_join(project_root, path);
    if !normalized.starts_with(project_root) {
        return Err(format!(
            "Candidate path outside project root: {}",
            path.display()
        ));
    }
    let candidate = display_path(project_root, &normalized);
    let stem = candidate_stem_from_path(&normalized, &candidate)?;
    let pattern = reference_pattern(stem).map_err(|error| {
        format!("Unable to build reference pattern for candidate {candidate}: {error}")
    })?;
    Ok(CandidatePattern {
        candidate,
        path: normalized,
        pattern,
    })
}

fn candidate_stem_from_path<'a>(path: &'a Path, candidate: &str) -> Result<&'a str, String> {
    let stem = path
        .file_stem()
        .ok_or_else(|| format!("Candidate file stem is absent: {candidate}"))?;
    let stem = stem
        .to_str()
        .ok_or_else(|| format!("Non-UTF8 candidate file name: {candidate}"))?;
    if stem.is_empty() {
        return Err(format!("Candidate file stem is absent: {candidate}"));
    }
    Ok(stem)
}

fn reference_pattern(stem: &str) -> Result<Regex, regex::Error> {
    let stem = regex::escape(stem);
    let module_end = r"(?:$|[^A-Za-z0-9_-])";
    let go_import_name = r"(?:[A-Za-z_][A-Za-z0-9_]*|[_.])";
    let path = format!(r#"["'](?:[^/"']*/)*{stem}(?:\.(?:rs|ts|tsx|js|jsx|py|go))?["']"#);
    Regex::new(&format!(
        r#"(?xs)
            \bmod\s+{stem}{module_end}
            | \buse\s+[^;]*?(?:^|[:{{,\s]){stem}{module_end}
            | \bfrom\s+(?:\.+)?(?:[A-Za-z_][A-Za-z0-9_]*\.)*{stem}\s+import\b
            | (?:\bimport|,)\s*(?:[A-Za-z_][A-Za-z0-9_]*\.)*{stem}(?:$|[\s,])
            | \bimport\s*(?:\(\s*)?(?:[^;]*?\bfrom\s+)?{path}
            | \bimport\s+{go_import_name}\s+{path}
            | \bimport\s*\(\s*(?:(?:{go_import_name}\s+)?["'][^"'\r\n]+["']\s*)*(?:{go_import_name}\s+)?{path}
            | \brequire\s*\(\s*{path}
        "#
    ))
}

fn collect_source_paths(dir: &Path, source_paths: &mut Vec<PathBuf>, errors: &mut Vec<String>) {
    let entries = match directory_entries(dir, errors) {
        Some(entries) => entries,
        None => return,
    };
    for entry in entries {
        collect_entry(entry.path(), source_paths, errors);
    }
}

fn directory_entries(dir: &Path, errors: &mut Vec<String>) -> Option<Vec<std::fs::DirEntry>> {
    let entries = std::fs::read_dir(dir)
        .map_err(|error| {
            errors.push(format!(
                "Unable to read source directory {}: {error}",
                display_debug_path(dir)
            ));
        })
        .ok()?;
    let mut entries = entries
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            errors.push(format!(
                "Unable to enumerate source directory {}: {error}",
                display_debug_path(dir)
            ));
        })
        .ok()?;
    entries.sort_by_key(|entry| entry.path());
    Some(entries)
}

fn collect_entry(path: PathBuf, source_paths: &mut Vec<PathBuf>, errors: &mut Vec<String>) {
    let file_type = match std::fs::symlink_metadata(&path) {
        Ok(metadata) => metadata.file_type(),
        Err(error) => {
            errors.push(format!(
                "Unable to inspect source path {}: {error}",
                display_debug_path(&path)
            ));
            return;
        }
    };
    if file_type.is_symlink() {
        collect_symlink(path, source_paths, errors);
    } else if file_type.is_dir() && !should_skip_directory(path.file_name()) {
        collect_source_paths(&path, source_paths, errors);
    } else if file_type.is_file() {
        add_source_path(path, source_paths, errors);
    }
}

fn collect_symlink(path: PathBuf, source_paths: &mut Vec<PathBuf>, errors: &mut Vec<String>) {
    match std::fs::metadata(&path) {
        Ok(metadata) if metadata.is_file() => add_source_path(path, source_paths, errors),
        Ok(_) => {}
        Err(error) => errors.push(format!(
            "Unable to inspect source symlink target {}: {error}",
            display_debug_path(&path)
        )),
    }
}

fn add_source_path(path: PathBuf, source_paths: &mut Vec<PathBuf>, errors: &mut Vec<String>) {
    match source_extension(path.extension()) {
        Ok(true) => source_paths.push(path),
        Ok(false) => {}
        Err(()) => errors.push(format!(
            "Non-UTF8 source extension: {}",
            display_debug_path(&path)
        )),
    }
}

fn should_skip_directory(name: Option<&OsStr>) -> bool {
    let Some(name) = name else {
        return false;
    };
    is_hidden_directory(name) || matches!(name.to_str(), Some("target" | "node_modules"))
}

#[cfg(unix)]
fn is_hidden_directory(name: &OsStr) -> bool {
    use std::os::unix::ffi::OsStrExt;

    name.as_bytes().first() == Some(&b'.')
}

#[cfg(not(unix))]
fn is_hidden_directory(name: &OsStr) -> bool {
    name.to_str().is_some_and(|name| name.starts_with('.'))
}

fn source_extension(extension: Option<&OsStr>) -> Result<bool, ()> {
    match extension {
        Some(extension) => extension
            .to_str()
            .map(|extension| SOURCE_EXTENSIONS.contains(&extension))
            .ok_or(()),
        None => Ok(false),
    }
}

fn normalize_root_join(project_root: &Path, path: &Path) -> PathBuf {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        project_root.join(path)
    };
    normalize_path(&joined)
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    normalized.push(component.as_os_str());
                }
            }
            _ => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

fn display_path(project_root: &Path, path: &Path) -> String {
    path.strip_prefix(project_root)
        .unwrap_or(path)
        .display()
        .to_string()
}

fn display_debug_path(path: &Path) -> String {
    format!("{path:?}")
}
