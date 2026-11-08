use std::io::{self, Read};
use std::path::Path;
use std::process::Stdio;

const BLOCK_BYTES: usize = 8192;

// Inspect a bounded prefix rather than loading a potentially huge binary.
// NUL detects binary content without rejecting executable text scripts.
pub(super) fn is_text(bytes: &[u8]) -> bool {
    !bytes.iter().take(BLOCK_BYTES).any(|byte| *byte == 0)
}

fn reader_is_text(reader: impl Read) -> io::Result<bool> {
    let mut block = Vec::new();
    reader.take(BLOCK_BYTES as u64).read_to_end(&mut block)?;
    Ok(is_text(&block))
}

pub(super) fn file_is_text(path: &Path) -> io::Result<bool> {
    reader_is_text(std::fs::File::open(path)?)
}

pub(super) fn repository_is_text(repository: &Path, commit: &str, path: &str) -> io::Result<bool> {
    let mut child = archon_shell::spawn::git_command(repository)
        .args(["cat-file", "blob", &format!("{commit}:{path}")])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let mut block = Vec::new();
    let result = child
        .stdout
        .take()
        .expect("piped stdout")
        .take(BLOCK_BYTES as u64)
        .read_to_end(&mut block);
    // A full prefix is enough. Otherwise wait for EOF and verify the producer.
    let full = block.len() == BLOCK_BYTES;
    if full || result.is_err() {
        let _ = child.kill();
    }
    let status = child.wait()?;
    result?;
    if !full && !status.success() {
        return Err(io::Error::other("could not inspect repository content"));
    }
    Ok(is_text(&block))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nul_in_first_block_is_binary_but_scripts_are_text() {
        assert!(!is_text(b"\x7fELF\0payload"));
        assert!(!is_text(b"\xcf\xfa\xed\xfe\0payload"));
        assert!(is_text(b"#!/bin/sh\nexit 0\n"));
        assert!(is_text(b""));
        let mut bytes = vec![b'a'; BLOCK_BYTES + 1];
        bytes[BLOCK_BYTES - 1] = 0;
        assert!(!is_text(&bytes));
        bytes[BLOCK_BYTES - 1] = b'a';
        bytes[BLOCK_BYTES] = 0;
        assert!(is_text(&bytes));
    }

    #[test]
    fn large_working_and_committed_binary_prefixes_are_not_text() {
        let root = std::env::temp_dir().join(format!("binary-prefix-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("input");
        std::fs::File::create(&path)
            .unwrap()
            .set_len(100_173_120)
            .unwrap();
        assert!(!file_is_text(&path).unwrap());
        for args in [
            vec!["init", "-q"],
            vec!["add", "input"],
            vec![
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "-qm",
                "input",
            ],
        ] {
            assert!(
                std::process::Command::new("git")
                    .arg("-C")
                    .arg(&root)
                    .args(args)
                    .status()
                    .unwrap()
                    .success()
            );
        }
        assert!(!repository_is_text(&root, "HEAD", "input").unwrap());
        std::fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
        assert!(file_is_text(&path).unwrap());
        assert!(!repository_is_text(&root, "HEAD", "input").unwrap());
        assert!(repository_is_text(&root, "HEAD", "missing").is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
