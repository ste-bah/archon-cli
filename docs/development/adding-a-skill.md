# Adding a skill

Built-in skills are Rust-implemented composable command sequences. User-authored skills are TOML files (covered in [Skills reference](../reference/skills.md)). This page covers built-in skill development.

## Where built-in skills live

- `crates/archon-core/src/skills/builtin.rs` — 21 core skills
- `crates/archon-core/src/skills/expanded.rs` — 34 expanded skills

The split is historical; new skills can land in either file. Convention: lighter-weight session/git skills in `builtin.rs`, project-management and analysis skills in `expanded.rs`.

## Step 1: implement the Skill trait

```rust
// In crates/archon-core/src/skills/expanded.rs (or builtin.rs)

use crate::skills::{Skill, SkillContext, SkillResult};
use async_trait::async_trait;

pub struct MySkill;

#[async_trait]
impl Skill for MySkill {
    fn name(&self) -> &'static str { "my-skill" }

    fn description(&self) -> &'static str {
        "Does something useful. Invoke with /my-skill"
    }

    fn trigger(&self) -> &'static str { "/my-skill" }

    fn aliases(&self) -> &'static [&'static str] {
        &["/ms"]   // optional
    }

    async fn execute(&self, args: &str, ctx: SkillContext) -> SkillResult {
        // Implementation: typically constructs a prompt and submits it
        let prompt = format!("Run my-skill workflow with args: {}", args);
        ctx.submit_prompt(prompt).await
    }
}
```

## Step 2: register in the registry assembly

In `crates/archon-core/src/skills/expanded.rs`:

```rust
pub fn register_expanded_skills(registry: &mut SkillRegistry) {
    // ... existing skills
    registry.add(Box::new(MySkill));
}
```

The skill registry is assembled in `crates/archon-core/src/skills/mod.rs::default_registry`.

## Step 3: tests

In the same file:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn my_skill_metadata() {
        let skill = MySkill;
        assert_eq!(skill.name(), "my-skill");
        assert_eq!(skill.trigger(), "/my-skill");
    }

    #[tokio::test]
    async fn my_skill_executes() {
        let skill = MySkill;
        let ctx = SkillContext::test();
        let result = skill.execute("test args", ctx).await;
        assert!(result.is_ok());
    }
}
```

Plus a registry-level test in `crates/archon-core/src/skills/mod.rs`:

```rust
#[test]
fn default_registry_includes_my_skill() {
    let registry = default_registry();
    assert!(registry.has("/my-skill"), "missing /my-skill");
}
```

## Step 4: documentation

Update [docs/reference/skills.md](../reference/skills.md) — add to the highlights table if the skill is widely useful, or note it as part of the 55-total count.

If the skill name happens to conflict with a primary command, document the precedence (primary wins; skill is fallback).

## Step 5: dev flow gates

```bash
scripts/dev-flow-pass-gate.sh TASK-ID tests-written-first      "tests at expanded.rs:540-560"
scripts/dev-flow-pass-gate.sh TASK-ID implementation-complete  "cargo check -p archon-core -j1: PASS"
scripts/dev-flow-pass-gate.sh TASK-ID sherlock-code-review     "Sherlock APPROVED"
scripts/dev-flow-pass-gate.sh TASK-ID tests-passing            "cargo test -p archon-core skills: 5 passed (including default_registry_includes_my_skill)"
scripts/dev-flow-pass-gate.sh TASK-ID live-smoke-test          "TUI: typed /my-skill, observed expected behavior"
scripts/dev-flow-pass-gate.sh TASK-ID sherlock-final-review    "Sherlock APPROVED — registered in default_registry, autocomplete picks it up"
```

## Skill vs primary command

Pick a primary command (in `src/command/registry.rs`) instead of a skill if:
- The command needs Rust state (other than the prompt machinery)
- The command interacts with the TUI directly (e.g., overlay panels)
- The command must run synchronously (skills typically construct prompts and submit)

Pick a skill if:
- The "command" is fundamentally a prompt template
- The behavior is composable from existing tools
- You want users to be able to override / extend via TOML in `.archon/skills/`

## See also

- [Skills reference](../reference/skills.md) — TOML user skills
- [Slash commands reference](../reference/slash-commands.md) — primary commands
- [Dev flow gates](dev-flow-gates.md)
