//! Human-readable formatters for LSP operation results (TASK-CLI-313).

use lsp_types::{
    CallHierarchyIncomingCall, CallHierarchyItem, CallHierarchyOutgoingCall, DocumentSymbol,
    GotoDefinitionResponse, Hover, HoverContents, Location, MarkedString, MarkupContent,
    SymbolInformation, SymbolKind,
};

// ---------------------------------------------------------------------------
// goToDefinition
// ---------------------------------------------------------------------------

pub fn format_go_to_definition(result: &GotoDefinitionResponse) -> (String, usize, usize) {
    let locations = match result {
        GotoDefinitionResponse::Scalar(loc) => vec![loc.clone()],
        GotoDefinitionResponse::Array(locs) => locs.clone(),
        GotoDefinitionResponse::Link(links) => links
            .iter()
            .map(|l| Location {
                uri: l.target_uri.clone(),
                range: l.target_range,
            })
            .collect(),
    };
    format_locations_result("Definition", &locations)
}

// ---------------------------------------------------------------------------
// findReferences
// ---------------------------------------------------------------------------

pub fn format_find_references(locations: &[Location]) -> (String, usize, usize) {
    format_locations_result("References", locations)
}

// ---------------------------------------------------------------------------
// hover
// ---------------------------------------------------------------------------

pub fn format_hover(hover: &Hover) -> String {
    match &hover.contents {
        HoverContents::Scalar(marked) => format_marked_string(marked),
        HoverContents::Array(items) => items
            .iter()
            .map(format_marked_string)
            .collect::<Vec<_>>()
            .join("\n---\n"),
        HoverContents::Markup(MarkupContent { value, .. }) => value.clone(),
    }
}

fn format_marked_string(s: &MarkedString) -> String {
    match s {
        MarkedString::String(text) => text.clone(),
        MarkedString::LanguageString(ls) => format!("```{}\n{}\n```", ls.language, ls.value),
    }
}

// ---------------------------------------------------------------------------
// documentSymbol
// ---------------------------------------------------------------------------

