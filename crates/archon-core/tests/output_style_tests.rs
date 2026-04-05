//! Tests for TASK-CLI-310: Custom Output Styles
//!
//! Tests cover OutputStyleConfig, OutputStyleSource, OutputStyleRegistry,
//! built-in styles, file-based user styles, and injection logic.

use archon_core::output_style::{OutputStyleConfig, OutputStyleRegistry, OutputStyleSource};
use archon_core::output_style_loader::load_styles_from_dir;
use std::fs;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// OutputStyleSource
// ---------------------------------------------------------------------------

#[test]
fn source_variants_exist() {
    let _b = OutputStyleSource::BuiltIn;
    let _c = OutputStyleSource::Config;
    let _p = OutputStyleSource::Plugin;
}

#[test]
fn source_clone_and_debug() {
    let s = OutputStyleSource::BuiltIn;
    let _ = format!("{s:?}");
    let _ = s.clone();
}

// ---------------------------------------------------------------------------
// OutputStyleConfig structure
// ---------------------------------------------------------------------------

#[test]
fn config_fields_exist() {
    let c = OutputStyleConfig {
        name: "test".into(),
        description: "desc".into(),
        prompt: None,
        source: OutputStyleSource::BuiltIn,
        keep_coding_instructions: None,
        force_for_plugin: None,
    };
    assert_eq!(c.name, "test");
    assert!(c.prompt.is_none());
}

#[test]
fn config_with_prompt() {
    let c = OutputStyleConfig {
        name: "Explanatory".into(),
        description: "Adds educational insights".into(),
        prompt: Some("You are an educational assistant.".into()),
        source: OutputStyleSource::BuiltIn,
        keep_coding_instructions: Some(true),
        force_for_plugin: None,
    };
    assert!(c.prompt.is_some());
    assert_eq!(c.keep_coding_instructions, Some(true));
}

#[test]
fn config_clone_and_debug() {
    let c = OutputStyleConfig {
        name: "test".into(),
        description: "desc".into(),
        prompt: Some("inject me".into()),
        source: OutputStyleSource::Config,
        keep_coding_instructions: None,
        force_for_plugin: Some(false),
    };
    let _ = format!("{c:?}");
    let c2 = c.clone();
    assert_eq!(c2.name, "test");
}

// NO fg/bg/bold/italic fields — verify the struct has exactly the right shape
#[test]
fn config_has_no_visual_rendering_fields() {
    // This test compiles only if the struct matches the spec.
    // If fg/bg/bold/italic were added, this would need updating.
    let _c = OutputStyleConfig {
        name: String::new(),
        description: String::new(),
        prompt: None,
        source: OutputStyleSource::BuiltIn,
        keep_coding_instructions: None,
        force_for_plugin: None,
    };
    // If we got here, struct has exactly these fields (compiler enforces it
    // only when using struct literal syntax without `..` spread).
}

// ---------------------------------------------------------------------------
// OutputStyleRegistry — built-in styles
// ---------------------------------------------------------------------------

#[test]
fn registry_new_has_builtin_styles() {
    let reg = OutputStyleRegistry::new();
    // Must include all 5 built-ins
    assert!(reg.get("default").is_some());
    assert!(reg.get("Explanatory").is_some());
    assert!(reg.get("Learning").is_some());
    assert!(reg.get("Formal").is_some());
    assert!(reg.get("Concise").is_some());
}

#[test]
fn default_style_has_no_prompt() {
    let reg = OutputStyleRegistry::new();
    let style = reg.get("default").expect("default must exist");
    assert!(
        style.prompt.is_none(),
        "default style must have no prompt injection"
    );
}

#[test]
fn explanatory_style_has_prompt() {
    let reg = OutputStyleRegistry::new();
    let style = reg.get("Explanatory").expect("Explanatory must exist");
    assert!(style.prompt.is_some(), "Explanatory must have a prompt");
    assert!(!style.prompt.as_ref().unwrap().is_empty());
}

#[test]
fn learning_style_has_prompt() {
    let reg = OutputStyleRegistry::new();
    let style = reg.get("Learning").expect("Learning must exist");
    assert!(style.prompt.is_some(), "Learning must have a prompt");
}

#[test]
fn formal_style_has_prompt() {
    let reg = OutputStyleRegistry::new();
    let style = reg.get("Formal").expect("Formal must exist");
    assert!(style.prompt.is_some(), "Formal must have a prompt");
}

#[test]
fn concise_style_has_prompt() {
    let reg = OutputStyleRegistry::new();
    let style = reg.get("Concise").expect("Concise must exist");
    assert!(style.prompt.is_some(), "Concise must have a prompt");
}

#[test]
fn builtin_styles_have_builtin_source() {
    let reg = OutputStyleRegistry::new();
    for name in &["default", "Explanatory", "Learning", "Formal", "Concise"] {
        let style = reg.get(name).unwrap();
        assert!(
            matches!(style.source, OutputStyleSource::BuiltIn),
            "{name} should have BuiltIn source"
        );
    }
}

