//! The landing-policy section: configured numbers, the enforcer's own
//! counting rules, and the per-target headroom restated from the stamp.

use serde_json::json;

use super::preamble;
use crate::write_coordinator::WriteCoordinatorConfig;

/// Numbers no default shares, so a hardcoded prompt would fail here.
fn unusual() -> WriteCoordinatorConfig {
    WriteCoordinatorConfig {
        max_source_file_lines: 321,
        max_function_complexity: 7,
        max_file_bytes: 4_096,
        max_patch_bytes: 65_536,
        ..WriteCoordinatorConfig::default()
    }
}

/// Every cap is stated with the configured number, and the consequence is
/// the enforcer's: one violation refuses the whole patch.
#[test]
fn the_preamble_states_every_configured_cap_and_the_consequence() {
    let text = preamble(&unusual(), &json!({}));
    assert!(text.starts_with("\nLanding policy ("), "{text}");
    assert!(
        text.contains("a single violation refuses the ENTIRE patch"),
        "{text}"
    );
    assert!(
        text.contains("split files into submodules and functions into helpers BEFORE returning"),
        "{text}"
    );
    assert!(
        text.contains("- Source file length: at most 321 lines per changed file"),
        "{text}"
    );
    assert!(
        text.contains("- Function complexity: at most 7 per function"),
        "{text}"
    );
    assert!(
        text.contains("already over the cap may be changed only if its score does not grow"),
        "{text}"
    );
    assert!(
        text.contains("- File size: at most 4096 bytes per changed file"),
        "{text}"
    );
    assert!(
        text.contains("- Patch size: at most 65536 bytes for the whole patch"),
        "{text}"
    );
    assert!(!text.contains("500") && !text.contains("15 per"), "{text}");
}

/// The counting rules are the enforcer's: every line of the file counts,
/// only the checked extensions apply, and the complexity score is 1 plus
/// the branch tokens and logical operators `code_hygiene` tallies.
#[test]
fn the_preamble_names_the_metrics_as_the_enforcer_computes_them() {
    let text = preamble(&WriteCoordinatorConfig::default(), &json!({}));
    assert!(
        text.contains(
            "counted as every line of the file after your edit — blank lines, comments, doc \
             comments and in-file test modules all count"
        ),
        "{text}"
    );
    assert!(
        text.contains(
            "extensions: c, cc, cpp, cs, go, h, hpp, java, js, jsx, kt, kts, mjs, py, pyi, rs, \
             sh, swift, ts, tsx, vue."
        ),
        "{text}"
    );
    assert!(
        text.contains("A file already over the cap may be changed only if it does not grow"),
        "{text}"
    );
    assert!(
        text.contains(
            "scored as 1 plus one for every `if`, `for`, `while`, `match`, `case`, `catch`, \
             `elif` or `except` token and one for every `&&` or `||` operator"
        ),
        "{text}"
    );
    assert!(
        text.contains("comment text after `//` or `#` excluded"),
        "{text}"
    );
}

/// The default numbers, verbatim: what a live coder reads today.
#[test]
fn the_default_config_renders_the_live_caps() {
    let text = preamble(&WriteCoordinatorConfig::default(), &json!({}));
    assert!(
        text.contains("at most 500 lines per changed file"),
        "{text}"
    );
    assert!(text.contains("at most 15 per function"), "{text}");
    assert!(
        text.contains("at most 1048576 bytes per changed file"),
        "{text}"
    );
    assert!(
        text.contains("at most 10485760 bytes for the whole patch"),
        "{text}"
    );
    assert!(!text.contains("Declared target headroom"), "{text}");
}

/// A cap of zero disables the check in the enforcer, so the prompt says no
/// cap rather than "at most 0".
#[test]
fn a_zero_cap_is_stated_as_unconfigured() {
    let cfg = WriteCoordinatorConfig {
        max_source_file_lines: 0,
        max_function_complexity: 0,
        ..WriteCoordinatorConfig::default()
    };
    let text = preamble(&cfg, &json!({}));
    assert!(
        text.contains("- Source file length: no cap is configured."),
        "{text}"
    );
    assert!(
        text.contains("- Function complexity: no cap is configured."),
        "{text}"
    );
    assert!(!text.contains("at most 0"), "{text}");
}

/// Each declared target the stamp measured is restated as "N of cap lines
/// used" with the remainder, and a full file is told it may not grow. The
/// stamp's numbers are used as written — budget semantics stay in
/// `target_budgets`.
#[test]
fn the_headroom_line_restates_each_stamped_budget() {
    let item = json!({
        "target_files": ["src/big.rs", "src/new.rs", "src/full.rs"],
        "target_file_budgets": [
            {"path": "src/big.rs", "current_lines": 495, "max_lines": 500, "lines_remaining": 5},
            {"path": "src/new.rs", "current_lines": 0, "max_lines": 500, "lines_remaining": 500},
            {"path": "src/full.rs", "current_lines": 512, "max_lines": 500, "lines_remaining": 0},
        ],
    });
    let text = preamble(&WriteCoordinatorConfig::default(), &item);
    assert!(
        text.contains(
            "Declared target headroom: src/big.rs: 495 of 500 lines used (5 remaining); \
             src/new.rs: 0 of 500 lines used (500 remaining); src/full.rs: 512 of 500 lines \
             used (at or over the cap; it may not grow).\n"
        ),
        "{text}"
    );
    assert!(text.ends_with(".\n"), "{text}");
}

/// A malformed or empty stamp yields no headroom line rather than a panic
/// or a half sentence.
#[test]
fn a_missing_or_malformed_stamp_omits_the_headroom_line() {
    for item in [
        json!({}),
        json!({"target_file_budgets": []}),
        json!({"target_file_budgets": "not a list"}),
        json!({"target_file_budgets": [{"path": "src/a.rs"}]}),
    ] {
        let text = preamble(&WriteCoordinatorConfig::default(), &item);
        assert!(!text.contains("headroom"), "{item} -> {text}");
    }
}