pub fn format_document_symbols_flat(symbols: &[SymbolInformation]) -> String {
    if symbols.is_empty() {
        return "No symbols found.".to_string();
    }
    symbols
        .iter()
        .map(|s| {
            format!(
                "{} {} ({}:{})",
                symbol_kind_str(s.kind),
                s.name,
                s.location.uri.path(),
                s.location.range.start.line + 1
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn format_document_symbols_nested(symbols: &[DocumentSymbol], indent: usize) -> String {
    if symbols.is_empty() {
        return "No symbols found.".to_string();
    }
    let prefix = "  ".repeat(indent);
    symbols
        .iter()
        .map(|s| {
            let children = s.children.as_deref().unwrap_or(&[]);
            if children.is_empty() {
                format!(
                    "{}{} {} (line {})",
                    prefix,
                    symbol_kind_str(s.kind),
                    s.name,
                    s.range.start.line + 1
                )
            } else {
                let child_str = format_document_symbols_nested(children, indent + 1);
                format!(
                    "{}{} {} (line {})\n{}",
                    prefix,
                    symbol_kind_str(s.kind),
                    s.name,
                    s.range.start.line + 1,
                    child_str
                )
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ---------------------------------------------------------------------------
// workspaceSymbol
// ---------------------------------------------------------------------------

pub fn format_workspace_symbols(symbols: &[SymbolInformation]) -> (String, usize, usize) {
    if symbols.is_empty() {
        return ("No workspace symbols found.".to_string(), 0, 0);
    }
    let file_count = symbols
        .iter()
        .map(|s| s.location.uri.path().to_string())
        .collect::<std::collections::HashSet<_>>()
        .len();
    let text = symbols
        .iter()
        .map(|s| {
            format!(
                "{} {} — {}:{}",
                symbol_kind_str(s.kind),
                s.name,
                s.location.uri.path(),
                s.location.range.start.line + 1
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    (text, symbols.len(), file_count)
}

// ---------------------------------------------------------------------------
// prepareCallHierarchy / incomingCalls / outgoingCalls
// ---------------------------------------------------------------------------

pub fn format_prepare_call_hierarchy(items: &[CallHierarchyItem]) -> String {
    if items.is_empty() {
        return "No call hierarchy item found at this position.".to_string();
    }
    items
        .iter()
        .map(|item| {
            format!(
                "{} {} ({}:{})",
                symbol_kind_str(item.kind),
                item.name,
                item.uri.path(),
                item.range.start.line + 1
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn format_incoming_calls(calls: &[CallHierarchyIncomingCall]) -> (String, usize, usize) {
    if calls.is_empty() {
        return ("No incoming calls.".to_string(), 0, 0);
    }
    let file_count = calls
        .iter()
        .map(|c| c.from.uri.path().to_string())
        .collect::<std::collections::HashSet<_>>()
        .len();
    let text = calls
        .iter()
        .map(|c| {
            format!(
                "← {} {} ({}:{})",
                symbol_kind_str(c.from.kind),
                c.from.name,
                c.from.uri.path(),
                c.from.range.start.line + 1
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    (text, calls.len(), file_count)
}

pub fn format_outgoing_calls(calls: &[CallHierarchyOutgoingCall]) -> (String, usize, usize) {
    if calls.is_empty() {
        return ("No outgoing calls.".to_string(), 0, 0);
    }
    let file_count = calls
        .iter()
        .map(|c| c.to.uri.path().to_string())
        .collect::<std::collections::HashSet<_>>()
        .len();
    let text = calls
        .iter()
        .map(|c| {
            format!(
                "→ {} {} ({}:{})",
                symbol_kind_str(c.to.kind),
                c.to.name,
                c.to.uri.path(),
                c.to.range.start.line + 1
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    (text, calls.len(), file_count)
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn format_locations_result(label: &str, locations: &[Location]) -> (String, usize, usize) {
    if locations.is_empty() {
        return (format!("No {} found.", label.to_lowercase()), 0, 0);
    }
    let file_count = locations
        .iter()
        .map(|l| l.uri.path().to_string())
        .collect::<std::collections::HashSet<_>>()
        .len();
    let text = locations
        .iter()
        .map(|l| {
            format!(
                "{}:{}:{}",
                l.uri.path(),
                l.range.start.line + 1,
                l.range.start.character + 1
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    (text, locations.len(), file_count)
}

fn symbol_kind_str(kind: SymbolKind) -> &'static str {
    match kind {
        SymbolKind::FILE => "file",
        SymbolKind::MODULE => "module",
        SymbolKind::NAMESPACE => "namespace",
        SymbolKind::PACKAGE => "package",
        SymbolKind::CLASS => "class",
        SymbolKind::METHOD => "method",
        SymbolKind::PROPERTY => "property",
        SymbolKind::FIELD => "field",
        SymbolKind::CONSTRUCTOR => "constructor",
        SymbolKind::ENUM => "enum",
        SymbolKind::INTERFACE => "interface",
        SymbolKind::FUNCTION => "fn",
        SymbolKind::VARIABLE => "var",
        SymbolKind::CONSTANT => "const",
        SymbolKind::STRING => "string",
        SymbolKind::NUMBER => "number",
        SymbolKind::BOOLEAN => "bool",
        SymbolKind::ARRAY => "array",
        SymbolKind::STRUCT => "struct",
        SymbolKind::EVENT => "event",
        SymbolKind::OPERATOR => "op",
        SymbolKind::TYPE_PARAMETER => "type_param",
        _ => "symbol",
    }
}

// ---------------------------------------------------------------------------
// goToImplementation (same as goToDefinition format)
// ---------------------------------------------------------------------------

pub fn format_go_to_implementation(locations: &[Location]) -> (String, usize, usize) {
    format_locations_result("Implementation", locations)
}

// ---------------------------------------------------------------------------
// Hover for MarkupContent convenience
// ---------------------------------------------------------------------------

/// Format a plain markdown hover result.
pub fn format_hover_markup(content: &MarkupContent) -> String {
    content.value.clone()
}
