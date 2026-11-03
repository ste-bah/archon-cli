use super::*;

#[cfg(test)]
#[test]
fn atomic_candidate_replacement_never_exposes_partial_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("candidate.json");
    let first = serde_json::to_vec(&"a".repeat(256 * 1024)).unwrap();
    let second = serde_json::to_vec(&"b".repeat(256 * 1024)).unwrap();
    write_atomic(&target, &first).unwrap();
    let reader_path = target.clone();
    let reader = std::thread::spawn(move || {
        for _ in 0..200 {
            let bytes = std::fs::read(&reader_path).unwrap();
            assert!(serde_json::from_slice::<String>(&bytes).is_ok());
        }
    });
    for index in 0..20 {
        write_atomic(&target, if index % 2 == 0 { &second } else { &first }).unwrap();
    }
    reader.join().unwrap();
}

#[cfg(test)]
#[test]
fn archive_twice_with_the_same_publish_digest_preserves_both_records() {
    let dir = tempfile::tempdir().unwrap();
    for marker in ["first", "second"] {
        let records = dir.path().join(RECORDS);
        std::fs::create_dir_all(&records).unwrap();
        std::fs::write(records.join("record"), marker).unwrap();
        archive(dir.path(), "same-digest").unwrap();
    }
    let archives = std::fs::read_dir(dir.path().join(CONSUMED))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    assert_eq!(archives.len(), 2);
    let contents = archives
        .iter()
        .map(|archive| std::fs::read_to_string(archive.join("record")).unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        contents,
        ["first", "second"].into_iter().map(str::to_owned).collect()
    );
}

#[cfg(test)]
#[test]
fn forgetting_a_check_archives_its_saved_accepted_judgment() {
    let dir = tempfile::tempdir().unwrap();
    let records = dir.path().join(RECORDS);
    std::fs::create_dir_all(&records).unwrap();
    let path = records.join(format!("{}.key.json", digest(b"AC-X-001")));
    std::fs::write(
        &path,
        serde_json::json!({"verdict": "accepted", "check_id": "AC-X-001"}).to_string(),
    )
    .unwrap();

    forget(dir.path(), "AC-X-001").unwrap();

    assert_eq!(
        std::fs::read_dir(&records).unwrap().count(),
        0,
        "forgotten judgments leave the active cache"
    );
    let archived = dir.path().join(CONSUMED);
    let record = std::fs::read_dir(archived)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert_eq!(
        std::fs::read_to_string(record.join(path.file_name().unwrap())).unwrap(),
        serde_json::json!({"verdict": "accepted", "check_id": "AC-X-001"}).to_string()
    );
}

#[test]
fn named_file_helpers_are_bound_into_candidate_reuse() {
    for name in [
        "workflow_task_set_judge_named_files_text.rs",
        "workflow_task_set_judge_named_files_repository.rs",
    ] {
        let source = PROMPT_AND_JUDGE_SOURCES
            .iter()
            .find(|(path, _)| *path == name)
            .unwrap_or_else(|| panic!("missing source binding: {name}"));
        let mut changed = source.1.to_vec();
        changed.push(b' ');
        let sources: Vec<_> = PROMPT_AND_JUDGE_SOURCES
            .iter()
            .map(|(path, bytes)| {
                (
                    *path,
                    if *path == name {
                        changed.as_slice()
                    } else {
                        *bytes
                    },
                )
            })
            .collect();
        assert_ne!(
            prompt_and_judge_source_digest(PROMPT_AND_JUDGE_SOURCES),
            prompt_and_judge_source_digest(&sources),
            "{name}"
        );
    }
}
