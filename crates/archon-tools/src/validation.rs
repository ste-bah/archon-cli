//! Input validation for slash commands: model names, effort levels, permission modes.
//!
//! All validators return `Ok(canonical_value)` on success or `Err(user_facing_message)`
//! with a helpful suggestion when the input is close to a valid option.

/// Known shortcut names mapped to full model identifiers.
pub const KNOWN_SHORTCUTS: &[(&str, &str)] = &[
    ("opus", "claude-opus-4-6"),
    ("sonnet", "claude-sonnet-4-6"),
    ("haiku", "claude-haiku-4-5-20251001"),
];

/// Full model identifiers accepted without shortcut expansion.
pub const KNOWN_MODEL_IDS: &[&str] = &[
    "claude-opus-4-6",
    "claude-sonnet-4-6",
    "claude-haiku-4-5-20251001",
];

/// Valid effort level values (case-insensitive).
pub const VALID_EFFORT_LEVELS: &[&str] = &["high", "medium", "low"];

/// Valid permission mode identifiers (case-sensitive to match Claude Code conventions).
pub const VALID_PERMISSION_MODES: &[&str] = &[
    "default",
    "acceptEdits",
    "plan",
    "auto",
    "dontAsk",
    "bypassPermissions",
];

/// Legacy permission mode aliases that resolve to a canonical mode.
pub const LEGACY_PERMISSION_ALIASES: &[(&str, &str)] =
    &[("ask", "default"), ("yolo", "bypassPermissions")];

// ---------------------------------------------------------------------------
// Edit distance
// ---------------------------------------------------------------------------

/// Compute the Levenshtein edit distance between two strings.
///
/// Comparison is **case-insensitive** — both inputs are lowercased before measuring.
pub fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.to_lowercase().chars().collect();
    let b: Vec<char> = b.to_lowercase().chars().collect();

    let m = a.len();
    let n = b.len();

    // dp[i] represents the distance between a[..i] and b[..j] (updated in-place).
    let mut prev: Vec<usize> = (0..=n).collect();
    let mut curr: Vec<usize> = vec![0; n + 1];

    for i in 1..=m {
        curr[0] = i;
        for j in 1..=n {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }

    prev[n]
}

// ---------------------------------------------------------------------------
// Fuzzy suggestion helper
// ---------------------------------------------------------------------------

/// Find the closest match among `candidates` within `max_distance`.
///
/// Returns `Some((candidate, distance))` for the best match, or `None` if no
/// candidate is within the threshold. Ties are broken by smallest distance first,
/// then by list order (first candidate wins).
fn closest_match<'a>(
    input: &str,
    candidates: &[&'a str],
    max_distance: usize,
) -> Option<(&'a str, usize)> {
    let mut best: Option<(&str, usize)> = None;

    for &candidate in candidates {
        let dist = edit_distance(input, candidate);
        if dist <= max_distance {
            match best {
                None => best = Some((candidate, dist)),
                Some((_, best_dist)) if dist < best_dist => best = Some((candidate, dist)),
                _ => {} // keep existing (first wins on tie)
            }
        }
    }

    best
}

// ---------------------------------------------------------------------------
// Model name validation
// ---------------------------------------------------------------------------

/// Validate and resolve a model name input.
///
/// Accepts:
/// - Shortcut names: `opus`, `sonnet`, `haiku` (case-insensitive)
/// - Full model IDs: `claude-opus-4-6`, etc. (case-insensitive)
///
/// On failure, suggests the closest match if edit distance <= 2.
pub fn validate_model_name(input: &str) -> Result<String, String> {
    let lower = input.trim().to_lowercase();

    // Check shortcuts (case-insensitive)
    for &(shortcut, full_id) in KNOWN_SHORTCUTS {
        if lower == shortcut {
            return Ok(full_id.to_string());
        }
    }

    // Check full model IDs (case-insensitive)
    for &model_id in KNOWN_MODEL_IDS {
        if lower == model_id.to_lowercase() {
            return Ok(model_id.to_string());
        }
    }

    // Build candidate list: shortcuts + full IDs
    let shortcut_names: Vec<&str> = KNOWN_SHORTCUTS.iter().map(|(s, _)| *s).collect();
    let mut all_candidates: Vec<&str> = shortcut_names.clone();
    all_candidates.extend_from_slice(KNOWN_MODEL_IDS);

    if let Some((suggestion, _)) = closest_match(input, &all_candidates, 2) {
        return Err(format!(
            "Error: Unknown model '{input}'. Did you mean '{suggestion}'?\n\n\
             Valid shortcuts: {shortcuts}\n\
             Valid model IDs: {ids}",
            shortcuts = shortcut_names.join(", "),
            ids = KNOWN_MODEL_IDS.join(", "),
        ));
    }

    Err(format!(
        "Error: Unknown model '{input}'.\n\n\
         Valid shortcuts: {shortcuts}\n\
         Valid model IDs: {ids}",
        shortcuts = shortcut_names.join(", "),
        ids = KNOWN_MODEL_IDS.join(", "),
    ))
}

