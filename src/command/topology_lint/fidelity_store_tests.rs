//! The per-obligation store (#434): a saved verdict, true or false, stands
//! only over the same tasks with byte-identical texts; two standing verdicts
//! that disagree are kept as a contradiction, never resolved silently; and
//! concurrent saves never lose one.

use super::*;

fn identity() -> StoreIdentity {
    StoreIdentity {
        critic_logic: "critic-logic".into(),
        provider: Some("provider".into()),
        model: "model".into(),
        request: Some("representative-request".into()),
    }
}

fn store(dir: &Path) -> VerdictStore {
    VerdictStore::new(dir.join("verdicts"), identity())
}

fn obligation_with(text: &str) -> ClaimedObligation {
    ClaimedObligation {
        id: "OBL-1".into(),
        text: text.into(),
    }
}

const OBLIGES: &str = "The writer must store every entry in the shared registry it opens.";
const LOOPHOLE: &str = "Tests may use a temporary registry.";

fn tasks(first: &str, second: &str) -> Vec<ClaimingTask> {
    vec![
        ClaimingTask {
            task_id: "TASK-A-001".into(),
            text: first.into(),
        },
        ClaimingTask {
            task_id: "TASK-A-002".into(),
            text: second.into(),
        },
    ]
}

fn supported() -> FidelityVerdict {
    FidelityVerdict {
        obligation_id: "OBL-1".into(),
        necessarily_true: true,
        weakest_task_id: String::new(),
        reason: "the writer must store it".into(),
        quoted_task_text: String::new(),
        supporting_task_id: "TASK-A-001".into(),
        supporting_text: OBLIGES.into(),
    }
}

fn refuted() -> FidelityVerdict {
    FidelityVerdict {
        necessarily_true: false,
        weakest_task_id: "TASK-A-002".into(),
        reason: "a temporary registry is allowed".into(),
        quoted_task_text: LOOPHOLE.into(),
        supporting_task_id: String::new(),
        supporting_text: String::new(),
        ..supported()
    }
}

/// One question over `tasks`, with their text digests computed here.
struct Asked {
    obligation: ClaimedObligation,
    tasks: Vec<ClaimingTask>,
    texts: TaskTexts,
}

impl Asked {
    fn new(obligation: ClaimedObligation, tasks: Vec<ClaimingTask>) -> Self {
        let texts = task_texts(&tasks);
        Self {
            obligation,
            tasks,
            texts,
        }
    }

    fn question(&self) -> Question<'_> {
        Question {
            obligation: &self.obligation,
            skeleton: "skeleton",
            tasks: &self.tasks,
            texts: &self.texts,
        }
    }
}

fn records(store: &VerdictStore) -> usize {
    std::fs::read_dir(store.dir())
        .unwrap()
        .filter(|entry| {
            entry
                .as_ref()
                .unwrap()
                .path()
                .extension()
                .is_some_and(|ext| ext == "json")
        })
        .count()
}

#[test]
fn each_envelope_is_its_own_identity_and_unknown_envelopes_never_reuse() {
    let temp = tempfile::tempdir().unwrap();
    let store = store(temp.path());
    let asked = Asked::new(
        obligation_with("The registry holds every entry."),
        tasks(OBLIGES, "Second task."),
    );
    let first = store.for_request(Some("rendered-template-one".into()));
    let (standing, saved) = first.save(asked.question(), &supported()).unwrap();
    assert!(saved);
    assert_eq!(standing, Standing::Verdict(supported()));
    assert_eq!(
        first.load(asked.question()),
        Some(Standing::Verdict(supported()))
    );
    let changed = store.for_request(Some("rendered-template-two".into()));
    assert_eq!(changed.load(asked.question()), None);
    let unknown = store.for_request(None);
    assert_eq!(unknown.load(asked.question()), None);
    assert!(!unknown.save(asked.question(), &supported()).unwrap().1);
    let rebuilt = VerdictStore::new(store.dir().into(), identity().with_critic_logic("other"));
    assert_eq!(
        rebuilt.load(asked.question()),
        None,
        "other critic logic re-asks"
    );
    assert_eq!(records(&store), 1);
}

#[test]
fn a_changed_critic_logic_digest_reasks_new_schema_records() {
    let temp = tempfile::tempdir().unwrap();
    let original = store(temp.path());
    let asked = Asked::new(
        obligation_with("The registry holds every entry."),
        tasks(OBLIGES, "Second task."),
    );
    original.save(asked.question(), &supported()).unwrap();
    let changed = VerdictStore::new(
        original.dir().into(),
        identity().with_critic_logic("changed-critic-logic"),
    );
    assert_eq!(changed.load(asked.question()), None);
}

