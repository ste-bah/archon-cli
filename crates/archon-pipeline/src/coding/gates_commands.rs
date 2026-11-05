use serde::{Deserialize, Serialize};

/// Language of the project being built.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Language {
    Rust,
    TypeScript,
    /// Explicit commands are authoritative for any project or toolchain.
    Commands {
        build: GateCommand,
        test: GateCommand,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateCommand {
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
}

impl GateCommand {
    fn preset(program: &str, args: &[&str]) -> Self {
        Self {
            program: program.into(),
            args: args.iter().map(|arg| (*arg).into()).collect(),
        }
    }
}

impl Language {
    pub(super) fn build_command(self) -> GateCommand {
        match self {
            Self::Rust => GateCommand::preset("cargo", &["build"]),
            Self::TypeScript => GateCommand::preset("npm", &["run", "build"]),
            Self::Commands { build, .. } => build,
        }
    }
    pub(super) fn test_command(self) -> GateCommand {
        match self {
            Self::Rust => GateCommand::preset("cargo", &["test"]),
            Self::TypeScript => GateCommand::preset("npm", &["test"]),
            Self::Commands { test, .. } => test,
        }
    }
}
