//! Gate: the written-practice documentation stays wired together.
//!
//! Three separate failures this file exists to catch, each of which has already
//! happened in this repository:
//!
//! 1. **A link that points at nothing.** `docs/providers/bedrock.md` was linked
//!    from three committed pages for nine days and had never existed
//!    (`docs/postmortem/0005-...`).
//! 2. **A link that points at something only the author can see.** `.gitignore`
//!    excludes `/docs/*` and re-includes it directory by directory, and `git add`
//!    on an ignored path is a silent no-op that exits 0. A file can therefore sit
//!    on disk looking committed while being absent from every clone — so
//!    "the target exists" is checked against **git**, not against the filesystem.
//! 3. **An entry point that stops pointing at the practice.** A postmortem
//!    nobody is routed to is a diary entry. The wiring assertions below name the
//!    documents that must reference the practice, so deleting the link from
//!    `ARCHON.md` or `README.md` fails here rather than going unnoticed.
//!
//! These are all instances of the rule the practice is built on
//! (`docs/defensive-patterns.md`, DP-0): a check whose scan target can vanish
//! must fail, not pass. Accordingly every scan below asserts a non-zero
//! denominator — a run that inspected no files, found no links, or resolved no
//! headings fails, instead of reporting a clean tree it never looked at.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Entry points that must route a reader to the practice, and the paths each
/// must link to. Removing one of these links is the way this practice dies, so
/// it is the thing most worth asserting.
const REQUIRED_WIRING: &[(&str, &[&str])] = &[
    (
        "README.md",
        &[
            "docs/defensive-patterns.md",
            "docs/postmortem/",
            "docs/decisions/",
        ],
    ),
    (
        "ARCHON.md",
        &[
            "docs/defensive-patterns.md",
            "docs/postmortem/",
            "docs/decisions/",
        ],
    ),
    (
        "CONTRIBUTING.md",
        &[
            "docs/defensive-patterns.md",
            "docs/postmortem/",
            "docs/decisions/",
        ],
    ),
    (
        "docs/README.md",
        &[
            "defensive-patterns.md",
            "postmortem/README.md",
            "decisions/README.md",
        ],
    ),
];

/// Every postmortem must be listed in the postmortem index, and every decision
/// record in the decisions index. A note that is not indexed is not findable,
/// which is the only thing that makes any of this worth writing.
const INDEXED_TREES: &[(&str, &str)] = &[
    ("docs/postmortem", "docs/postmortem/README.md"),
    ("docs/decisions", "docs/decisions/README.md"),
];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

/// Paths git tracks, as forward-slash repo-relative strings.
///
/// `git ls-files` is the authority rather than the filesystem: an ignored file
/// is present for the author and absent for everyone else, and that asymmetry is
/// exactly the defect in postmortem 0005.
fn tracked_paths() -> BTreeSet<String> {
    let output = Command::new("git")
        .args(["ls-files", "-z"])
        .current_dir(repo_root())
        .output()
        .expect("failed to run `git ls-files`; this gate needs a git checkout");

    assert!(
        output.status.success(),
        "`git ls-files` exited {:?}: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );

    let paths: BTreeSet<String> = String::from_utf8_lossy(&output.stdout)
        .split('\0')
        .filter(|entry| !entry.is_empty())
        .map(|entry| entry.replace('\\', "/"))
        .collect();

    assert!(
        !paths.is_empty(),
        "`git ls-files` returned no paths. Nothing below would have been checked, \
         so this fails rather than reporting a clean tree (DP-0)."
    );
    paths
}

/// Markdown documents this gate reads: the whole committed `docs/` tree plus the
/// three root entry points.
fn documents(tracked: &BTreeSet<String>) -> Vec<String> {
    let docs: Vec<String> = tracked
        .iter()
        .filter(|path| path.starts_with("docs/") && path.ends_with(".md"))
        .cloned()
        .collect();

    assert!(
        docs.len() > 50,
        "expected the committed docs tree to hold many markdown pages, found {}. \
         Either the scan is pointed at the wrong place or `git ls-files` is being \
         misread; a near-empty scan must fail (DP-0).",
        docs.len()
    );

    let mut all = docs;
    for root_doc in ["README.md", "CONTRIBUTING.md", "ARCHON.md"] {
        assert!(
            tracked.contains(root_doc),
            "{root_doc} is not tracked by git, so the wiring assertions below \
             would silently check nothing"
        );
        all.push(root_doc.to_string());
    }
    all
}

/// Inline markdown links: `[text](target)`, with an optional `"title"`.
///
/// Deliberately hand-rolled rather than regex-based, to avoid adding a
/// dependency for a gate. Skips fenced code blocks, so the `.gitignore` and
/// shell samples quoted inside the postmortems are not mistaken for links.
fn links(markdown: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut in_fence = false;

    for line in markdown.lines() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }

        let bytes: Vec<char> = line.chars().collect();
        let mut index = 0;
        while index < bytes.len() {
            if bytes[index] != '[' {
                index += 1;
                continue;
            }
            let Some(close_bracket) = find_from(&bytes, index, ']') else {
                break;
            };
            if bytes.get(close_bracket + 1) != Some(&'(') {
                index = close_bracket + 1;
                continue;
            }
            let Some(close_paren) = find_from(&bytes, close_bracket + 2, ')') else {
                break;
            };
            let target: String = bytes[close_bracket + 2..close_paren].iter().collect();
            // Strip an optional link title: `(path "Title")`.
            let target = target.split_whitespace().next().unwrap_or("").to_string();
            if !target.is_empty() {
                found.push(target);
            }
            index = close_paren + 1;
        }
    }
    found
}

