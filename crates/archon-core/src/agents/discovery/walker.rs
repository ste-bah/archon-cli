// TASK-AGS-302: Recursive category walk with walkdir.
//
// Fixes the 11/234 bug: the old loader used non-recursive fs::read_dir
// against `custom/` only. This walker recurses all subdirectories and
// derives category from the first path component below root.

use std::path::{Path, PathBuf};

use crate::agents::catalog::DiscoveryError;

/// A file discovered during the agent directory walk.
#[derive(Debug, Clone)]
pub struct DiscoveredFile {
    pub path: PathBuf,
    pub category: String,
}

/// Allowed file extensions for agent metadata files.
const ALLOWED_EXTENSIONS: &[&str] = &["json", "yaml", "yml", "toml"];

/// Recursively walk `root` and return all agent metadata files with
/// their inferred category.
///
/// Category derivation: first path component below root. Files directly
/// in root get category "uncategorized". Hidden directories (starting
/// with `.`) are skipped.
pub fn walk_agents_dir(root: &Path) -> Result<Vec<DiscoveredFile>, DiscoveryError> {
    if !root.exists() {
        return Ok(Vec::new());
    }

    let mut results = Vec::new();

    for entry in walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            // Skip hidden directories/files (component starting with '.')
            // but always allow the root entry itself (depth 0)
            if e.depth() == 0 {
                return true;
            }
            e.file_name()
                .to_str()
                .map(|s| !s.starts_with('.'))
                .unwrap_or(false)
        })
    {
        let entry = entry.map_err(|e| {
            DiscoveryError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                e.to_string(),
            ))
        })?;

        // Only process files, not directories
        if !entry.file_type().is_file() {
            continue;
        }

        // Filter by extension
        let ext = entry
            .path()
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        if !ALLOWED_EXTENSIONS.contains(&ext) {
            continue;
        }

        // Derive category from first component below root
        let relative = entry
            .path()
            .strip_prefix(root)
            .map_err(|e| DiscoveryError::Parse(e.to_string()))?;

        let category = relative
            .components()
            .next()
            .and_then(|c| {
                let s = c.as_os_str().to_str()?;
                // If it's the filename itself (file directly in root),
                // return None to fall through to "uncategorized"
                if relative.components().count() == 1 {
                    None
                } else {
                    Some(s.to_string())
                }
            })
            .unwrap_or_else(|| "uncategorized".to_string());

        results.push(DiscoveredFile {
            path: entry.path().to_path_buf(),
            category,
        });
    }

    Ok(results)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn setup_fixture(tmp: &TempDir) {
        let root = tmp.path();
        // custom/a.yaml
        fs::create_dir_all(root.join("custom")).unwrap();
        fs::write(root.join("custom/a.yaml"), "name: a").unwrap();
        // development/b.json
        fs::create_dir_all(root.join("development")).unwrap();
        fs::write(root.join("development/b.json"), "{}").unwrap();
        // coding-pipeline/sub/c.toml
        fs::create_dir_all(root.join("coding-pipeline/sub")).unwrap();
        fs::write(root.join("coding-pipeline/sub/c.toml"), "").unwrap();
        // analysis/d.yml
        fs::create_dir_all(root.join("analysis")).unwrap();
        fs::write(root.join("analysis/d.yml"), "").unwrap();
    }

    #[test]
    fn discovers_files_across_categories() {
        let tmp = TempDir::new().unwrap();
        setup_fixture(&tmp);

        let results = walk_agents_dir(tmp.path()).unwrap();
        assert_eq!(results.len(), 4);

        let categories: Vec<&str> = results.iter().map(|f| f.category.as_str()).collect();
        assert!(categories.contains(&"custom"));
        assert!(categories.contains(&"development"));
        assert!(categories.contains(&"coding-pipeline"));
        assert!(categories.contains(&"analysis"));
    }

    #[test]
    fn hidden_directories_skipped() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join(".hidden")).unwrap();
        fs::write(tmp.path().join(".hidden/x.yaml"), "").unwrap();
        // Also add a visible one
        fs::create_dir_all(tmp.path().join("visible")).unwrap();
        fs::write(tmp.path().join("visible/y.yaml"), "").unwrap();

        let results = walk_agents_dir(tmp.path()).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].category, "visible");
    }

    #[test]
    fn non_whitelisted_extension_excluded() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("custom")).unwrap();
        fs::write(tmp.path().join("custom/readme.md"), "").unwrap();
        fs::write(tmp.path().join("custom/agent.yaml"), "").unwrap();

        let results = walk_agents_dir(tmp.path()).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].path.ends_with("agent.yaml"));
    }

    #[test]
    fn file_at_root_is_uncategorized() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("top.yaml"), "").unwrap();

        let results = walk_agents_dir(tmp.path()).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].category, "uncategorized");
    }

    #[test]
    fn perf_300_files_under_500ms() {
        let tmp = TempDir::new().unwrap();
        let categories = ["dev", "ops", "ml", "infra", "test", "core"];
        for (i, cat) in categories.iter().enumerate() {
            let dir = tmp.path().join(cat);
            fs::create_dir_all(&dir).unwrap();
            for j in 0..50 {
                fs::write(dir.join(format!("agent-{i}-{j}.yaml")), "").unwrap();
            }
        }

        let start = std::time::Instant::now();
        let results = walk_agents_dir(tmp.path()).unwrap();
        let elapsed = start.elapsed();

        assert_eq!(results.len(), 300);
        assert!(
            elapsed.as_millis() < 500,
            "walk took {}ms, expected <500ms",
            elapsed.as_millis()
        );
    }
}