#[test]
fn unknown_style_returns_none() {
    let reg = OutputStyleRegistry::new();
    assert!(reg.get("NonExistentStyle").is_none());
}

#[test]
fn list_styles_includes_builtins() {
    let reg = OutputStyleRegistry::new();
    let names = reg.list();
    assert!(names.contains(&"default".to_string()));
    assert!(names.contains(&"Explanatory".to_string()));
    assert!(names.contains(&"Learning".to_string()));
    assert!(names.contains(&"Formal".to_string()));
    assert!(names.contains(&"Concise".to_string()));
}

// ---------------------------------------------------------------------------
// OutputStyleRegistry — register and get
// ---------------------------------------------------------------------------

#[test]
fn register_custom_style_retrievable() {
    let mut reg = OutputStyleRegistry::new();
    let style = OutputStyleConfig {
        name: "MyCustom".into(),
        description: "Custom test style".into(),
        prompt: Some("Be extra helpful.".into()),
        source: OutputStyleSource::Config,
        keep_coding_instructions: None,
        force_for_plugin: None,
    };
    reg.register(style);
    let got = reg.get("MyCustom").expect("should find registered style");
    assert_eq!(got.description, "Custom test style");
}

#[test]
fn register_overwrites_existing() {
    let mut reg = OutputStyleRegistry::new();
    let s1 = OutputStyleConfig {
        name: "Dup".into(),
        description: "first".into(),
        prompt: Some("v1".into()),
        source: OutputStyleSource::Config,
        keep_coding_instructions: None,
        force_for_plugin: None,
    };
    let s2 = OutputStyleConfig {
        name: "Dup".into(),
        description: "second".into(),
        prompt: Some("v2".into()),
        source: OutputStyleSource::Config,
        keep_coding_instructions: None,
        force_for_plugin: None,
    };
    reg.register(s1);
    reg.register(s2);
    let got = reg.get("Dup").unwrap();
    assert_eq!(got.description, "second");
}

#[test]
fn get_or_default_returns_default_for_unknown() {
    let reg = OutputStyleRegistry::new();
    let style = reg.get_or_default("NoSuchStyle");
    assert_eq!(style.name, "default");
    assert!(style.prompt.is_none());
}

#[test]
fn get_or_default_returns_style_when_found() {
    let reg = OutputStyleRegistry::new();
    let style = reg.get_or_default("Concise");
    assert_eq!(style.name, "Concise");
}

// ---------------------------------------------------------------------------
// System prompt injection
// ---------------------------------------------------------------------------

#[test]
fn inject_no_op_for_default_style() {
    let reg = OutputStyleRegistry::new();
    let style = reg.get("default").unwrap();
    let base = "Base system prompt.".to_string();
    let result = style.inject_into(&base);
    // default style must not change the prompt
    assert_eq!(result, base);
}

#[test]
fn inject_appends_prompt_for_explanatory() {
    let reg = OutputStyleRegistry::new();
    let style = reg.get("Explanatory").unwrap();
    let base = "Base prompt.";
    let result = style.inject_into(base);
    assert!(result.starts_with("Base prompt."));
    assert!(result.len() > base.len(), "prompt should be appended");
    // The style's prompt content should be in the result
    let inject = style.prompt.as_ref().unwrap();
    assert!(result.contains(inject.as_str()));
}

#[test]
fn inject_appends_for_all_non_default_builtins() {
    let reg = OutputStyleRegistry::new();
    let base = "Sys prompt.";
    for name in &["Explanatory", "Learning", "Formal", "Concise"] {
        let style = reg.get(name).unwrap();
        let result = style.inject_into(base);
        assert!(
            result.len() > base.len(),
            "{name} should extend the system prompt"
        );
    }
}

#[test]
fn inject_custom_style_prompt() {
    let style = OutputStyleConfig {
        name: "Custom".into(),
        description: "d".into(),
        prompt: Some("Respond only in bullet points.".into()),
        source: OutputStyleSource::Config,
        keep_coding_instructions: None,
        force_for_plugin: None,
    };
    let result = style.inject_into("Base.");
    assert!(result.contains("Respond only in bullet points."));
}

// ---------------------------------------------------------------------------
// File-based style loading (output_style_loader)
// ---------------------------------------------------------------------------

#[test]
fn load_from_dir_empty_dir_returns_empty() {
    let tmp = TempDir::new().unwrap();
    let styles = load_styles_from_dir(tmp.path());
    assert!(styles.is_empty());
}

#[test]
fn load_from_dir_nonexistent_path_returns_empty() {
    let styles = load_styles_from_dir(std::path::Path::new("/nonexistent/path/xyz"));
    assert!(styles.is_empty());
}