// ---------------------------------------------------------------------------
// Effort level validation
// ---------------------------------------------------------------------------

/// Validate an effort level input (case-insensitive).
///
/// On failure, suggests the closest match if edit distance <= 2.
pub fn validate_effort_level(input: &str) -> Result<String, String> {
    let lower = input.trim().to_lowercase();

    for &level in VALID_EFFORT_LEVELS {
        if lower == level {
            return Ok(level.to_string());
        }
    }

    if let Some((suggestion, _)) = closest_match(input, VALID_EFFORT_LEVELS, 2) {
        return Err(format!(
            "Error: Invalid effort level '{input}'. Did you mean '{suggestion}'?\n\n\
             Valid levels: {levels}",
            levels = VALID_EFFORT_LEVELS.join(", "),
        ));
    }

    Err(format!(
        "Error: Invalid effort level '{input}'.\n\n\
         Valid levels: {levels}",
        levels = VALID_EFFORT_LEVELS.join(", "),
    ))
}

// ---------------------------------------------------------------------------
// Permission mode validation
// ---------------------------------------------------------------------------

/// Validate a permission mode input.
///
/// Accepts canonical modes (case-sensitive) and legacy aliases (`ask` -> `default`,
/// `yolo` -> `bypassPermissions`). On failure, suggests the closest match among
/// all modes and aliases if edit distance <= 2.
pub fn validate_permission_mode(input: &str) -> Result<String, String> {
    let trimmed = input.trim();

    // Exact match against canonical modes (case-sensitive)
    for &mode in VALID_PERMISSION_MODES {
        if trimmed == mode {
            return Ok(mode.to_string());
        }
    }

    // Legacy alias resolution (case-sensitive)
    for &(alias, canonical) in LEGACY_PERMISSION_ALIASES {
        if trimmed == alias {
            return Ok(canonical.to_string());
        }
    }

    // Build candidates: canonical modes + legacy aliases
    let mut all_candidates: Vec<&str> = VALID_PERMISSION_MODES.to_vec();
    let alias_names: Vec<&str> = LEGACY_PERMISSION_ALIASES.iter().map(|(a, _)| *a).collect();
    all_candidates.extend_from_slice(&alias_names);

    if let Some((suggestion, _)) = closest_match(trimmed, &all_candidates, 2) {
        return Err(format!(
            "Error: Invalid permission mode '{trimmed}'. Did you mean '{suggestion}'?\n\n\
             Valid modes: {modes}\n\
             Legacy aliases: {aliases}",
            modes = VALID_PERMISSION_MODES.join(", "),
            aliases = LEGACY_PERMISSION_ALIASES
                .iter()
                .map(|(a, c)| format!("{a} -> {c}"))
                .collect::<Vec<_>>()
                .join(", "),
        ));
    }

    Err(format!(
        "Error: Invalid permission mode '{trimmed}'.\n\n\
         Valid modes: {modes}\n\
         Legacy aliases: {aliases}",
        modes = VALID_PERMISSION_MODES.join(", "),
        aliases = LEGACY_PERMISSION_ALIASES
            .iter()
            .map(|(a, c)| format!("{a} -> {c}"))
            .collect::<Vec<_>>()
            .join(", "),
    ))
}