/// A true verdict stands only over the same tasks with byte-identical texts:
/// any edit to any of them, a task joining or leaving the cluster, or a
/// changed obligation text re-asks it — its quote being still present is
/// not enough.
#[test]
fn a_true_verdict_stands_only_over_the_identical_cluster() {
    let temp = tempfile::tempdir().unwrap();
    let store = store(temp.path());
    let obligation = obligation_with("The registry holds every entry.");
    let original = Asked::new(
        obligation.clone(),
        tasks(&format!("# A\n\n{OBLIGES}\n"), "Second task."),
    );
    store.save(original.question(), &supported()).unwrap();
    let again = Asked::new(
        obligation.clone(),
        tasks(&format!("# A\n\n{OBLIGES}\n"), "Second task."),
    );
    assert_eq!(
        store.load(again.question()),
        Some(Standing::Verdict(supported())),
        "byte-identical cluster: reused"
    );
    let qualified = Asked::new(
        obligation.clone(),
        tasks(
            &format!("# A\n\n{OBLIGES}\nThis applies only when the registry exists.\n"),
            "Second task.",
        ),
    );
    assert_eq!(
        store.load(qualified.question()),
        None,
        "an edit beside the quote"
    );
    let loophole = Asked::new(
        obligation.clone(),
        tasks(&format!("# A\n\n{OBLIGES}\n"), LOOPHOLE),
    );
    assert_eq!(
        store.load(loophole.question()),
        None,
        "another claimer's edit"
    );
    let mut joined = tasks(&format!("# A\n\n{OBLIGES}\n"), "Second task.");
    joined.push(ClaimingTask {
        task_id: "TASK-A-003".into(),
        text: LOOPHOLE.into(),
    });
    let joined = Asked::new(obligation.clone(), joined);
    assert_eq!(
        store.load(joined.question()),
        None,
        "a claimer joining the cluster"
    );
    let reworded = Asked::new(
        obligation_with("The registry holds every entry, signed."),
        original.tasks.clone(),
    );
    assert_eq!(
        store.load(reworded.question()),
        None,
        "a changed obligation text"
    );
}

#[test]
fn a_false_verdict_stands_only_over_the_identical_cluster() {
    let temp = tempfile::tempdir().unwrap();
    let store = store(temp.path());
    let obligation = obligation_with("The registry holds every entry.");
    let cluster = Asked::new(obligation.clone(), tasks("Writes entries.", LOOPHOLE));
    store.save(cluster.question(), &refuted()).unwrap();
    assert_eq!(
        store.load(cluster.question()),
        Some(Standing::Verdict(refuted()))
    );
    let edited = Asked::new(obligation, tasks("Writes every entry.", LOOPHOLE));
    assert_eq!(
        store.load(edited.question()),
        None,
        "another claimer's edit can close the loophole, so it is asked again"
    );
}

/// A fresh answer that disagrees with a standing verdict of the other
/// meaning is a contradiction, in both directions; neither wins once either
/// stops standing — the obligation is asked again.
#[test]
fn disagreeing_standing_verdicts_are_a_contradiction_until_asked_again() {
    let temp = tempfile::tempdir().unwrap();
    let store = store(temp.path());
    let obligation = obligation_with("The registry holds every entry.");
    let cluster = Asked::new(obligation.clone(), tasks(OBLIGES, LOOPHOLE));
    store.save(cluster.question(), &supported()).unwrap();
    let contradiction = Standing::Contradiction {
        refuted: refuted(),
        supported: supported(),
    };
    assert_eq!(
        store.save(cluster.question(), &refuted()).unwrap().0,
        contradiction
    );
    assert_eq!(store.load(cluster.question()), Some(contradiction));
    let reverse = Asked::new(
        obligation_with("Another obligation."),
        tasks(OBLIGES, LOOPHOLE),
    );
    store.save(reverse.question(), &refuted()).unwrap();
    assert!(matches!(
        store.save(reverse.question(), &supported()).unwrap().0,
        Standing::Contradiction { .. }
    ));
    // The texts change: the contradiction is asked again, never settled by
    // the survivor, and the fresh answer replaces both.
    let changed = Asked::new(
        obligation,
        tasks(OBLIGES, "Tests use the shared registry only."),
    );
    assert_eq!(store.load(changed.question()), None);
    assert_eq!(
        store.save(changed.question(), &supported()).unwrap().0,
        Standing::Verdict(supported())
    );
    assert_eq!(
        store.load(changed.question()),
        Some(Standing::Verdict(supported()))
    );
}

#[test]
fn a_verdict_that_fails_its_own_check_is_never_saved() {
    let temp = tempfile::tempdir().unwrap();
    let store = store(temp.path());
    let cluster = Asked::new(
        obligation_with("The registry holds every entry."),
        tasks(OBLIGES, LOOPHOLE),
    );
    let mut paraphrase = supported();
    paraphrase.supporting_text = "The writer stores entries in the registry it opens.".into();
    assert!(store.save(cluster.question(), &paraphrase).is_err());
    assert_eq!(store.load(cluster.question()), None);
}

/// Saves of one obligation from many threads at once, half true and half
/// false over the same texts: under the record lock no write is lost, so the
/// disagreement is always seen as a contradiction.
#[test]
fn concurrent_saves_never_lose_a_verdict() {
    for _ in 0..20 {
        let temp = tempfile::tempdir().unwrap();
        let store = std::sync::Arc::new(store(temp.path()));
        let cluster = std::sync::Arc::new(Asked::new(
            obligation_with("The registry holds every entry."),
            tasks(OBLIGES, LOOPHOLE),
        ));
        let threads: Vec<_> = (0..8)
            .map(|n| {
                let (store, cluster) = (store.clone(), cluster.clone());
                std::thread::spawn(move || {
                    let verdict = if n % 2 == 0 { supported() } else { refuted() };
                    store.save(cluster.question(), &verdict).unwrap();
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        assert!(matches!(
            store.load(cluster.question()),
            Some(Standing::Contradiction { .. })
        ));
    }
}

#[path = "fidelity_store_legacy_tests.rs"]
mod legacy_tests;

#[path = "fidelity_logic_digest_tests.rs"]
mod logic_digest_tests;
