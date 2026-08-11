//! `archon kb kbs` — list every knowledge base the store knows about.
//!
//! The rest of the family takes `--kb <name>` as a filter and assumes the name
//! is already known. This verb is the only way to recover a name you did not
//! record (#170), so it prints the `kb_id` verbatim: that string is what
//! `--kb` matches, and it is not always the directory slug the web workbench
//! shows.

use anyhow::Result;
use cozo::DbInstance;

pub(crate) fn print_kbs(db: &DbInstance) -> Result<()> {
    let rows = archon_knowledge::store::list_kbs(db)?;
    for row in &rows {
        println!("{}", describe(row));
    }
    println!("{} knowledge base(s)", rows.len());
    Ok(())
}

/// One listing line. The name comes first and unadorned so it can be copied
/// straight into `--kb`.
fn describe(row: &archon_knowledge::store::KnowledgeBaseRow) -> String {
    let mut line = format!("{}  {} document(s)", row.kb_id, row.documents);
    if !row.scope.is_empty() {
        line.push_str(&format!("  scope={}", row.scope));
    }
    if row.registered && row.documents == 0 {
        line.push_str("  (declared, nothing attached yet)");
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli_args::KbAction;

    fn store_path(dir: &tempfile::TempDir) -> std::path::PathBuf {
        dir.path().join("evidence.db")
    }

    /// The verb has to run end to end against a real store, not just format a
    /// row: an unwired listing that prints nothing looks exactly like a store
    /// with no knowledge bases.
    #[tokio::test]
    async fn kbs_verb_runs_against_a_store() {
        let dir = tempfile::tempdir().unwrap();
        let path = store_path(&dir);
        {
            let db = archon_docs::acquire_docs_db(&path).unwrap();
            archon_docs::schema::ensure_doc_schema(&db).unwrap();
            archon_knowledge::schema::ensure_knowledge_schema(&db).unwrap();
            archon_knowledge::store::register_kb(&db, "alpha", "project", "notes").unwrap();
        }

        crate::command::kb::handle_kb_command_at(&path, KbAction::Kbs)
            .await
            .unwrap();
    }

    #[test]
    fn a_declared_but_empty_knowledge_base_says_so() {
        let row = archon_knowledge::store::KnowledgeBaseRow {
            kb_id: "alpha".into(),
            documents: 0,
            scope: "project".into(),
            registered: true,
        };
        let line = describe(&row);
        assert!(line.starts_with("alpha  0 document(s)"));
        assert!(line.contains("declared"));
    }

    /// `/kb kbs` reaches this through `CliMirrorHandler::prefixed("kb", ..)`,
    /// which forwards whatever it is handed — there is no per-subcommand
    /// allowlist here, unlike `/cognitive`, whose list twice omitted a real
    /// subcommand and answered "unknown subcommand" for a shipped command. So
    /// the only way the TUI form can break is the CLI verb not parsing.
    #[test]
    fn the_cli_parses_the_kbs_verb() {
        use clap::Parser;

        let cli = crate::cli_args::Cli::parse_from(["archon", "kb", "kbs"]);

        assert!(
            matches!(
                cli.command,
                Some(crate::cli_args::Commands::Kb {
                    action: KbAction::Kbs
                })
            ),
            "`archon kb kbs` must parse to the listing action"
        );
    }

    /// Whatever else the line carries, the first token is the exact string to
    /// pass to `--kb`.
    #[test]
    fn the_name_leads_the_line_verbatim() {
        let row = archon_knowledge::store::KnowledgeBaseRow {
            kb_id: "two words".into(),
            documents: 3,
            scope: String::new(),
            registered: false,
        };
        assert_eq!(describe(&row), "two words  3 document(s)");
    }
}
