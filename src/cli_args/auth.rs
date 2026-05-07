use clap::{Args, Subcommand, ValueEnum};

#[derive(Args, Debug, Clone)]
pub struct AuthArgs {
    #[command(subcommand)]
    pub command: AuthSubcommand,
}

#[derive(Subcommand, Debug, Clone)]
pub enum AuthSubcommand {
    Login {
        #[arg(long, value_enum, default_value = "anthropic")]
        provider: AuthProviderKind,
        #[arg(long, help = "Skip TOS warning prompt for this invocation only")]
        accept_tos: bool,
    },
    Status,
    Logout {
        #[arg(long, value_enum)]
        provider: Option<AuthProviderKind>,
    },
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthProviderKind {
    Anthropic,
    #[value(name = "openai-codex")]
    OpenaiCodex,
    Google,
}

#[derive(Args, Debug, Clone)]
pub struct ChatArgs {
    /// Provider id (e.g. "anthropic", "openai-codex")
    #[arg(long, default_value = "anthropic")]
    pub provider: String,
    /// Model id override
    #[arg(long)]
    pub model: Option<String>,
    /// Disable streaming; print full response after completion
    #[arg(long)]
    pub no_stream: bool,
    /// Maximum output tokens
    #[arg(long, default_value_t = 1024)]
    pub max_tokens: u32,
    /// User prompt
    pub prompt: String,
}
