//! Writing style profile management and prompt injection.
//!
//! Manages writing style profiles (language variant, citation style, formality)
//! and conditionally injects style guidelines into Phase 6 research agent prompts.
//! Default is American English, APA citation, formal academic tone.

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Default style prompt (American English, APA, formal).
pub const DEFAULT_STYLE_PROMPT: &str = "\
Regional Language Settings:\n\
- Use American English spelling conventions\n\
- Examples: color, organization, analyze, center, behavior\n\
- Use APA citation style\n\
- Formality: Academic, formal tone";

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Characteristics describing a writing style.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StyleCharacteristics {
    /// "US" or "UK"
    pub language_variant: String,
    /// "formal", "semi-formal", "informal"
    pub formality_level: String,
    /// "APA", "Chicago", "MLA", "Harvard"
    pub citation_style: String,
    pub academic_tone: bool,
    /// Regional spelling examples, e.g. ["color", "organization"]
    pub regional_settings: Vec<String>,
}

/// Metadata about a style profile.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StyleProfileMetadata {
    pub id: String,
    pub name: String,
    pub description: String,
    pub source_type: String,
    pub source_count: usize,
    pub created_at: String,
    pub updated_at: String,
    pub tags: Vec<String>,
}

/// A complete style profile combining metadata, characteristics, and samples.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StyleProfile {
    pub metadata: StyleProfileMetadata,
    pub characteristics: StyleCharacteristics,
    pub sample_texts: Vec<String>,
}

/// Contextual information for output file naming.
#[derive(Clone, Debug)]
pub struct OutputContext {
    /// 0-based agent index.
    pub agent_index: usize,
    pub agent_key: String,
    pub output_file_path: Option<String>,
}

/// Result of validating style injection in a prompt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidationResult {
    pub valid: bool,
    pub has_english_marker: bool,
    pub has_spell_marker: bool,
}

// ---------------------------------------------------------------------------
// StyleProfileManager
// ---------------------------------------------------------------------------

/// JSON shape persisted to disk.
#[derive(Serialize, Deserialize)]
struct StorageFormat {
    profiles: HashMap<String, StyleProfile>,
    active_profile: Option<String>,
}

/// Manages a collection of writing-style profiles.
pub struct StyleProfileManager {
    profiles: HashMap<String, StyleProfile>,
    active_profile_id: Option<String>,
    storage_path: PathBuf,
}

impl StyleProfileManager {
    /// Create a new manager backed by the given storage path.
    pub fn new(storage_path: PathBuf) -> Self {
        Self {
            profiles: HashMap::new(),
            active_profile_id: None,
            storage_path,
        }
    }

    /// Insert a new profile. Errors if a profile with the same ID already exists.
    pub fn create_profile(&mut self, profile: StyleProfile) -> Result<()> {
        let id = profile.metadata.id.clone();
        if self.profiles.contains_key(&id) {
            bail!("Profile '{}' already exists", id);
        }
        self.profiles.insert(id, profile);
        Ok(())
    }

    /// Retrieve a profile by ID.
    pub fn get_profile(&self, id: &str) -> Option<&StyleProfile> {
        self.profiles.get(id)
    }

    /// List metadata for all stored profiles.
    pub fn list_profiles(&self) -> Vec<&StyleProfileMetadata> {
        self.profiles.values().map(|p| &p.metadata).collect()
    }

    /// Remove a profile. Clears active selection if the deleted profile was active.
    pub fn delete_profile(&mut self, id: &str) -> Result<()> {
        if self.profiles.remove(id).is_none() {
            bail!("Profile '{}' not found", id);
        }
        if self.active_profile_id.as_deref() == Some(id) {
            self.active_profile_id = None;
        }
        Ok(())
    }

    /// Mark a profile as active. Errors if the ID does not exist.
    pub fn set_active_profile(&mut self, id: &str) -> Result<()> {
        if !self.profiles.contains_key(id) {
            bail!("Profile '{}' not found", id);
        }
        self.active_profile_id = Some(id.to_string());
        Ok(())
    }

    /// Return the currently-active profile, if any.
    pub fn get_active_profile(&self) -> Option<&StyleProfile> {
        self.active_profile_id
            .as_deref()
            .and_then(|id| self.profiles.get(id))
    }

