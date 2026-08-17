//! Skill catalogue advertised to the model (#187).
//!
//! The `Skill` tool has always been registered, but nothing told the model a
//! skill existed — discovery required speculatively calling `Skill(list)` on
//! the chance something relevant was there. Undiscoverable and absent are
//! indistinguishable from inside a context window, so in practice no skill was
//! ever invoked without the user typing the slash command.
//!
//! This renders the catalogue into the system prompt so relevance can be
//! judged directly. Only [`Skill::agent_invocable`] skills are listed: the
//! descriptor-only builtins return `SkillOutput::Text`, which renders in the
//! TUI and never reaches the model, so listing them is pure token cost.
//!
//! **Cache placement.** The catalogue is dynamic — it changes when a project
//! adds a `SKILL.md`. It must land outside the cached prompt prefix or it
//! invalidates the prompt cache for every session in that project (#178).
//! Callers put it in the `dynamic` section for exactly this reason.

use std::path::Path;

use super::{builtin, discovery};

/// Longest description rendered per skill before truncation.
///
/// Descriptions are authored as trigger conditions and are normally well
/// under this. The cap only bounds a pathological `SKILL.md` — a project with
/// many skills still gets all of them, just not an unbounded prompt.
const MAX_DESCRIPTION: usize = 240;

/// Render the agent-invocable skill catalogue, or `None` when none qualify.
///
/// `None` rather than an empty string so callers emit no section at all
/// instead of a header with nothing under it.
pub fn render_for_prompt(working_dir: &Path) -> Option<String> {
    let mut registry = builtin::register_builtins();
    for skill in discovery::discover_user_skills(working_dir) {
        registry.register(Box::new(skill));
    }
    render_entries(&registry.list_agent_invocable())
}

/// Split declared skill names into those that resolve and those that do not.
///
/// Agent definitions may name skills that no longer exist, or — as 43 shipped
/// definitions do — name capability descriptors that were never registry
/// entries at all. Instructing a model to invoke a name that resolves to
/// nothing produces a silent no-op, so callers list only the first half and
/// warn about the second.
///
/// Resolution goes through `resolve`, so aliases count as known.
pub fn partition_known(names: &[String], working_dir: &Path) -> (Vec<String>, Vec<String>) {
    let mut registry = builtin::register_builtins();
    for skill in discovery::discover_user_skills(working_dir) {
        registry.register(Box::new(skill));
    }
    names
        .iter()
        .cloned()
        .partition(|name| registry.resolve(name).is_some())
}

/// Render `(name, description)` pairs into the prompt section.
///
/// Split from [`render_for_prompt`] so the wording can be tested without
/// touching the filesystem or the builtin registry.
pub fn render_entries(entries: &[(&str, &str)]) -> Option<String> {
    if entries.is_empty() {
        return None;
    }

    let mut out = String::from(
        "## Skills\n\n\
         Each skill below is a written method for a kind of work. Load one with \
         the `Skill` tool (`action=\"invoke\"`, `name=<name>`) when the task in \
         front of you matches what it covers — you do not need to be asked, and \
         you do not need to call `action=\"list\"` first.\n\n\
         Judge relevance from the description, not the name. Invoke a skill when \
         following it would change how you approach the work; if none fit, carry \
         on normally rather than forcing one.\n\n",
    );

    for (name, description) in entries {
        out.push_str("- `");
        out.push_str(name);
        out.push('`');
        let description = description.trim();
        if !description.is_empty() {
            out.push_str(" — ");
            out.push_str(&truncate(description, MAX_DESCRIPTION));
        }
        out.push('\n');
    }

    Some(out)
}

/// Truncate on a char boundary, appending an ellipsis when shortened.
fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_entries_renders_nothing() {
        assert!(
            render_entries(&[]).is_none(),
            "an empty catalogue must emit no section, not a bare header"
        );
    }

    #[test]
    fn entries_are_listed_with_descriptions() {
        let rendered = render_entries(&[("tdd", "Use when building a feature")]).unwrap();

        assert!(rendered.contains("`tdd`"), "{rendered}");
        assert!(
            rendered.contains("Use when building a feature"),
            "{rendered}"
        );
    }

    /// The point of the section: the model must know it may act unprompted,
    /// and must not think a `list` round-trip is required first.
    #[test]
    fn the_protocol_grants_unprompted_invocation() {
        let rendered = render_entries(&[("tdd", "d")]).unwrap();

        assert!(rendered.contains("Skill"), "names the tool: {rendered}");
        assert!(
            rendered.contains("do not need to be asked"),
            "must authorise unprompted use: {rendered}"
        );
        assert!(
            rendered.contains("do not need to call"),
            "must not imply a list round-trip: {rendered}"
        );
    }

    #[test]
    fn a_missing_description_still_lists_the_skill() {
        let rendered = render_entries(&[("bare", "")]).unwrap();

        assert!(rendered.contains("`bare`"), "{rendered}");
        assert!(!rendered.contains("— \n"), "no dangling dash: {rendered}");
    }

    #[test]
    fn long_descriptions_are_bounded() {
        let long = "x".repeat(MAX_DESCRIPTION * 2);
        let rendered = render_entries(&[("verbose", long.as_str())]).unwrap();

        assert!(rendered.contains('…'), "should be truncated: {rendered}");
        assert!(
            !rendered.contains(long.as_str()),
            "the full description must not survive truncation"
        );
    }

    /// Truncation must not split a multi-byte character.
    #[test]
    fn truncation_respects_char_boundaries() {
        let text = "é".repeat(MAX_DESCRIPTION + 10);
        let truncated = truncate(&text, MAX_DESCRIPTION);

        assert_eq!(truncated.chars().count(), MAX_DESCRIPTION + 1);
    }

    /// Descriptor-only builtins (`/help`, `/cost`) render as TUI text and
    /// never reach the model, so advertising them is wasted prompt.
    #[test]
    fn builtins_that_cannot_reach_the_model_are_excluded() {
        let registry = builtin::register_builtins();

        let listed = registry.list_agent_invocable();
        let names: Vec<&str> = listed.iter().map(|(n, _)| *n).collect();

        assert!(!names.contains(&"help"), "descriptor-only: {names:?}");
        assert!(!names.contains(&"cost"), "descriptor-only: {names:?}");
        assert!(
            names.contains(&"tdd"),
            "embedded methodology skills must be advertised: {names:?}"
        );
        assert!(
            listed.len() < registry.list_all().len(),
            "the catalogue is a strict subset of the registry"
        );
    }

    /// A project skill must reach the model without a restart-time config
    /// step — dropping a `SKILL.md` in is the whole interface.
    #[test]
    fn a_project_skill_is_advertised() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path().join(".archon/skills/house-style");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            "---\nname: house-style\ndescription: Use when writing prose\n---\nBody.",
        )
        .unwrap();

        let rendered = render_for_prompt(tmp.path()).unwrap();

        assert!(rendered.contains("`house-style`"), "{rendered}");
        assert!(rendered.contains("Use when writing prose"), "{rendered}");
    }
}