fn find_from(chars: &[char], start: usize, needle: char) -> Option<usize> {
    chars
        .iter()
        .skip(start)
        .position(|c| *c == needle)
        .map(|offset| start + offset)
}

/// GitHub's heading-anchor slug: lowercase, drop backticks and punctuation, and
/// turn each space into one hyphen (runs of spaces are *not* collapsed).
fn slug(heading: &str) -> String {
    let without_links = strip_inline_links(heading.trim());
    let mut out = String::new();
    for ch in without_links.chars() {
        if ch == '`' {
            continue;
        }
        if ch == ' ' {
            out.push('-');
        } else if ch.is_alphanumeric() || ch == '-' || ch == '_' {
            out.extend(ch.to_lowercase());
        }
    }
    out
}

fn strip_inline_links(text: &str) -> String {
    let mut out = String::new();
    let chars: Vec<char> = text.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '['
            && let Some(close_bracket) = find_from(&chars, index, ']')
            && chars.get(close_bracket + 1) == Some(&'(')
            && let Some(close_paren) = find_from(&chars, close_bracket + 2, ')')
        {
            out.extend(&chars[index + 1..close_bracket]);
            index = close_paren + 1;
            continue;
        }
        out.push(chars[index]);
        index += 1;
    }
    out
}

fn headings(markdown: &str) -> BTreeSet<String> {
    let mut in_fence = false;
    let mut anchors = BTreeSet::new();
    for line in markdown.lines() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        if let Some(rest) = line.strip_prefix('#') {
            let text = rest.trim_start_matches('#');
            if text.starts_with(' ') {
                anchors.insert(slug(text));
            }
        }
    }
    anchors
}

fn read(relative: &str) -> String {
    std::fs::read_to_string(repo_root().join(relative))
        .unwrap_or_else(|error| panic!("failed to read {relative}: {error}"))
}

/// Resolve a link target relative to the document containing it, normalising
/// `..` segments, and return it as a repo-relative forward-slash path.
fn resolve(from_doc: &str, target: &str) -> Option<String> {
    let mut parts: Vec<&str> = from_doc.split('/').collect();
    parts.pop(); // drop the filename
    for segment in target.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            other => parts.push(other),
        }
    }
    Some(parts.join("/"))
}

