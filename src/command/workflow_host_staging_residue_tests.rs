//! Residue records name staging directories, never paths (#297 round 9).
use super::*;

#[test]
fn a_record_names_its_staging_and_clearing_it_twice_is_harmless() {
    let run = tempfile::tempdir().unwrap();
    let residue = StagingResidue::at(run.path(), "call/../x");
    residue.record("call/../x", "cmd", 3).unwrap();
    assert_eq!(
        residue.path(),
        run.path().join(RESIDUE_DIR).join("call----x.json")
    );
    let staging = run.path().join("host-command-staging").join("call----x");
    std::fs::create_dir_all(staging.join("nested")).unwrap();
    assert_eq!(clear_left(run.path()).unwrap(), vec![staging.clone()]);
    assert!(!staging.exists() && !residue.path().exists());
    residue.clear().unwrap();
}

#[test]
fn a_record_whose_name_is_not_a_staging_name_refuses_the_resume() {
    let run = tempfile::tempdir().unwrap();
    let dir = run.path().join(RESIDUE_DIR);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a b.json"), b"{}").unwrap();
    let error = clear_left(run.path()).unwrap_err().to_string();
    assert!(error.contains("names no staging directory"), "{error}");
}
