use std::collections::HashSet;
use std::fs;
use std::path::Path;

/// Discover and load all ARCHON.md files from global to project.
///
/// Loading order:
/// 1. `~/.archon/ARCHON.md` (global, new) — falls back to `~/.claude/CLAUDE.md`
/// 2. Walk from filesystem root toward `working_dir`, checking each ancestor for:
///    - `.archon/ARCHON.md` (preferred)
///    - `.claude/CLAUDE.md` (deprecated fallback)
///    - `ARCHON.md` (root fallback)
///    - `CLAUDE.md` (root deprecated fallback)
/// 3. Check `working_dir` itself with the same preference
///
/// Each file is emitted with a section header: `# ARCHON.md from {path}`
/// Files are deduplicated by canonical path so symlinks don't cause double-loading.
pub fn load_hierarchical_archon_md(working_dir: &Path) -> String {
    collect_archon_md_sections(working_dir, None)
}

/// Same as [`load_hierarchical_archon_md`] but truncates from the beginning
/// (global entries first) if the total exceeds `max_chars`, preserving the
/// most-local (project) content.
pub fn load_hierarchical_archon_md_with_limit(working_dir: &Path, max_chars: usize) -> String {
    collect_archon_md_sections(working_dir, Some(max_chars))
}

/// Internal: collect sections, optionally truncating.
fn collect_archon_md_sections(working_dir: &Path, max_chars: Option<usize>) -> String {
    let mut sections: Vec<String> = Vec::new();
    let mut seen: HashSet<std::path::PathBuf> = HashSet::new();

    // 1. Global: ~/.archon/ARCHON.md (with ~/.claude/CLAUDE.md fallback)
    if let Some(home) = dirs::home_dir() {
        let new_global = home.join(".archon").join("ARCHON.md");
        let old_global = home.join(".claude").join("CLAUDE.md");
        if new_global.is_file() {
            try_load(&new_global, &mut sections, &mut seen);
        } else if old_global.is_file() {
            tracing::warn!(
                "Loading from deprecated path {}. Rename to {} to suppress this warning.",
                old_global.display(),
                new_global.display()
            );
            try_load(&old_global, &mut sections, &mut seen);
        }
    }

    // 2. Walk ancestors from root toward working_dir
    let canonical = match working_dir.canonicalize() {
        Ok(p) => p,
        Err(_) => working_dir.to_path_buf(),
    };

    let ancestors: Vec<&Path> = canonical.ancestors().collect();
    // ancestors goes from working_dir up to root; reverse to go root-first
    // Skip the last element (working_dir itself) -- we handle it in step 3.
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

/// Try to load ARCHON.md from a directory, with backward compat fallback.
///
/// Preference order:
/// 1. `.archon/ARCHON.md` (new, preferred)
/// 2. `.claude/CLAUDE.md` (deprecated fallback)
/// 3. `ARCHON.md` (root-level new)
/// 4. `CLAUDE.md` (root-level deprecated fallback)
fn try_load_dir(dir: &Path, sections: &mut Vec<String>, seen: &mut HashSet<std::path::PathBuf>) {
    let dot_archon = dir.join(".archon").join("ARCHON.md");
    let dot_claude = dir.join(".claude").join("CLAUDE.md");
    let plain_archon = dir.join("ARCHON.md");
    let plain_claude = dir.join("CLAUDE.md");

    // Prefer .archon/ARCHON.md; fall back to .claude/CLAUDE.md; then root files
    if dot_archon.is_file() {
        try_load(&dot_archon, sections, seen);
    } else if dot_claude.is_file() {
        tracing::warn!(
            "Loading from deprecated path {}. Rename to {} to suppress this warning.",
            dot_claude.display(),
            dot_archon.display()
        );
        try_load(&dot_claude, sections, seen);
    } else if plain_archon.is_file() {
        try_load(&plain_archon, sections, seen);
    } else if plain_claude.is_file() {
        tracing::warn!(
            "Loading from deprecated path {}. Rename to {} to suppress this warning.",
            plain_claude.display(),
            plain_archon.display()
        );
        try_load(&plain_claude, sections, seen);
    }
}

/// Try to load a single instructions file, deduplicating by canonical path.
fn try_load(path: &Path, sections: &mut Vec<String>, seen: &mut HashSet<std::path::PathBuf>) {
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

    let header = format!("# ARCHON.md from {}\n", path.display());
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
