use super::*;
use crate::filesystem::{FileMeta, FileSystem, LocalFs};
use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Debug, Default)]
struct RecordingFs(Mutex<BTreeSet<PathBuf>>);

#[async_trait::async_trait]
impl FileSystem for RecordingFs {
    async fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        LocalFs.read(path).await
    }
    async fn write(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        LocalFs.write(path, bytes).await
    }
    async fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        LocalFs.create_dir_all(path).await
    }
    async fn metadata(&self, path: &Path) -> io::Result<FileMeta> {
        LocalFs.metadata(path).await
    }
    async fn read_dir(&self, path: &Path) -> io::Result<Vec<PathBuf>> {
        self.0.lock().unwrap().insert(path.to_path_buf());
        LocalFs.read_dir(path).await
    }
    async fn read_dir_with_types(&self, path: &Path) -> io::Result<Vec<(PathBuf, Option<bool>)>> {
        self.0.lock().unwrap().insert(path.to_path_buf());
        LocalFs.read_dir_with_types(path).await
    }
    async fn remove_file(&self, path: &Path) -> io::Result<()> {
        LocalFs.remove_file(path).await
    }
    async fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        LocalFs.rename(from, to).await
    }
    fn rerooted(self: Arc<Self>, _working_dir: &Path) -> Arc<dyn FileSystem> {
        self
    }
    async fn glob(&self, _base: &Path, _pattern: &str) -> io::Result<Vec<PathBuf>> {
        panic!("the production walk must perform directory reads")
    }
}

#[tokio::test]
async fn glob_fixed_prefix_visits_only_reachable_directories() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("workspace");
    let base = project.join(".archon");
    std::fs::create_dir_all(base.join("selected/data/nested")).unwrap();
    for index in 0..64 {
        std::fs::create_dir_all(base.join(format!("outside-{index}/branch/leaf"))).unwrap();
        std::fs::create_dir_all(base.join(format!("selected/other-{index}/leaf"))).unwrap();
    }
    std::fs::create_dir_all(base.join("selected-other/branch")).unwrap();
    for name in [
        "selected/data/first.json",
        "selected/data/nested/second.json",
    ] {
        std::fs::write(base.join(name), "{}").unwrap();
    }
    let project = project.canonicalize().unwrap();
    let base = base.canonicalize().unwrap();
    let fs = Arc::new(RecordingFs::default());
    let ctx = ToolContext {
        working_dir: project,
        fs: Some(fs.clone()),
        ..ToolContext::default()
    };
    let expected = ["", "selected", "selected/data", "selected/data/nested"]
        .into_iter()
        .map(|path| base.join(path))
        .collect::<BTreeSet<_>>();
    for pattern in [
        "selected/data/**/*.json".to_string(),
        base.join("selected/data/**/*.json")
            .to_string_lossy()
            .into_owned(),
    ] {
        fs.0.lock().unwrap().clear();
        let result = GlobTool
            .execute(json!({"path": base, "pattern": pattern}), &ctx)
            .await;
        assert!(!result.is_error, "{}", result.content);
        assert_eq!(result.content.lines().count(), 2, "{}", result.content);
        let visited = fs.0.lock().unwrap().clone();
        assert_eq!(
            visited.len(),
            expected.len(),
            "unreachable directories were walked"
        );
        assert_eq!(visited, expected);
    }
    // With no fixed directory prefix, every branch remains reachable.
    fs.0.lock().unwrap().clear();
    let result = GlobTool
        .execute(json!({"path": base, "pattern": "*/data/**/*.json"}), &ctx)
        .await;
    assert!(!result.is_error, "{}", result.content);
    assert_eq!(result.content.lines().count(), 2, "{}", result.content);
    assert!(fs.0.lock().unwrap().len() > expected.len());
}
