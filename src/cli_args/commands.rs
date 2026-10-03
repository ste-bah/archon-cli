use clap::Subcommand;

use super::{
    AgentAction, AuthArgs, BehaviourAction, BriefingAction, ChatArgs, CognitiveAction,
    CompletionAction, ConstellationAction, DocsAction, GametheoryAction, KbAction, LearningAction,
    MeaningAction, MemoryAction, PermissionsAction, PipelineAction, PluginAction, ProvAction,
    ProvidersAction, ReasoningAction, RemoteAction, RequirementsAction, SandboxAction, SelfAction,
    StyleAction, TeamAction, TradingCliAction, VideoAction, WorkflowAction, WorldAction,
};

/// `Trading` carries by far the largest payload, so `clippy::large_enum_variant`
/// fires at 440 bytes. Allowed rather than boxed: this enum is built once by
/// `clap` at process start, moved once into the dispatcher, and dropped. There
/// is no hot path and no collection of them, so the 440 bytes are paid once —
/// while boxing costs an indirection at every construction and match site,
/// including 38 in the tests that assert on parsed commands.
#[allow(clippy::large_enum_variant)]
#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Authenticate with Anthropic via OAuth PKCE flow (deprecated alias for `auth login`)
    Login,
    /// Sign out of Anthropic (deprecated alias for `auth logout`)
    Logout,
    /// Manage provider authentication
    Auth(AuthArgs),
    /// Single-turn chat completion against a selected provider
    Chat(ChatArgs),
    /// Inspect provider registry and Archon-level capability support
    Providers {
        #[command(subcommand)]
        action: Option<ProvidersAction>,
    },
    /// Inspect sandbox policy and backend readiness
    Sandbox {
        #[command(subcommand)]
        action: Option<SandboxAction>,
    },
    /// Audit durable permission decisions and denials
    Permissions {
        #[command(subcommand)]
        action: PermissionsAction,
    },
    /// Manage plugins
    Plugin {
        #[command(subcommand)]
        action: PluginAction,
    },
    /// Check for and install updates
    Update(UpdateArgs),
    /// Remote agent mode
    Remote {
        #[command(subcommand)]
        action: RemoteAction,
    },
    /// Start a WebSocket server for remote agent access
    Serve(ServeArgs),
    /// Manage and run multi-agent teams
    Team {
        #[command(subcommand)]
        action: TeamAction,
    },
    /// Speak the Agent Client Protocol over stdin/stdout, so an ACP-capable
    /// editor can drive archon without a per-editor extension (#189 Phase 11)
    Acp(AcpArgs),
    /// Run in IDE stdio mode (JSON-RPC over stdin/stdout)
    IdeStdio(IdeStdioArgs),
    /// Run and manage multi-agent pipelines
    Pipeline {
        #[command(subcommand)]
        action: PipelineAction,
    },
    /// Plan, run, resume, and inspect provider-neutral dynamic workflows
    Workflow {
        #[command(subcommand)]
        action: WorkflowAction,
    },
    /// Start the browser-based web UI on localhost
    Web(WebArgs),
    /// Submit an async agent task
    RunAgentAsync(RunAgentAsyncArgs),
    /// Draft a dissertation section with the FCDP protocol (D1 → D1.5 → D2 → gauntlet → R-loop)
    Draft(DraftArgs),
    /// Manage governed learning behaviour
    Behaviour {
        #[command(subcommand)]
        action: BehaviourAction,
    },
    /// Inspect agent definitions and governed agent evolution
    Agent {
        #[command(subcommand)]
        action: AgentAction,
    },
    /// Inspect learning subsystem diagnostics
    Learning {
        #[command(subcommand)]
        action: LearningAction,
    },
    /// Manage local world-model learning
    World {
        #[command(subcommand)]
        action: WorldAction,
    },
    /// Inspect first-class reasoning-quality claim/evidence events
    Reasoning {
        #[command(subcommand)]
        action: ReasoningAction,
    },
    /// Inspect and run the cognitive executive loop
    Cognitive {
        #[command(subcommand)]
        action: CognitiveAction,
    },
    /// Preview proactive session-start briefing content
    Briefing {
        #[command(subcommand)]
        action: BriefingAction,
    },
    /// Check status of an async task
    TaskStatus(TaskStatusArgs),
    /// Get result of a completed async task
    TaskResult(TaskResultArgs),
    /// Cancel a running async task
    TaskCancel(TaskCancelArgs),
    /// List async tasks
    TaskList(TaskListArgs),
    /// Stream events for a task (NDJSON)
    TaskEvents(TaskEventsArgs),
    /// Show task execution metrics (prometheus format)
    Metrics,
    /// List all discovered agents
    AgentList(AgentListArgs),
    /// Search agents by tag, capability, name pattern, or version
    AgentSearch(AgentSearchArgs),
    /// Show detailed information about a specific agent
    AgentInfo(AgentInfoArgs),
    /// Manage the knowledge base
    Kb {
        #[command(subcommand)]
        action: KbAction,
    },
    /// Manage document ingestion, inspection, and status
    Docs {
        #[command(subcommand)]
        action: DocsAction,
    },
    /// Ingest and inspect video evidence
    Video {
        #[command(subcommand)]
        action: VideoAction,
    },
    /// Governed trading research and execution-lab controls
    Trading {
        #[command(subcommand)]
        action: TradingCliAction,
    },
    /// Trace PRD requirements to code with a proof ladder
    Requirements {
        #[command(subcommand)]
        action: RequirementsAction,
    },
    /// Inspect and export provenance traces
    Prov {
        #[command(subcommand)]
        action: ProvAction,
    },
    /// Build meaning samples, pairs, triplets, and eval data
    Meaning {
        #[command(subcommand)]
        action: MeaningAction,
    },
    /// Build and inspect learned constellation centroids
    Constellation {
        #[command(subcommand)]
        action: ConstellationAction,
    },
    /// Manage the persistent memory graph
    Memory {
        #[command(subcommand)]
        action: MemoryAction,
    },
    /// Train and manage prose output-styles via Lanham style analysis
    Style {
        #[command(subcommand)]
        action: StyleAction,
    },
    /// Inspect Archon's self-calibration records
    #[command(name = "self")]
    SelfCmd {
        #[command(subcommand)]
        action: SelfAction,
    },
    /// Game-theory strategic analysis
    Gametheory(GametheoryArgs),
    /// Completion-integrity checks (TSPEC §10)
    Completion {
        #[command(subcommand)]
        action: CompletionAction,
    },
}

#[path = "commands_args.rs"]
pub(super) mod commands_args;
pub use commands_args::*;
