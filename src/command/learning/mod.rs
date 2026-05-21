//! `archon learning` CLI handlers.

pub(crate) mod gnn;

use anyhow::Result;

use crate::cli_args::{LearningAction, LearningGnnAction};

pub(crate) async fn handle_learning_command(
    action: LearningAction,
    config: &archon_core::config::ArchonConfig,
) -> Result<()> {
    match action {
        LearningAction::Gnn {
            action: LearningGnnAction::Status,
        } => gnn::print_gnn_status(config).await,
    }
}
