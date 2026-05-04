//! Slash handlers for Evidence Engine TUI inspection views.

use anyhow::Result;
use archon_tui::app::{TuiEvent, ViewId};

use crate::command::registry::{CommandContext, CommandHandler};

pub(crate) struct DocsViewHandler;
pub(crate) struct LearningViewHandler;

impl CommandHandler for DocsViewHandler {
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> Result<()> {
        if matches!(
            args.first().map(String::as_str),
            None | Some("open" | "view")
        ) {
            ctx.emit(TuiEvent::OpenView(ViewId::Docs));
            return Ok(());
        }
        ctx.emit(TuiEvent::TextDelta(
            "Usage: /docs [open|view]\nOpens the document/evidence TUI browser.".into(),
        ));
        Ok(())
    }

    fn description(&self) -> &str {
        "Open the document/evidence TUI browser"
    }
}

impl CommandHandler for LearningViewHandler {
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> Result<()> {
        if matches!(
            args.first().map(String::as_str),
            None | Some("open" | "view")
        ) {
            ctx.emit(TuiEvent::OpenView(ViewId::Learning));
            return Ok(());
        }
        ctx.emit(TuiEvent::TextDelta(
            "Usage: /learning [open|view]\nOpens the governed-learning TUI browser.".into(),
        ));
        Ok(())
    }

    fn description(&self) -> &str {
        "Open the governed-learning TUI browser"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::registry::default_registry;
    use crate::command::test_support::{CtxBuilder, drain_tui_events};

    #[test]
    fn default_registry_registers_evidence_view_primaries() {
        let registry = default_registry();
        assert!(registry.is_primary("docs"));
        assert!(registry.is_primary("learning"));
    }

    #[test]
    fn docs_view_handler_emits_open_view_event() {
        let (mut ctx, mut rx) = CtxBuilder::new().build();
        DocsViewHandler.execute(&mut ctx, &[]).unwrap();
        let events = drain_tui_events(&mut rx);
        assert!(matches!(
            events.as_slice(),
            [TuiEvent::OpenView(ViewId::Docs)]
        ));
    }

    #[test]
    fn learning_view_handler_emits_open_view_event() {
        let (mut ctx, mut rx) = CtxBuilder::new().build();
        LearningViewHandler.execute(&mut ctx, &[]).unwrap();
        let events = drain_tui_events(&mut rx);
        assert!(matches!(
            events.as_slice(),
            [TuiEvent::OpenView(ViewId::Learning)]
        ));
    }
}
