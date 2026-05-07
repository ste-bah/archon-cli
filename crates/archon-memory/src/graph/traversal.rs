use std::collections::{BTreeMap, HashSet};

use cozo::{DataValue, ScriptMutability};

use super::MemoryGraph;
use super::helpers::db_err;
use crate::types::{Memory, MemoryError};

impl MemoryGraph {
    // -- graph traversal ---------------------------------------

    /// Recursive BFS traversal up to `depth` hops from a starting memory.
    /// Returns all reachable memories (excluding the start node).
    pub fn get_related_memories(&self, id: &str, depth: u32) -> Result<Vec<Memory>, MemoryError> {
        if depth == 0 {
            return Ok(Vec::new());
        }

        let mut visited: HashSet<String> = HashSet::new();
        visited.insert(id.to_string());
        let mut frontier: Vec<String> = vec![id.to_string()];

        for _ in 0..depth {
            if frontier.is_empty() {
                break;
            }
            let mut next_frontier: Vec<String> = Vec::new();
            for node in &frontier {
                let neighbours = self.direct_neighbours(node)?;
                for n in neighbours {
                    if visited.insert(n.clone()) {
                        next_frontier.push(n);
                    }
                }
            }
            frontier = next_frontier;
        }

        visited.remove(id);

        let mut results = Vec::with_capacity(visited.len());
        for mem_id in &visited {
            match self.read_memory(mem_id) {
                Ok(m) => results.push(m),
                Err(MemoryError::NotFound(_)) => continue,
                Err(e) => return Err(e),
            }
        }
        Ok(results)
    }

    /// Return ids of all direct neighbours (both directions).
    fn direct_neighbours(&self, id: &str) -> Result<Vec<String>, MemoryError> {
        let mut params = BTreeMap::new();
        params.insert("id".to_string(), DataValue::from(id));

        let result = self
            .db
            .run_script(
                "?[neighbour] := *relationships{from_id, to_id}, from_id = $id, neighbour = to_id
                 ?[neighbour] := *relationships{from_id, to_id}, to_id = $id, neighbour = from_id",
                params,
                ScriptMutability::Immutable,
            )
            .map_err(db_err)?;

        let mut ids = Vec::new();
        for row in &result.rows {
            if let Some(s) = row[0].get_str() {
                ids.push(s.to_string());
            }
        }
        Ok(ids)
    }
}
