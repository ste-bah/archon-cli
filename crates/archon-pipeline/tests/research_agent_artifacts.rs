use std::fs;

use archon_pipeline::research::artifacts::write_research_agent_artifacts;

#[test]
fn writes_canonical_markdown_and_named_artifacts() {
    let tmp = tempfile::tempdir().unwrap();
    let artifacts = write_research_agent_artifacts(
        tmp.path(),
        5,
        "dissertation-architect",
        "# Structure\n\n**Total Chapters**: 8\n",
    )
    .unwrap();

    let canonical = tmp
        .path()
        .join("outputs/markdown/005-dissertation-architect.md");
    let named = tmp
        .path()
        .join("outputs/artifacts/005-dissertation-architect/chapter-structure.md");
    let rlm = tmp
        .path()
        .join("outputs/rlm/research/structure/chapters.md");

    assert!(canonical.exists());
    assert!(named.exists());
    assert!(rlm.exists());
    assert!(artifacts.iter().any(|artifact| artifact.path == canonical));
    assert!(
        fs::read_to_string(named)
            .unwrap()
            .contains("Total Chapters")
    );
    assert!(fs::read_to_string(rlm).unwrap().contains("Total Chapters"));
}
