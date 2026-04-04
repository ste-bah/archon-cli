use std::collections::HashSet;
use std::fs;
use std::path::Path;

/// Discover and load all CLAUDE.md files from global to project.
///
/// Loading order:
/// 1. `~/.claude/CLAUDE.md` (global)
/// 2. Walk from filesystem root toward `working_dir`, checking each ancestor for:
///    - `.claude/CLAUDE.md` (preferred)
///    - `CLAUDE.md` (fallback)
/// 3. Check `working_dir` itself with the same preference
///
/// Each file is emitted with a section header: `# CLAUDE.md from {path}`
/// Files are deduplicated by canonical path so symlinks don't cause double-loading.
pub fn load_hierarchical_claude_md(working_dir: &Path) -> String {
    collect_claude_md_sections(working_dir, None)
}

/// Same as [`load_hierarchical_claude_md`] but truncates from the beginning
/// (global entries first) if the total exceeds `max_chars`, preserving the
/// most-local (project) content.
pub fn load_hierarchical_claude_md_with_limit(working_dir: &Path, max_chars: usize) -> String {
    collect_claude_md_sections(working_dir, Some(max_chars))
}

/// Internal: collect sections, optionally truncating.
fn collect_claude_md_sections(working_dir: &Path, max_chars: Option<usize>) -> String {
    let mut sections: Vec<String> = Vec::new();
    let mut seen: HashSet<std::path::PathBuf> = HashSet::new();

    // 1. Global: ~/.claude/CLAUDE.md
    if let Some(home) = dirs::home_dir() {
        let global_path = home.join(".claude").join("CLAUDE.md");
        try_load(&global_path, &mut sections, &mut seen);
    }

    // 2. Walk ancestors from root toward working_dir
    let canonical = match working_dir.canonicalize() {
        Ok(p) => p,
        Err(_) => working_dir.to_path_buf(),
    };

    let ancestors: Vec<&Path> = canonical.ancestors().collect();
    // ancestors goes from working_dir up to root; reverse to go root-first
    // Skip the last element (working_dir itself) -- we handle it in step 3.
    // Also skip the first element in reversed order if it's "/" (root) since
    // it's unlikely to have CLAUDE.md and we already checked global.
    for ancestor in ancestors.iter().rev() {
        // Skip the working_dir itself (handled after the loop)
        if *ancestor == canonical.as_path() {
            continue;
        }
        try_load_dir(ancestor, &mut sections, &mut seen);
    }

    // 3. Working dir itself
    try_load_dir(canonical.as_path(), &mut sections, &mut seen);

    let combined = sections.join("\n");

    match max_chars {
        Some(limit) if combined.len() > limit => truncate_from_front(sections, limit),
        _ => combined,
    }
}

/// Try to load CLAUDE.md from a directory, preferring `.claude/CLAUDE.md`.
fn try_load_dir(
    dir: &Path,
    sections: &mut Vec<String>,
    seen: &mut HashSet<std::path::PathBuf>,
) {
    let dot_claude = dir.join(".claude").join("CLAUDE.md");
    let plain = dir.join("CLAUDE.md");

    // Prefer .claude/CLAUDE.md; if it exists, skip the plain one
    if dot_claude.is_file() {
        try_load(&dot_claude, sections, seen);
    } else if plain.is_file() {
        try_load(&plain, sections, seen);
    }
}

/// Try to load a single CLAUDE.md file, deduplicating by canonical path.
fn try_load(
    path: &Path,
    sections: &mut Vec<String>,
    seen: &mut HashSet<std::path::PathBuf>,
) {
    let canon = match path.canonicalize() {
        Ok(p) => p,
        Err(_) => return,
    };

    if !seen.insert(canon) {
        return; // Already loaded this exact file
    }

    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return, // Skip non-UTF-8 or unreadable files
    };

    if content.is_empty() {
        return;
    }

    let header = format!("# CLAUDE.md from {}\n", path.display());
    sections.push(format!("{header}\n{content}"));
}

/// Truncate sections from the front (global first) until total fits in limit.
/// Returns as much of the most-local content as possible.
fn truncate_from_front(sections: Vec<String>, max_chars: usize) -> String {
    // Try dropping sections from the front until it fits
    for start_idx in 0..sections.len() {
        let candidate: String = sections[start_idx..].join("\n");
        if candidate.len() <= max_chars {
            return candidate;
        }
    }

    // Even the last section alone is too large -- truncate it from the front
    if let Some(last) = sections.last() {
        if last.len() > max_chars {
            return last[last.len() - max_chars..].to_string();
        }
        return last.clone();
    }

    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_from_front_preserves_last() {
        let sections = vec!["AAAA".to_string(), "BB".to_string()];
        let result = truncate_from_front(sections, 3);
        assert_eq!(result, "BB");
    }

    #[test]
    fn truncate_from_front_returns_empty_for_empty() {
        let result = truncate_from_front(Vec::new(), 10);
        assert!(result.is_empty());
    }
}
