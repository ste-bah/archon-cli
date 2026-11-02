//! Issue 544 (review r8): verdicts cached by an older binary carry failure
//! fingerprints of another format (masked text, not indexed placeholders);
//! the cache key names its format, so an old entry is observed again, never
//! compared with a new fingerprint.

use super::*;
use std::sync::Mutex;

struct Counting(Mutex<usize>);

#[async_trait::async_trait]
impl CheckObserver for Counting {
    async fn observe(&self, _: &str, ids: &[String]) -> Option<BTreeMap<String, Verdict>> {
        *self.0.lock().unwrap() += 1;
        Some(
            ids.iter()
                .map(|id| {
                    (
                        id.clone(),
                        Verdict::Failed("Some(3) refused: <secret#1>".into()),
                    )
                })
                .collect(),
        )
    }
}

#[tokio::test]
async fn an_old_format_cached_verdict_is_observed_again() {
    let run = tempfile::tempdir().unwrap();
    let commit = "0123456789abcdef0123456789abcdef01234567";
    let check = FailingCheck {
        id: "AC-X-001".into(),
        owners: Vec::new(),
        signature: "Some(3) refused: <secret#1>".into(),
        fingerprint: "fp".into(),
    };
    // What an older binary wrote: the bare `id@fingerprint` key and a
    // fingerprint taken from masked text.
    let path = cache_path(run.path(), commit);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        serde_json::to_vec(&BTreeMap::from([(
            "AC-X-001@fp".to_string(),
            Verdict::Failed("Some(3) refused: [REDACTED]".into()),
        )]))
        .unwrap(),
    )
    .unwrap();
    let observer = Counting(Mutex::new(0));
    let mut observations = Observations::new(run.path(), &observer, SearchBudget::default());
    let seen = observations
        .at(commit, &[(check.id.clone(), check.cache_key())])
        .await
        .unwrap();
    assert_eq!(
        *observer.0.lock().unwrap(),
        1,
        "the old entry is not reused"
    );
    assert_eq!(
        seen["AC-X-001"],
        Verdict::Failed("Some(3) refused: <secret#1>".into())
    );
    // The new entry is reused from then on.
    let again = observations
        .at(commit, &[(check.id.clone(), check.cache_key())])
        .await
        .unwrap();
    assert_eq!(*observer.0.lock().unwrap(), 1);
    assert_eq!(again, seen);
}

/// Review r8: placeholder identities survive the comparator's normalization:
/// distinct secrets stay distinct failures, the same secret the same one.
#[test]
fn numbered_placeholders_stay_distinct_through_fails_as() {
    let first = Verdict::Failed("Some(3) refused: <secret#1> after 12 tries".into());
    assert!(!first.fails_as("Some(3) refused: <secret#2> after 12 tries"));
    assert!(first.fails_as("Some(3) refused: <secret#1> after 13 tries"));
    assert!(
        Verdict::Failed("Some(3) <secret#10>".into()).fails_as("Some(3) <secret#10>")
            && !Verdict::Failed("Some(3) <secret#10>".into()).fails_as("Some(3) <secret#1>")
    );
}