#[test]
fn load_from_dir_reads_md_file() {
    let tmp = TempDir::new().unwrap();
    let content = "# Pirate\nDescription: Talk like a pirate\nYo ho ho, matey!\n";
    fs::write(tmp.path().join("pirate.md"), content).unwrap();

    let styles = load_styles_from_dir(tmp.path());
    assert_eq!(styles.len(), 1);
    let s = &styles[0];
    assert_eq!(s.name, "Pirate");
    assert_eq!(s.description, "Talk like a pirate");
    assert!(s.prompt.as_ref().unwrap().contains("Yo ho ho"));
    assert!(matches!(s.source, OutputStyleSource::Config));
}

#[test]
fn load_from_dir_ignores_non_md_files() {
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join("notes.txt"), "not a style").unwrap();
    fs::write(tmp.path().join("style.md"), "# S\nDescription: d\nbody").unwrap();

    let styles = load_styles_from_dir(tmp.path());
    assert_eq!(styles.len(), 1);
}

#[test]
fn load_from_dir_multiple_files() {
    let tmp = TempDir::new().unwrap();
    for (fname, name, desc, body) in &[
        ("a.md", "StyleA", "Desc A", "Body A"),
        ("b.md", "StyleB", "Desc B", "Body B"),
    ] {
        let content = format!("# {name}\nDescription: {desc}\n{body}");
        fs::write(tmp.path().join(fname), content).unwrap();
    }

    let styles = load_styles_from_dir(tmp.path());
    assert_eq!(styles.len(), 2);
}

#[test]
fn load_from_dir_missing_description_line_handled() {
    // A file that only has a name but no Description: line — should load with empty description
    let tmp = TempDir::new().unwrap();
    let content = "# NameOnly\nJust a body with no description line.\n";
    fs::write(tmp.path().join("nameonly.md"), content).unwrap();

    let styles = load_styles_from_dir(tmp.path());
    assert_eq!(styles.len(), 1);
    // should not panic; name parsed, description may be empty
    assert_eq!(styles[0].name, "NameOnly");
}

#[test]
fn load_from_dir_empty_body_gives_none_prompt() {
    let tmp = TempDir::new().unwrap();
    // File with name + description but no body
    let content = "# Empty\nDescription: nothing here\n";
    fs::write(tmp.path().join("empty.md"), content).unwrap();

    let styles = load_styles_from_dir(tmp.path());
    assert_eq!(styles.len(), 1);
    // prompt should be None or empty string — either is acceptable
    let p = &styles[0].prompt;
    assert!(p.is_none() || p.as_deref().unwrap_or("").trim().is_empty());
}

#[test]
fn load_from_dir_registers_into_registry() {
    let tmp = TempDir::new().unwrap();
    let content = "# UserStyle\nDescription: My personal style\nAlways be concise.\n";
    fs::write(tmp.path().join("user.md"), content).unwrap();

    let mut reg = OutputStyleRegistry::new();
    let styles = load_styles_from_dir(tmp.path());
    for style in styles {
        reg.register(style);
    }

    let got = reg.get("UserStyle").expect("should be registered");
    assert!(got.prompt.as_ref().unwrap().contains("Always be concise."));
}

// ---------------------------------------------------------------------------
// Plugin-source style
// ---------------------------------------------------------------------------

#[test]
fn plugin_style_has_plugin_source() {
    let style = OutputStyleConfig {
        name: "PluginStyle".into(),
        description: "from plugin".into(),
        prompt: Some("Plugin prompt.".into()),
        source: OutputStyleSource::Plugin,
        keep_coding_instructions: None,
        force_for_plugin: Some(true),
    };
    assert!(matches!(style.source, OutputStyleSource::Plugin));
    assert_eq!(style.force_for_plugin, Some(true));
}

#[test]
fn registry_can_hold_plugin_styles() {
    let mut reg = OutputStyleRegistry::new();
    reg.register(OutputStyleConfig {
        name: "PluginX".into(),
        description: "plugin x".into(),
        prompt: Some("do X".into()),
        source: OutputStyleSource::Plugin,
        keep_coding_instructions: None,
        force_for_plugin: Some(false),
    });
    assert!(reg.get("PluginX").is_some());
}

#[test]
fn forced_plugin_styles_findable() {
    let mut reg = OutputStyleRegistry::new();
    reg.register(OutputStyleConfig {
        name: "AutoApply".into(),
        description: "auto".into(),
        prompt: Some("apply".into()),
        source: OutputStyleSource::Plugin,
        keep_coding_instructions: None,
        force_for_plugin: Some(true),
    });
    reg.register(OutputStyleConfig {
        name: "NotForced".into(),
        description: "not forced".into(),
        prompt: None,
        source: OutputStyleSource::Plugin,
        keep_coding_instructions: None,
        force_for_plugin: Some(false),
    });

    let forced = reg.forced_plugin_style();
    assert!(forced.is_some());
    assert_eq!(forced.unwrap().name, "AutoApply");
}

#[test]
fn no_forced_style_returns_none() {
    let reg = OutputStyleRegistry::new();
    assert!(reg.forced_plugin_style().is_none());
}
