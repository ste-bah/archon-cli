//! Agent-callable document intelligence tools required by the Evidence Engine.

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::evidence_cli;
use crate::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

pub const DOC_TOOL_NAMES: &[&str] = &[
    "DocIngest",
    "DocList",
    "DocGet",
    "DocStatus",
    "DocSearch",
    "DocAnswer",
    "DocProvenance",
    "DocInspect",
    "DocModelStatus",
];

macro_rules! doc_tool {
    ($struct_name:ident, $name:literal, $desc:literal, $schema:ident, $args:ident, $perm:expr) => {
        pub struct $struct_name;

        #[async_trait]
        impl Tool for $struct_name {
            fn name(&self) -> &str {
                $name
            }

            fn description(&self) -> &str {
                $desc
            }

            fn input_schema(&self) -> Value {
                $schema()
            }

            async fn execute(&self, input: Value, ctx: &ToolContext) -> ToolResult {
                match $args(&input) {
                    Ok(args) => evidence_cli::run_archon(args, ctx).await,
                    Err(e) => ToolResult::error(e),
                }
            }

            fn permission_level(&self, _input: &Value) -> PermissionLevel {
                $perm
            }
        }
    };
}

doc_tool!(
    DocIngest,
    "DocIngest",
    "Ingest a file or directory into the document evidence store.",
    path_schema,
    ingest_args,
    PermissionLevel::Risky
);
doc_tool!(
    DocList,
    "DocList",
    "List ingested documents.",
    empty_schema,
    list_args,
    PermissionLevel::Safe
);
doc_tool!(
    DocGet,
    "DocGet",
    "Show document metadata by id.",
    document_schema,
    get_args,
    PermissionLevel::Safe
);
doc_tool!(
    DocStatus,
    "DocStatus",
    "Show document ingestion/index status.",
    empty_schema,
    status_args,
    PermissionLevel::Safe
);
doc_tool!(
    DocSearch,
    "DocSearch",
    "Search document chunks with exact, semantic, or hybrid retrieval.",
    search_schema,
    search_args,
    PermissionLevel::Safe
);
doc_tool!(
    DocAnswer,
    "DocAnswer",
    "Answer a question using document evidence and citations.",
    query_schema,
    answer_args,
    PermissionLevel::Safe
);
doc_tool!(
    DocProvenance,
    "DocProvenance",
    "Show provenance for a chunk or answer artifact.",
    provenance_schema,
    provenance_args,
    PermissionLevel::Safe
);
doc_tool!(
    DocInspect,
    "DocInspect",
    "Inspect a document including pages, chunks, OCR runs, and provenance.",
    document_schema,
    inspect_args,
    PermissionLevel::Safe
);
doc_tool!(
    DocModelStatus,
    "DocModelStatus",
    "Report local embedding/OCR model status.",
    empty_schema,
    model_status_args,
    PermissionLevel::Safe
);

fn ingest_args(input: &Value) -> Result<Vec<String>, String> {
    Ok(vec![
        "docs".into(),
        "ingest".into(),
        evidence_cli::required_string(input, "path")?,
    ])
}

fn list_args(_input: &Value) -> Result<Vec<String>, String> {
    Ok(vec!["docs".into(), "list".into()])
}

fn get_args(input: &Value) -> Result<Vec<String>, String> {
    Ok(vec!["docs".into(), "show".into(), document_id(input)?])
}

fn status_args(_input: &Value) -> Result<Vec<String>, String> {
    Ok(vec!["docs".into(), "status".into()])
}

fn search_args(input: &Value) -> Result<Vec<String>, String> {
    let mut args = vec![
        "docs".into(),
        "search".into(),
        evidence_cli::required_string(input, "query")?,
    ];
    args.push("--mode".into());
    args.push(evidence_cli::opt_string(input, "mode").unwrap_or_else(|| "hybrid".into()));
    if evidence_cli::opt_bool(input, "debug") {
        args.push("--debug".into());
    }
    Ok(args)
}

fn answer_args(input: &Value) -> Result<Vec<String>, String> {
    Ok(vec![
        "docs".into(),
        "answer".into(),
        evidence_cli::required_string(input, "query")?,
    ])
}

fn provenance_args(input: &Value) -> Result<Vec<String>, String> {
    Ok(vec![
        "docs".into(),
        "provenance".into(),
        evidence_cli::required_string(input, "artifact_id")?,
    ])
}

fn inspect_args(input: &Value) -> Result<Vec<String>, String> {
    Ok(vec!["docs".into(), "inspect".into(), document_id(input)?])
}

fn model_status_args(_input: &Value) -> Result<Vec<String>, String> {
    Ok(vec!["docs".into(), "model-status".into()])
}

fn document_id(input: &Value) -> Result<String, String> {
    evidence_cli::required_string(input, "document_id")
}

fn empty_schema() -> Value {
    evidence_cli::object_schema(json!({}), &[])
}

fn path_schema() -> Value {
    evidence_cli::object_schema(json!({ "path": { "type": "string" } }), &["path"])
}

fn document_schema() -> Value {
    evidence_cli::object_schema(
        json!({ "document_id": { "type": "string" } }),
        &["document_id"],
    )
}

fn query_schema() -> Value {
    evidence_cli::object_schema(json!({ "query": { "type": "string" } }), &["query"])
}

fn search_schema() -> Value {
    evidence_cli::object_schema(
        json!({
            "query": { "type": "string" },
            "mode": { "type": "string", "enum": ["exact", "semantic", "hybrid"] },
            "debug": { "type": "boolean" }
        }),
        &["query"],
    )
}

fn provenance_schema() -> Value {
    evidence_cli::object_schema(
        json!({ "artifact_id": { "type": "string" } }),
        &["artifact_id"],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_doc_tool_names_match_tspec_surface() {
        assert_eq!(DOC_TOOL_NAMES.len(), 9);
        assert!(DOC_TOOL_NAMES.contains(&"DocIngest"));
        assert!(DOC_TOOL_NAMES.contains(&"DocModelStatus"));
    }

    #[test]
    fn test_doc_search_defaults_to_hybrid() {
        let args = search_args(&json!({ "query": "needle" })).unwrap();
        assert_eq!(args, vec!["docs", "search", "needle", "--mode", "hybrid"]);
    }

    #[test]
    fn test_doc_get_rejects_empty_id() {
        let err = get_args(&json!({ "document_id": " " })).unwrap_err();
        assert!(err.contains("document_id is required"));
    }
}
