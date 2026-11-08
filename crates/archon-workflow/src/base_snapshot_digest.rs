use crate::task_set_contract::content_digest;
use std::io::Read;
use std::path::Path;

pub(super) fn digest_live(path: &Path) -> Result<String, String> {
    fn file(path: &Path, remaining: &mut u64) -> Result<String, String> {
        let mut input =
            std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        if input
            .metadata()
            .map_err(|e| format!("{}: {e}", path.display()))?
            .len()
            > *remaining
        {
            return Err(format!("{}: live evidence exceeds 1 GiB", path.display()));
        }
        let mut hash = blake3::Hasher::new();
        let mut buffer = [0; 65536];
        loop {
            let count = input
                .read(&mut buffer)
                .map_err(|e| format!("{}: {e}", path.display()))?;
            if count == 0 {
                break;
            }
            *remaining = remaining
                .checked_sub(count as u64)
                .ok_or_else(|| format!("{}: live evidence exceeds 1 GiB", path.display()))?;
            hash.update(&buffer[..count]);
        }
        Ok(hash.finalize().to_hex().to_string())
    }
    fn visit(
        root: &Path,
        path: &Path,
        remaining: &mut u64,
        lines: &mut Vec<String>,
    ) -> Result<(), String> {
        let meta = path
            .symlink_metadata()
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let key = crate::repository_record::normalize_relative(
            &path
                .strip_prefix(root)
                .map_err(|e| e.to_string())?
                .to_string_lossy(),
        );
        let value = if meta.file_type().is_symlink() {
            format!(
                "link:{}",
                std::fs::read_link(path)
                    .map_err(|e| format!("{}: {e}", path.display()))?
                    .display()
            )
        } else if meta.is_dir() {
            "directory".into()
        } else if meta.is_file() {
            format!("file:{}:{}", meta.len(), file(path, remaining)?)
        } else {
            return Err(format!("{}: nonregular live evidence", path.display()));
        };
        lines.push(format!("{key}\t{value}\n"));
        if meta.is_dir() {
            for item in std::fs::read_dir(path).map_err(|e| format!("{}: {e}", path.display()))? {
                visit(
                    root,
                    &item.map_err(|e| e.to_string())?.path(),
                    remaining,
                    lines,
                )?;
            }
        }
        Ok(())
    }
    let mut remaining = 1 << 30;
    let meta = path
        .symlink_metadata()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    if meta.is_file() {
        return file(path, &mut remaining);
    }
    let mut lines = Vec::new();
    visit(path, path, &mut remaining, &mut lines)?;
    lines.sort();
    Ok(content_digest(lines.concat().as_bytes()))
}
