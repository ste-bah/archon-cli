use super::*;

/// A run directory laid out as the host lays one out: the staging root a
/// child is given, and the attempt's spill directory beside it.
fn run_layout() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let run = tempfile::tempdir().unwrap();
    let staging = run.path().join("host-command-staging").join("call-1");
    std::fs::create_dir_all(&staging).unwrap();
    let attempt = run
        .path()
        .join("host-command-results")
        .join("call-1")
        .join("attempt-1-x");
    (run, staging, attempt)
}

#[test]
fn only_stdin_past_the_limit_or_with_the_reserved_prefix_spills() {
    let limit = 8;
    assert!(!needs_spill(b"", limit), "empty stdin stays inline");
    assert!(
        !needs_spill(b"12345678", limit),
        "exactly at the limit stays inline"
    );
    assert!(
        needs_spill(b"123456789", limit),
        "one byte past the limit spills"
    );
    assert!(
        needs_spill(STDIN_SPILL_PREFIX.as_bytes(), u64::MAX),
        "authored text that looks like a view is spilled, never taken for one"
    );
}

#[test]
fn the_child_reads_every_spilled_byte_and_never_the_view() {
    let (_run, staging, attempt) = run_layout();
    // Multi-byte text across what would be any chunk boundary.
    let candidate = "é".repeat(600_000).into_bytes();
    let view = spill(&attempt, &candidate).unwrap();
    assert!(view.starts_with(STDIN_SPILL_PREFIX.as_bytes()));
    assert!(
        view.len() < 4096,
        "the view is bounded: {} bytes",
        view.len()
    );
    let named: StdinSpillView =
        serde_json::from_slice(view[STDIN_SPILL_PREFIX.len()..].trim_ascii_end()).unwrap();
    assert_eq!(named.path, attempt.join(STDIN_SPILL_FILE));
    assert_eq!(std::fs::read(&named.path).unwrap(), candidate);
    let read = read_candidate(view.as_slice(), 1024, &staging).unwrap();
    assert_eq!(read.len(), candidate.len());
    assert_eq!(read, candidate);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&named.path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "the spill is owner-only");
    }
}

#[test]
fn inline_stdin_is_read_as_before_and_refused_past_the_limit() {
    let (_run, staging, _) = run_layout();
    assert_eq!(
        read_candidate(&b"inline body"[..], 64, &staging).unwrap(),
        b"inline body"
    );
    let error = read_candidate(&[b'x'; 65][..], 64, &staging).unwrap_err();
    assert!(error.to_string().contains("exceeds 64 bytes"), "{error}");
}

#[test]
fn a_view_naming_bytes_the_spill_does_not_hold_is_refused() {
    let (_run, staging, attempt) = run_layout();
    let view = spill(&attempt, b"the candidate").unwrap();
    std::fs::write(attempt.join(STDIN_SPILL_FILE), b"the candidatX").unwrap();
    let error = read_candidate(view.as_slice(), 64, &staging).unwrap_err();
    assert!(error.to_string().contains("but its view names"), "{error}");
}

#[test]
fn a_view_naming_a_file_outside_the_run_spill_root_is_refused() {
    let (run, staging, _) = run_layout();
    let outside = run.path().join("secret.txt");
    std::fs::write(&outside, b"not a spill").unwrap();
    std::fs::create_dir_all(run.path().join("host-command-results")).unwrap();
    let view = StdinSpillView {
        path: outside.clone(),
        bytes: 11,
        blake3: archon_workflow::task_set_contract::content_digest(b"not a spill"),
    }
    .encode()
    .unwrap();
    let error = read_candidate(view.as_slice(), 64, &staging).unwrap_err();
    assert!(
        error.to_string().contains("is not a regular file under"),
        "{error}"
    );
}

#[cfg(unix)]
#[test]
fn a_view_naming_a_symlink_is_refused() {
    let (run, staging, attempt) = run_layout();
    spill(&attempt, b"spilled").unwrap();
    let link = attempt.join("link.bin");
    std::os::unix::fs::symlink(attempt.join(STDIN_SPILL_FILE), &link).unwrap();
    let view = StdinSpillView {
        path: link,
        bytes: 7,
        blake3: archon_workflow::task_set_contract::content_digest(b"spilled"),
    }
    .encode()
    .unwrap();
    let error = read_candidate(view.as_slice(), 64, &staging).unwrap_err();
    assert!(
        error.to_string().contains("is not a regular file under"),
        "{error}"
    );
    drop(run);
}

#[test]
fn a_view_is_read_whole_even_when_the_inline_limit_is_shorter_than_it() {
    let (_run, staging, attempt) = run_layout();
    let view = spill(&attempt, b"0123456789").unwrap();
    assert!(view.len() > 8);
    assert_eq!(
        read_candidate(view.as_slice(), 8, &staging).unwrap(),
        b"0123456789"
    );
}

#[test]
fn a_second_spill_into_the_same_attempt_is_refused_not_overwritten() {
    let (_run, _staging, attempt) = run_layout();
    spill(&attempt, b"first").unwrap();
    assert!(spill(&attempt, b"second").is_err());
    assert_eq!(
        std::fs::read(attempt.join(STDIN_SPILL_FILE)).unwrap(),
        b"first"
    );
}
