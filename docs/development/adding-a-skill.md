# Adding a skill

Built-in skills are Rust-implemented composable command sequences. User-authored skills are SKILL.md or TOML files (covered in [Skills reference](../reference/skills.md)). This page covers built-in skill development.

## Where built-in skills live

- `crates/archon-core/src/skills/builtin.rs` — 21 core skills + `register_builtins()` assembly
- `crates/archon-core/src/skills/expanded.rs` — 34 expanded skills
- `crates/archon-core/src/skills/engineering_pack.rs` — 5 Phase 2 skills via `engineering_skill!` macro
- `crates/archon-core/src/skills/archon_pack.rs` — 5 Phase 3 skills via `archon_skill!` macro
- `crates/archon-core/src/skills/embedded_skill_md.rs` — `include_str!()` constants for embedded SKILL.md bodies

## Two approaches

### Approach A: Embedded prompt-template skill (recommended for new skills)

If the skill is fundamentally a prompt template, write a SKILL.md in `assets/skills/<name>/SKILL.md`, embed it, and use the macro:

**Step 1:** Create `assets/skills/my-skill/SKILL.md` with frontmatter and process body.

**Step 2:** Add `include_str!()` constant in `embedded_skill_md.rs`:
```rust
pub const MY_SKILL: &str = include_str!("../../../../assets/skills/my-skill/SKILL.md");
```

**Step 3:** Use the macro in `archon_pack.rs` (or `engineering_pack.rs`):
```rust
archon_skill!(MySkill, MY_SKILL);
```

**Step 4:** Register in `builtin.rs::register_builtins()`:
```rust
registry.register(Box::new(super::archon_pack::MySkill));
```

The macro generates the full `Skill` trait impl including override resolution. Users can replace the body without recompiling.

### Approach B: Manual Skill trait impl

For skills that need Rust logic beyond a prompt template.

**Step 1:** Implement the Skill trait:
```rust
use crate::skills::{Skill, SkillContext, SkillOutput};

pub struct MySkill;

impl Skill for MySkill {
    fn name(&self) -> &str { "my-skill" }
    fn description(&self) -> &str { "Does something useful." }
    fn execute(&self, args: &[String], ctx: &SkillContext) -> SkillOutput {
        // Custom logic: return Text, Markdown, Prompt, or Error
        SkillOutput::Text("result".to_string())
    }
}
```

**Step 2:** Register in `builtin.rs::register_builtins()`.

## Tests

Inline tests in the same file (metadata + smoke):

```rust
#[test]
fn my_skill_metadata() {
    assert_eq!(MySkill.name(), "my-skill");
    assert!(!MySkill.description().is_empty());
}
```

Integration tests in `crates/archon-core/tests/`:
- Registry lookup
- Prompt emission
- Override precedence (flat-file, subdir, embedded fallback)

## CI gate verification

```bash
cargo check -p archon-core -j1
cargo test -p archon-core -j1 -- --test-threads=2
cargo fmt --all -- --check
./scripts/ci-gate.sh
```

## See also

- [Skills reference](../reference/skills.md) — user-authored SKILL.md and TOML skills
- [Slash commands reference](../reference/slash-commands.md) — primary commands
- [Dev flow gates](dev-flow-gates.md)
