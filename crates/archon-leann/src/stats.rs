//! Index statistics and health.

use crate::metadata::IndexStats;
use std::fmt;

impl IndexStats {
    /// Create a new empty `IndexStats`.
    pub fn new() -> Self {
        Self::default()
    }
}

impl fmt::Display for IndexStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Index Statistics:")?;
        writeln!(f, "  Files:  {}", self.total_files)?;
        writeln!(f, "  Chunks: {}", self.total_chunks)?;
        writeln!(f, "  Size:   {} bytes", self.index_size_bytes)?;
        writeln!(f, "  Languages: {}", self.languages.len())?;
        if let Some(ref ts) = self.created_at {
            writeln!(f, "  Created: {}", ts)?;
        }
        Ok(())
    }
}
