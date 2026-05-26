use clap::Subcommand;

#[derive(Subcommand, Debug, Clone)]
pub enum CognitiveAction {
    /// Show read-only cognitive executive-loop status.
    Status {
        /// Emit machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Run one governed autonomous cognitive maintenance tick.
    Tick {
        /// Emit machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Run or manage the background cognitive daemon.
    Daemon {
        #[command(subcommand)]
        action: CognitiveDaemonAction,
    },
    /// Inspect a cognitive decision or recent decisions for a session.
    Inspect {
        /// Decision id to inspect.
        decision_id: Option<String>,
        /// Session id to list decisions for.
        #[arg(long)]
        session: Option<String>,
        /// Maximum session decisions to show.
        #[arg(long, default_value = "10")]
        limit: usize,
        /// Emit machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Show self-model facts and trust calibration.
    SelfModel {
        /// Domain to inspect. Repeat for multiple domains.
        #[arg(long = "domain")]
        domains: Vec<String>,
        /// Emit machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// List recent safe cognitive reflection summaries.
    Reflections {
        /// Optional session id filter.
        #[arg(long)]
        session: Option<String>,
        /// Maximum reflections to show.
        #[arg(long, default_value = "10")]
        limit: usize,
        /// Emit machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum CognitiveDaemonAction {
    /// Spawn the daemon in the background.
    Start {
        /// Override the configured interval for this daemon process.
        #[arg(long)]
        interval_ms: Option<u64>,
        /// Emit machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Run the daemon in the foreground.
    Run {
        /// Override the configured interval for this daemon process.
        #[arg(long)]
        interval_ms: Option<u64>,
        /// Emit machine-readable JSON on exit.
        #[arg(long)]
        json: bool,
    },
    /// Run exactly one daemon maintenance pass with daemon policy gates.
    RunOnce {
        /// Emit machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Request the running daemon to stop.
    Stop {
        /// Emit machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Show daemon status.
    Status {
        /// Emit machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
}