#[test]
fn every_relative_link_resolves_to_a_committed_file() {
    let tracked = tracked_paths();
    let docs = documents(&tracked);

    // Cache of path -> heading anchors, so each page is parsed once.
    let mut anchors: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut checked_links = 0usize;
    let mut checked_anchors = 0usize;
    let mut failures: Vec<String> = Vec::new();

    for doc in &docs {
        let body = read(doc);
        for target in links(&body) {
            if target.starts_with("http://")
                || target.starts_with("https://")
                || target.starts_with("mailto:")
            {
                continue;
            }

            let (path_part, anchor) = match target.split_once('#') {
                Some((path, fragment)) => (path, Some(fragment.to_string())),
                None => (target.as_str(), None),
            };

            // A pure `#anchor` link refers to the current document.
            let resolved = if path_part.is_empty() {
                doc.clone()
            } else {
                match resolve(doc, path_part) {
                    Some(resolved) => resolved,
                    None => {
                        failures.push(format!("{doc} -> {target} (escapes the repository root)"));
                        continue;
                    }
                }
            };

            checked_links += 1;

            // A directory link is satisfied by any tracked file beneath it.
            let is_dir_link = path_part.ends_with('/');
            let exists = if is_dir_link {
                let prefix = format!("{}/", resolved.trim_end_matches('/'));
                tracked.iter().any(|path| path.starts_with(&prefix))
            } else {
                tracked.contains(&resolved)
            };

            if !exists {
                let on_disk = repo_root().join(&resolved).exists();
                let hint = if on_disk {
                    " — it EXISTS ON DISK but git does not track it. Check \
                     `git check-ignore -v` for that path: `git add` on an ignored \
                     path is a silent no-op (see docs/postmortem/0005)."
                } else {
                    " — no such file"
                };
                failures.push(format!("{doc} -> {target} (resolved to {resolved}){hint}"));
                continue;
            }

            let Some(anchor) = anchor else { continue };
            if anchor.is_empty() || is_dir_link {
                continue;
            }
            let page_anchors = anchors
                .entry(resolved.clone())
                .or_insert_with(|| headings(&read(&resolved)));
            checked_anchors += 1;
            if !page_anchors.contains(&anchor) {
                failures.push(format!(
                    "{doc} -> {target}: {resolved} has no heading with anchor `{anchor}`"
                ));
            }
        }
    }

    assert!(
        checked_links > 300,
        "only {checked_links} links were inspected across {} documents. The link \
         parser has stopped matching; a scan that inspects almost nothing must \
         fail rather than report clean (DP-0).",
        docs.len()
    );
    assert!(
        checked_anchors > 20,
        "only {checked_anchors} heading anchors were inspected. The anchor half of \
         this gate has gone vacuous (DP-0)."
    );

    assert!(
        failures.is_empty(),
        "{} broken documentation cross-reference(s) out of {checked_links} links \
         and {checked_anchors} anchors checked:\n  {}",
        failures.len(),
        failures.join("\n  ")
    );
}

#[test]
fn the_entry_points_still_route_readers_to_the_practice() {
    let tracked = tracked_paths();
    let mut failures = Vec::new();
    let mut checked = 0usize;

    for (doc, required) in REQUIRED_WIRING {
        assert!(
            tracked.contains(*doc),
            "{doc} is not tracked; this assertion would check nothing (DP-0)"
        );
        let body = read(doc);
        let targets = links(&body);
        assert!(
            !targets.is_empty(),
            "no links parsed out of {doc}; the parser has gone vacuous (DP-0)"
        );

        for needle in *required {
            checked += 1;
            let present = targets.iter().any(|target| {
                let path = target.split('#').next().unwrap_or("");
                path == *needle || path.starts_with(needle) || path.ends_with(needle)
            });
            if !present {
                failures.push(format!(
                    "{doc} no longer links to `{needle}`. The practice survives only \
                     as long as the entry points point at it — restore the link, or \
                     update REQUIRED_WIRING and say in the commit message where a \
                     reader is meant to find it instead."
                ));
            }
        }
    }

    assert!(checked >= 12, "only {checked} wiring links asserted (DP-0)");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn every_postmortem_and_decision_record_is_listed_in_its_index() {
    let tracked = tracked_paths();
    let mut failures = Vec::new();
    let mut checked = 0usize;

    for (tree, index) in INDEXED_TREES {
        assert!(
            tracked.contains(*index),
            "{index} is not tracked by git; the index assertion is vacuous (DP-0)"
        );
        let index_body = read(index);
        let listed: BTreeSet<String> = links(&index_body)
            .iter()
            .filter_map(|target| {
                let path = target.split('#').next().unwrap_or("");
                resolve(index, path)
            })
            .collect();

        let prefix = format!("{tree}/");
        let notes: Vec<&String> = tracked
            .iter()
            .filter(|path| {
                path.starts_with(&prefix) && path.ends_with(".md") && !path.ends_with("README.md")
            })
            .collect();

        assert!(
            !notes.is_empty(),
            "no notes found under {tree}/. Either the tree was emptied or it is not \
             committed — both must fail here rather than pass silently (DP-0)."
        );

        for note in notes {
            checked += 1;
            if !listed.contains(note) {
                failures.push(format!(
                    "{note} is committed but not listed in {index}. An unindexed note \
                     is unfindable, which defeats the point of writing it."
                ));
            }
        }
    }

    assert!(checked >= 6, "only {checked} notes inspected (DP-0)");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
