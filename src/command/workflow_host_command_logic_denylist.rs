/// Files the closure of a capability reaches that no verdict depends on,
/// each with the reason. A file here can change without a logic-version
/// decision, so an entry is a reviewed line; the guard test fails on an
/// entry no capability reaches.
pub(crate) const LOGIC_DENYLIST: &[(&str, &str)] = &[
    (
        "crates/archon-llm/",
        "LLM provider transport and model catalogue. The catalogue resolves the critic's \
         model alias, so a remap changes which model judges task-set-lint: that is a \
         model choice, not logic. It re-runs no recorded verdict; a lint that runs anyway \
         asks the critic again, as its findings store keys on the resolved model. The \
         request and its reply reader are hashed",
    ),
    (
        "src/command/pipeline_workflow_llm.rs",
        "adapter from the workflow LLM port to the pipeline runner: transport only",
    ),
    (
        "crates/archon-tools/",
        "the tools agents call at run time; the lint reads tool names and permissions, \
         never a tool's behaviour",
    ),
    (
        "crates/archon-mcp/",
        "MCP server transport and lifecycle; the permitted tool list is read by hashed code",
    ),
    (
        "crates/archon-memory/",
        "memory store client, reached only through agent configuration",
    ),
    (
        "crates/archon-core/src/agent/",
        "interactive agent loop, reached through configuration types",
    ),
    (
        "crates/archon-core/src/agents/",
        "agent definition registry, reached through configuration types",
    ),
    (
        "crates/archon-core/src/subagent/",
        "subagent runner and retention, reached through configuration types",
    ),
    (
        "crates/archon-core/src/sandbox/",
        "container and SSH sandbox backends the configuration names",
    ),
    (
        "crates/archon-core/src/orchestrator/",
        "agent pool, reached through configuration types",
    ),
    (
        "src/command/requirement_trace/slash.rs",
        "the interactive /requirements front end; the host command runs the CLI path",
    ),
];