    /// Generate a style prompt from a specific or active profile.
    /// Returns `None` when no profile is available (caller should fall back to
    /// `DEFAULT_STYLE_PROMPT`).
    pub fn generate_style_prompt(&self, profile_id: Option<&str>) -> Option<String> {
        let profile = match profile_id {
            Some(id) => self.profiles.get(id)?,
            None => self.get_active_profile()?,
        };
        let chars = &profile.characteristics;
        let formality_cap = capitalize_first(&chars.formality_level);
        let examples = chars.regional_settings.join(", ");
        Some(format!(
            "Regional Language Settings:\n\
             - Use {} English spelling conventions\n\
             - Examples: {}\n\
             - Use {} citation style\n\
             - Formality: {}",
            chars.language_variant, examples, chars.citation_style, formality_cap,
        ))
    }

    /// Persist all profiles and active selection to JSON on disk.
    pub fn save(&self) -> Result<()> {
        let data = StorageFormat {
            profiles: self.profiles.clone(),
            active_profile: self.active_profile_id.clone(),
        };
        let json = serde_json::to_string_pretty(&data)?;
        if let Some(parent) = self.storage_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&self.storage_path, json)?;
        Ok(())
    }

    /// Load profiles from the JSON file on disk.
    pub fn load(&mut self) -> Result<()> {
        let contents = std::fs::read_to_string(&self.storage_path)?;
        let data: StorageFormat = serde_json::from_str(&contents)?;
        self.profiles = data.profiles;
        self.active_profile_id = data.active_profile;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// StyleInjector
// ---------------------------------------------------------------------------

/// Conditionally injects writing-style guidelines into agent prompts.
pub struct StyleInjector;

impl StyleInjector {
    pub fn new() -> Self {
        Self
    }

    /// Build an agent prompt with conditional style injection.
    ///
    /// Only Phase 6 agents receive style injection — all other phases get the
    /// prompt without style guidelines appended.
    pub fn build_agent_prompt(
        &self,
        base_prompt: &str,
        agent_phase: u8,
        style_prompt: Option<&str>,
        query: Option<&str>,
        output_context: Option<&OutputContext>,
    ) -> String {
        let mut prompt = String::new();

        // Prepend research query if provided.
        if let Some(q) = query {
            prompt.push_str(&format!("## RESEARCH QUERY\n\"{}\"\n\n", q));
        }

        // Prepend output requirements if provided.
        if let Some(ctx) = output_context {
            let display_index = ctx.agent_index + 1; // 1-based display
            let padded = format!("{:02}", display_index);
            prompt.push_str(&format!(
                "## OUTPUT REQUIREMENTS\nWrite output to: {}-{}.md\n\n",
                padded, ctx.agent_key
            ));
        }

        prompt.push_str(base_prompt);

        // Phase 6 only: inject style.
        if agent_phase == 6
            && let Some(style) = style_prompt {
                prompt = self.build_styled_prompt(&prompt, style);
            }

        prompt
    }

    /// Append style guidelines section to a prompt.
    pub fn build_styled_prompt(&self, base_prompt: &str, style_prompt: &str) -> String {
        format!("{}\n\n## STYLE GUIDELINES\n{}", base_prompt, style_prompt)
    }

    /// Validate that a prompt contains expected style markers.
    pub fn validate_style_injection(&self, prompt: &str) -> ValidationResult {
        let has_english_marker = prompt.contains("English");
        let has_spell_marker = prompt.to_lowercase().contains("spell");
        ValidationResult {
            valid: has_english_marker && has_spell_marker,
            has_english_marker,
            has_spell_marker,
        }
    }

    /// Check whether a prompt already contains style injection.
    pub fn has_style_injection(&self, prompt: &str) -> bool {
        prompt.contains("## STYLE GUIDELINES")
            || prompt.contains("Regional Language Settings")
            || prompt.contains("Sentence Structure:")
    }
}

impl Default for StyleInjector {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Built-in profiles
// ---------------------------------------------------------------------------

/// Create the built-in UK Academic style profile.
pub fn uk_academic_profile() -> StyleProfile {
    StyleProfile {
        metadata: StyleProfileMetadata {
            id: "uk-academic".to_string(),
            name: "UK Academic".to_string(),
            description: "British English academic writing style".to_string(),
            source_type: "built-in".to_string(),
            source_count: 0,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            tags: vec![
                "uk".to_string(),
                "academic".to_string(),
                "formal".to_string(),
            ],
        },
        characteristics: StyleCharacteristics {
            language_variant: "UK".to_string(),
            formality_level: "formal".to_string(),
            citation_style: "APA".to_string(),
            academic_tone: true,
            regional_settings: vec![
                "colour".to_string(),
                "organisation".to_string(),
                "analyse".to_string(),
                "centre".to_string(),
                "behaviour".to_string(),
            ],
        },
        sample_texts: vec![],
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().to_string() + chars.as_str(),
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // Helper: create a minimal test profile.
    fn make_profile(id: &str) -> StyleProfile {
        StyleProfile {
            metadata: StyleProfileMetadata {
                id: id.to_string(),
                name: format!("Test {}", id),
                description: "test profile".to_string(),
                source_type: "test".to_string(),
                source_count: 1,
                created_at: "2026-01-01T00:00:00Z".to_string(),
                updated_at: "2026-01-01T00:00:00Z".to_string(),
                tags: vec![],
            },
            characteristics: StyleCharacteristics {
                language_variant: "US".to_string(),
                formality_level: "formal".to_string(),
                citation_style: "APA".to_string(),
                academic_tone: true,
                regional_settings: vec!["color".to_string(), "organization".to_string()],
            },
            sample_texts: vec![],
        }
    }

    // 1. DEFAULT_STYLE_PROMPT contains "American English" and "APA"
    #[test]
    fn default_style_prompt_content() {
        assert!(
            DEFAULT_STYLE_PROMPT.contains("American English"),
            "should mention American English"
        );
        assert!(DEFAULT_STYLE_PROMPT.contains("APA"), "should mention APA");
    }

    // 2. Phase 6 agents get style injection
    #[test]
    fn phase_6_gets_style_injection() {
        let injector = StyleInjector::new();
        let result = injector.build_agent_prompt(
            "Write the introduction.",
            6,
            Some(DEFAULT_STYLE_PROMPT),
            None,
            None,
        );
        assert!(result.contains("## STYLE GUIDELINES"));
        assert!(result.contains("American English"));
    }

    // 3. Non-Phase 6 agents do NOT get style injection
    #[test]
    fn non_phase_6_no_style_injection() {
        let injector = StyleInjector::new();
        for phase in [1u8, 2, 3, 4, 5, 7] {
            let result = injector.build_agent_prompt(
                "Do research.",
                phase,
                Some(DEFAULT_STYLE_PROMPT),
                None,
                None,
            );
            assert!(
                !result.contains("## STYLE GUIDELINES"),
                "Phase {} should NOT have style injection",
                phase
            );
        }
    }

    // 4. Custom profile generates correct prompt
    #[test]
    fn custom_profile_generates_correct_prompt() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("styles.json");
        let mut mgr = StyleProfileManager::new(path);

        let uk = uk_academic_profile();
        mgr.create_profile(uk).unwrap();
        mgr.set_active_profile("uk-academic").unwrap();

        let prompt = mgr.generate_style_prompt(None).unwrap();
        assert!(prompt.contains("UK English"));
        assert!(prompt.contains("colour"));
        assert!(prompt.contains("organisation"));
        assert!(prompt.contains("APA"));
    }

    // 5. Default profile functional when no custom profile
    #[test]
    fn no_profile_returns_none() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("styles.json");
        let mgr = StyleProfileManager::new(path);
        assert!(mgr.generate_style_prompt(None).is_none());
    }

    // 6. validate_style_injection returns valid when both markers present
    #[test]
    fn validate_style_injection_valid() {
        let injector = StyleInjector::new();
        let prompt = "Use American English spelling conventions and spell check everything.";
        let result = injector.validate_style_injection(prompt);
        assert!(result.valid);
        assert!(result.has_english_marker);
        assert!(result.has_spell_marker);
    }

    // 6b. validate_style_injection returns invalid when markers missing
    #[test]
    fn validate_style_injection_invalid() {
        let injector = StyleInjector::new();
        let result = injector.validate_style_injection("Just a plain prompt.");
        assert!(!result.valid);
        assert!(!result.has_english_marker);
        assert!(!result.has_spell_marker);
    }

    // 7. has_style_injection detects known markers
    #[test]
    fn has_style_injection_detection() {
        let injector = StyleInjector::new();
        assert!(injector.has_style_injection("blah ## STYLE GUIDELINES blah"));
        assert!(injector.has_style_injection("Regional Language Settings:\n- stuff"));
        assert!(injector.has_style_injection("Sentence Structure: short"));
        assert!(!injector.has_style_injection("no markers here"));
    }

    // 8. Profile CRUD: create, get, list, delete
    #[test]
    fn profile_crud() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("styles.json");
        let mut mgr = StyleProfileManager::new(path);

        // Create
        mgr.create_profile(make_profile("a")).unwrap();
        mgr.create_profile(make_profile("b")).unwrap();

        // Duplicate fails
        assert!(mgr.create_profile(make_profile("a")).is_err());

        // Get
        assert!(mgr.get_profile("a").is_some());
        assert!(mgr.get_profile("z").is_none());

        // List
        assert_eq!(mgr.list_profiles().len(), 2);

        // Delete
        mgr.delete_profile("a").unwrap();
        assert!(mgr.get_profile("a").is_none());
        assert_eq!(mgr.list_profiles().len(), 1);

        // Delete non-existent fails
        assert!(mgr.delete_profile("a").is_err());
    }

    // 9. set_active_profile works
    #[test]
    fn set_active_profile() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("styles.json");
        let mut mgr = StyleProfileManager::new(path);

        mgr.create_profile(make_profile("x")).unwrap();
        assert!(mgr.get_active_profile().is_none());

        mgr.set_active_profile("x").unwrap();
        assert_eq!(mgr.get_active_profile().unwrap().metadata.id, "x");

        // Non-existent fails
        assert!(mgr.set_active_profile("nope").is_err());
    }

    // 10. generate_style_prompt returns None when no profiles
    #[test]
    fn generate_style_prompt_none_when_empty() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("styles.json");
        let mgr = StyleProfileManager::new(path);
        assert!(mgr.generate_style_prompt(None).is_none());
        assert!(mgr.generate_style_prompt(Some("nonexistent")).is_none());
    }

    // 11. Output requirements include padded index (0-based -> 1-based, zero-padded)
    #[test]
    fn output_requirements_padded_index() {
        let injector = StyleInjector::new();
        let ctx = OutputContext {
            agent_index: 6,
            agent_key: "intro-writer".to_string(),
            output_file_path: None,
        };
        let result = injector.build_agent_prompt("base", 6, None, None, Some(&ctx));
        assert!(
            result.contains("07-intro-writer.md"),
            "agent_index 6 should display as 07 (1-based, zero-padded). Got: {}",
            result
        );
    }

    // 12. Research query prepended when provided
    #[test]
    fn research_query_prepended() {
        let injector = StyleInjector::new();
        let result = injector.build_agent_prompt(
            "base prompt",
            3,
            None,
            Some("How does AI affect education?"),
            None,
        );
        assert!(result.starts_with("## RESEARCH QUERY"));
        assert!(result.contains("How does AI affect education?"));
        assert!(result.contains("base prompt"));
    }

    // 13. UK academic profile generates correct spelling examples
    #[test]
    fn uk_academic_profile_spelling() {
        let uk = uk_academic_profile();
        assert_eq!(uk.characteristics.language_variant, "UK");
        assert!(
            uk.characteristics
                .regional_settings
                .contains(&"colour".to_string())
        );
        assert!(
            uk.characteristics
                .regional_settings
                .contains(&"behaviour".to_string())
        );
        assert!(
            uk.characteristics
                .regional_settings
                .contains(&"centre".to_string())
        );
        assert!(
            uk.characteristics
                .regional_settings
                .contains(&"analyse".to_string())
        );
        assert!(
            uk.characteristics
                .regional_settings
                .contains(&"organisation".to_string())
        );
    }

    // 14. Save and load round-trip
    #[test]
    fn save_and_load_round_trip() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("styles.json");

        // Save
        let mut mgr = StyleProfileManager::new(path.clone());
        mgr.create_profile(uk_academic_profile()).unwrap();
        mgr.set_active_profile("uk-academic").unwrap();
        mgr.save().unwrap();

        // Load into fresh manager
        let mut mgr2 = StyleProfileManager::new(path);
        mgr2.load().unwrap();
        assert_eq!(mgr2.list_profiles().len(), 1);
        assert_eq!(
            mgr2.get_active_profile().unwrap().metadata.id,
            "uk-academic"
        );
    }

    // 15. Deleting active profile clears active selection
    #[test]
    fn delete_active_profile_clears_selection() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("styles.json");
        let mut mgr = StyleProfileManager::new(path);

        mgr.create_profile(make_profile("p")).unwrap();
        mgr.set_active_profile("p").unwrap();
        assert!(mgr.get_active_profile().is_some());

        mgr.delete_profile("p").unwrap();
        assert!(mgr.get_active_profile().is_none());
    }

    // 16. build_styled_prompt appends correctly
    #[test]
    fn build_styled_prompt_format() {
        let injector = StyleInjector::new();
        let result = injector.build_styled_prompt("Hello world", "Use formal tone");
        assert_eq!(
            result,
            "Hello world\n\n## STYLE GUIDELINES\nUse formal tone"
        );
    }
}
