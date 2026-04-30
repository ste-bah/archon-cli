# Adding a tool

Tools are callable by the LLM during agent turns. Adding one requires implementing the `Tool` trait, registering it in the default registry, and updating docs.

## Step 1: implement the trait

```rust
// crates/archon-tools/src/my_tool.rs
use archon_tools::tool::*;
use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct MyToolArgs {
    pub input: String,
}

#[derive(Debug, Serialize)]
pub struct MyToolOutput {
    pub result: String,
}

pub struct MyTool;

#[async_trait]
impl Tool for MyTool {
    fn name(&self) -> &'static str { "MyTool" }

    fn description(&self) -> &'static str {
        "Does something specific. Use when ..."
    }

    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "input": { "type": "string", "description": "..." }
            },
            "required": ["input"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Safe   // Safe | Risky | Variable
    }

    async fn execute(&self, args: serde_json::Value, ctx: ToolContext) -> Result<serde_json::Value> {
        let args: MyToolArgs = serde_json::from_value(args)?;
        // Implementation here
        let output = MyToolOutput { result: format!("processed: {}", args.input) };
        Ok(serde_json::to_value(output)?)
    }
}
```

## Step 2: add to the workspace

In `crates/archon-tools/src/lib.rs`:
```rust
pub mod my_tool;
```

## Step 3: register in default registry

In `crates/archon-core/src/dispatch.rs::create_default_registry`:

```rust
registry.register(Box::new(archon_tools::my_tool::MyTool));
```

The registration order doesn't affect behavior but the convention is to keep related tools together (file ops, shell ops, etc.).

## Step 4: tests

Two test layers:

### Unit tests (in the tool's module)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn my_tool_processes_input() {
        let tool = MyTool;
        let args = serde_json::json!({ "input": "hello" });
        let result = tool.execute(args, ToolContext::default()).await.unwrap();
        assert_eq!(result["result"], "processed: hello");
    }
}
```

### Registry test (`crates/archon-core/src/dispatch.rs`)

The `default_registry_has_all_tools` test enumerates expected tool names. Add yours:

```rust
assert!(names.contains(&"MyTool"), "missing MyTool");
```

## Step 5: permissions

If your tool is `Variable`, add it to the per-command classifier in `crates/archon-permissions/src/classifier.rs`. Pattern:

```rust
match tool_name {
    "MyTool" => classify_my_tool(args),
    _ => /* existing match */
}
```

## Step 6: documentation

Update [docs/reference/tools.md](../reference/tools.md) — add the tool to the appropriate category table. Include name, permission level, and one-line purpose.

If the tool is risky / variable, update the permission examples in [docs/reference/permissions.md](../reference/permissions.md) if relevant.

## Step 7: CI gate verification

Run locally before pushing:

```bash
cargo check -p archon-tools -j1
cargo test -p archon-tools my_tool -- --test-threads=2
cargo fmt --all -- --check
cargo clippy --workspace -- -D warnings
./scripts/ci-gate.sh                  # full CI gate
```

Plus a live smoke test in the TUI: invoke your tool from an agent context and confirm it behaves end-to-end (don't trust unit tests alone — see `docs/development/dev-flow-gates.md` for the cold-read audit pattern).

## Common patterns

- **Streaming output:** wrap output in `ToolStream` and emit chunks. Useful for long-running tools (Bash, Monitor).
- **Async I/O:** use `tokio::fs`, `reqwest`, etc. Avoid blocking I/O in the async path.
- **Cancellation:** check `ctx.cancellation_token` for long operations; honour it to allow `/cancel`.
- **Error context:** use `anyhow::Context` to thread error origin info.

## See also

- [Tools reference](../reference/tools.md)
- [Permissions](../reference/permissions.md)
- [Dev flow gates](dev-flow-gates.md)
- Existing tools at `crates/archon-tools/src/*.rs` for reference implementations
