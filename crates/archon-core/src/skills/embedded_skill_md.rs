//! Embedded SKILL.md content for built-in engineering skills (Phase 2).
//!
//! Each constant is the result of `include_str!()` against the
//! corresponding file in `assets/skills/<name>/SKILL.md`. Embedded
//! here so the binary works without the repo present at runtime.
//!
//! Override resolution (highest priority first), via Phase 1 loader
//! + the `resolve_skill_body` helper added below in `templates.rs`:
//!   1. <workdir>/.archon/skills/<name>.md         (flat-file project)
//!   2. <workdir>/.archon/skills/<name>/SKILL.md   (subdir project)
//!   3. ~/.config/archon/skills/<name>.md          (flat-file user)
//!   4. ~/.config/archon/skills/<name>/SKILL.md    (subdir user)
//!   5. embedded fallback (this module)

pub const GRILL_ME: &str = include_str!("../../../../assets/skills/grill-me/SKILL.md");
pub const GRILL_WITH_DOCS: &str =
    include_str!("../../../../assets/skills/grill-with-docs/SKILL.md");
pub const DIAGNOSE: &str = include_str!("../../../../assets/skills/diagnose/SKILL.md");
pub const TDD: &str = include_str!("../../../../assets/skills/tdd/SKILL.md");
pub const ZOOM_OUT: &str = include_str!("../../../../assets/skills/zoom-out/SKILL.md");

// Phase 3 archon-specific skills
pub const SPEC_TO_TASKS: &str = include_str!("../../../../assets/skills/spec-to-tasks/SKILL.md");
pub const COMPOSE_PIPELINE: &str =
    include_str!("../../../../assets/skills/compose-pipeline/SKILL.md");
pub const CI_GATE_WALKER: &str = include_str!("../../../../assets/skills/ci-gate-walker/SKILL.md");
pub const SETUP_ARCHON_SKILLS: &str =
    include_str!("../../../../assets/skills/setup-archon-skills/SKILL.md");
pub const WRITE_A_SKILL: &str = include_str!("../../../../assets/skills/write-a-skill/SKILL.md");
