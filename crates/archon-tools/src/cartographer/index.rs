use std::collections::HashMap;

use petgraph::graph::DiGraph;
use serde::{Deserialize, Serialize};

/// A single extracted symbol from source code.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    /// Relative path from project root.
    pub file: String,
    pub line: usize,
    pub signature: String,
}

/// Kind of source code symbol.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SymbolKind {
    Struct,
    Class,
    Function,
    Method,
    Module,
    Enum,
    Interface,
    Type,
}

/// In-memory index of codebase symbols and dependency graph.
pub struct CodebaseIndex {
    /// Map from relative file path to list of symbols in that file.
    pub symbols: HashMap<String, Vec<Symbol>>,
    /// Directed dependency graph between files.
    pub deps: DiGraph<String, ()>,
    /// Map from file path to petgraph node index.
    pub node_index: HashMap<String, petgraph::graph::NodeIndex>,
    /// Map from file path to last-modified unix timestamp.
    pub mtimes: HashMap<String, u64>,
}

impl CodebaseIndex {
    /// Create a new empty index.
    pub fn new() -> Self {
        Self {
            symbols: HashMap::new(),
            deps: DiGraph::new(),
            node_index: HashMap::new(),
            mtimes: HashMap::new(),
        }
    }

    /// Find all symbols whose name contains `query` (case-insensitive substring match).
    pub fn find_symbol<'a>(&'a self, query: &str) -> Vec<&'a Symbol> {
        let lower = query.to_lowercase();
        let mut results = Vec::new();
        for syms in self.symbols.values() {
            for sym in syms {
                if sym.name.to_lowercase().contains(&lower) {
                    results.push(sym);
                }
            }
        }
        results
    }

    /// Return a slice of symbols for the given file path.
    pub fn symbols_in_file(&self, file: &str) -> &[Symbol] {
        self.symbols.get(file).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Get or create a node for the given file path in the dependency graph.
    pub fn get_or_create_node(&mut self, file: &str) -> petgraph::graph::NodeIndex {
        if let Some(&idx) = self.node_index.get(file) {
            return idx;
        }
        let idx = self.deps.add_node(file.to_string());
        self.node_index.insert(file.to_string(), idx);
        idx
    }

    /// Add a dependency edge from `from` to `to`.
    pub fn add_dep_edge(&mut self, from: &str, to: &str) {
        let from_idx = self.get_or_create_node(from);
        let to_idx = self.get_or_create_node(to);
        // Avoid duplicate edges.
        if !self.deps.contains_edge(from_idx, to_idx) {
            self.deps.add_edge(from_idx, to_idx, ());
        }
    }
}

impl Default for CodebaseIndex {
    fn default() -> Self {
        Self::new()
    }
}
