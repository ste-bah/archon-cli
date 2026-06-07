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
pub struct DocList;

#[async_trait]
impl Tool for DocList {
    fn name(&self) -> &str {
        "DocList"
    }

    fn description(&self) -> &str {
        "List a compact document inventory. Use DocSearch, not DocList, for content questions."
    }

    fn input_schema(&self) -> Value {
        list_schema()
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> ToolResult {
        match list_limit(&input) {
            Ok(limit) => crate::docs_runtime::run_list(limit, ctx).await,
            Err(e) => ToolResult::error(e),
        }
    }

    fn permission_level(&self, _input: &Value) -> PermissionLevel {
        PermissionLevel::Safe
    }
}

pub struct DocGet;

#[async_trait]
impl Tool for DocGet {
    fn name(&self) -> &str {
        "DocGet"
    }

    fn description(&self) -> &str {
        "Show compact document metadata by id. Use DocSearch for document content."
    }

    fn input_schema(&self) -> Value {
        document_schema()
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> ToolResult {
        match document_id(&input) {
            Ok(document_id) => crate::docs_runtime::run_get(document_id, ctx).await,
            Err(e) => ToolResult::error(e),
        }
    }

    fn permission_level(&self, _input: &Value) -> PermissionLevel {
        PermissionLevel::Safe
    }
}
doc_tool!(
    DocStatus,
    "DocStatus",
    "Show document ingestion/index status.",
    empty_schema,
    status_args,
    PermissionLevel::Safe
);
pub struct DocSearch;

#[async_trait]
impl Tool for DocSearch {
    fn name(&self) -> &str {
        "DocSearch"
    }

    fn description(&self) -> &str {
        "Search document chunks with exact, semantic, or hybrid retrieval."
    }

    fn input_schema(&self) -> Value {
        search_schema()
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> ToolResult {
        match search_args(&input) {
            Ok(args) => crate::docs_runtime::run_search(args, ctx).await,
            Err(e) => ToolResult::error(e),
        }
    }

    fn permission_level(&self, _input: &Value) -> PermissionLevel {
        PermissionLevel::Safe
    }
}

pub struct DocAnswer;

#[async_trait]
impl Tool for DocAnswer {
    fn name(&self) -> &str {
        "DocAnswer"
    }

    fn description(&self) -> &str {
        "Answer a question using document evidence and citations."
    }

    fn input_schema(&self) -> Value {
        query_schema()
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> ToolResult {
        match answer_args(&input) {
            Ok(args) => crate::docs_runtime::run_answer(args, ctx).await,
            Err(e) => ToolResult::error(e),
        }
    }

    fn permission_level(&self, _input: &Value) -> PermissionLevel {
        PermissionLevel::Safe
    }
}
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

fn list_limit(input: &Value) -> Result<usize, String> {
    Ok(evidence_cli::opt_usize(input, "limit", 25)?.clamp(1, 50))
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

fn list_schema() -> Value {
    evidence_cli::object_schema(
        json!({ "limit": { "type": "integer", "minimum": 1, "maximum": 50 } }),
        &[],
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
        let err = document_id(&json!({ "document_id": " " })).unwrap_err();
        assert!(err.contains("document_id is required"));
    }

    #[test]
    fn test_doc_list_limit_is_capped() {
        assert_eq!(list_limit(&json!({ "limit": 500 })).unwrap(), 50);
        assert_eq!(list_limit(&json!({ "limit": 0 })).unwrap(), 1);
    }
}
