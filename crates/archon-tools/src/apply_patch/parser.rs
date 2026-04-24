// TASK-P0-B.5 (#183): ApplyPatch — unified-diff parser.
//
// Written from the published unified-diff format spec (POSIX
// `diff -u` output shape); no external crate is used. The full
// reference shape accepted here is:
//
//   --- a/<path>
//   +++ b/<path>
//   @@ -<old_start>,<old_len> +<new_start>,<new_len> @@
//    <context line verbatim>
//   -<line to remove>
//   +<line to add>
//
// Multi-hunk patches repeat the `@@ ... @@` header. Line counts in
// the header are 1-based. A hunk header may omit the length, e.g.
// `@@ -12 +12 @@` means length 1 on that side. A body line starting
// with ' ' is context, '-' is remove, '+' is add. Lines starting
// with '\' (e.g. `\ No newline at end of file`) are metadata and
// silently skipped.

// ---------------------------------------------------------------------------
// Internal parser types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum HunkLine {
    Context(String),
    Remove(String),
    Add(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Hunk {
    pub(super) old_start: usize,
    pub(super) old_len: usize,
    pub(super) new_start: usize,
    pub(super) new_len: usize,
    pub(super) lines: Vec<HunkLine>,
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

/// Parse a unified-diff patch into a sequence of hunks.
///
/// Requires the `--- a/...` and `+++ b/...` headers before the first
/// `@@` hunk header. The file-path portion of those headers is not
/// required to match the target path — the tool's caller is the source
/// of truth for which file is being patched.
pub(super) fn parse_hunks(patch: &str) -> Result<Vec<Hunk>, String> {
    if patch.trim().is_empty() {
        return Err("patch is empty".to_string());
    }

    let mut lines = patch.lines().peekable();

    // File headers: require both `--- ` and `+++ ` before first hunk.
    let mut saw_minus_header = false;
    let mut saw_plus_header = false;
    while let Some(&line) = lines.peek() {
        if line.starts_with("--- ") {
            saw_minus_header = true;
            lines.next();
        } else if line.starts_with("+++ ") {
            saw_plus_header = true;
            lines.next();
        } else if line.starts_with("@@") {
            break;
        } else {
            // Allow/ignore any preamble lines until we hit headers or a hunk
            // (for example "diff --git a/x b/x" or "index abc..def" lines).
            // But if we never saw proper headers, the next `@@` check below
            // will still reject malformed patches.
            lines.next();
        }
    }

    if !saw_minus_header || !saw_plus_header {
        return Err(
            "patch is missing required '--- a/...' or '+++ b/...' file headers".to_string(),
        );
    }

    let mut hunks: Vec<Hunk> = Vec::new();
    let mut current: Option<Hunk> = None;

    for line in lines {
        if let Some(rest) = line.strip_prefix("@@") {
            // Close any prior hunk and start a new one.
            if let Some(h) = current.take() {
                hunks.push(h);
            }
            let header = parse_hunk_header(rest)?;
            current = Some(header);
            continue;
        }

        let Some(h) = current.as_mut() else {
            // Body line before any `@@` header — malformed.
            return Err(format!("unexpected line before first hunk header: {line:?}"));
        };

        // Classify body line by leading byte. An empty line in the diff
        // is treated as an empty context line (common in diffs where the
        // trailing space on a context line has been stripped by mailers).
        if line.is_empty() {
            h.lines.push(HunkLine::Context(String::new()));
            continue;
        }
        let (tag, rest) = line.split_at(1);
        match tag {
            " " => h.lines.push(HunkLine::Context(rest.to_string())),
            "-" => h.lines.push(HunkLine::Remove(rest.to_string())),
            "+" => h.lines.push(HunkLine::Add(rest.to_string())),
            "\\" => {
                // "\ No newline at end of file" metadata — ignore. The
                // trailing-newline handling in `apply_hunks` covers the
                // actual behaviour.
                continue;
            }
            _ => {
                return Err(format!(
                    "unrecognised hunk body line (must start with ' ', '-', '+', or '\\'): {line:?}"
                ));
            }
        }
    }

    if let Some(h) = current.take() {
        hunks.push(h);
    }

    if hunks.is_empty() {
        return Err("patch has file headers but no '@@' hunks".to_string());
    }

    Ok(hunks)
}

/// Parse the numeric section of a hunk header such as
/// ` -12,7 +14,8 @@ optional section heading`.
///
/// The input to this helper is the substring that follows the leading
/// `@@` in the raw header line. Both length fields are optional; when
/// omitted the spec defines the length as 1.
fn parse_hunk_header(rest: &str) -> Result<Hunk, String> {
    // Strip the closing `@@` plus anything after it (section heading).
    let (numeric, _tail) = rest
        .split_once("@@")
        .ok_or_else(|| format!("hunk header missing closing '@@': @@{rest}"))?;

    let numeric = numeric.trim();
    // Expected shape: "-old_start[,old_len] +new_start[,new_len]"
    let mut parts = numeric.split_whitespace();
    let old_spec = parts
        .next()
        .ok_or_else(|| format!("hunk header missing old range: @@{rest}"))?;
    let new_spec = parts
        .next()
        .ok_or_else(|| format!("hunk header missing new range: @@{rest}"))?;

    let old_spec = old_spec
        .strip_prefix('-')
        .ok_or_else(|| format!("hunk old range must start with '-': {old_spec}"))?;
    let new_spec = new_spec
        .strip_prefix('+')
        .ok_or_else(|| format!("hunk new range must start with '+': {new_spec}"))?;

    let (old_start, old_len) = parse_range(old_spec)?;
    let (new_start, new_len) = parse_range(new_spec)?;

    Ok(Hunk {
        old_start,
        old_len,
        new_start,
        new_len,
        lines: Vec::new(),
    })
}

fn parse_range(spec: &str) -> Result<(usize, usize), String> {
    if let Some((start, len)) = spec.split_once(',') {
        let start = start
            .parse::<usize>()
            .map_err(|e| format!("invalid hunk range start {start:?}: {e}"))?;
        let len = len
            .parse::<usize>()
            .map_err(|e| format!("invalid hunk range length {len:?}: {e}"))?;
        Ok((start, len))
    } else {
        let start = spec
            .parse::<usize>()
            .map_err(|e| format!("invalid hunk range {spec:?}: {e}"))?;
        // Per spec: omitted length means 1.
        Ok((start, 1))
    }
}
