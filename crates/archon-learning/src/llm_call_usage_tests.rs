use std::sync::{Arc, Barrier};

use tempfile::tempdir;

use crate::cozo_guard::test_sqlite_db;
use crate::llm_call_usage::{
    InsertLlmCallUsageOutcome, LlmCallUsageRecord, LlmCallUsageScope, UsageAvailability,
    insert_llm_call_usage, list_llm_call_usage,
};

fn usage(request_id: &str) -> LlmCallUsageRecord {
    LlmCallUsageRecord {
        request_id: request_id.into(),
        run_id: Some("run-1".into()),
        session_id: Some("session-1".into()),
        turn: Some(2),
        round: Some(3),
        role: Some("assistant".into()),
        origin: Some("main_session".into()),
        provider_id: "anthropic".into(),
        model_id: "claude-sonnet-4-6".into(),
        input_tokens: UsageAvailability::Known(11),
        output_tokens: UsageAvailability::Known(7),
        cache_creation_input_tokens: UsageAvailability::Known(0),
        cache_read_input_tokens: UsageAvailability::Unavailable,
        context_input_tokens: Some(11),
        effective_denominator: Some(100),
        terminal_status: "succeeded".into(),
        created_at: "2026-07-24T12:00:00Z".into(),
    }
}

#[test]
fn usage_roundtrips_with_zero_distinct_from_unavailable() {
    let db = test_sqlite_db("llm-call-usage-roundtrip");
    insert_llm_call_usage(&db, &usage("request-1")).unwrap();

    let rows = list_llm_call_usage(
        &db,
        &LlmCallUsageScope::new(Some("run-1"), Some("session-1")),
    )
    .unwrap();

    assert_eq!(rows, vec![usage("request-1")]);
    assert_eq!(rows[0].effective_burn(), Some(18));
}

#[test]
fn usage_query_is_scoped_to_run_and_session() {
    let db = test_sqlite_db("llm-call-usage-scope");
    insert_llm_call_usage(&db, &usage("request-1")).unwrap();
    let mut other = usage("request-2");
    other.run_id = Some("run-2".into());
    other.session_id = Some("session-2".into());
    insert_llm_call_usage(&db, &other).unwrap();

    let rows = list_llm_call_usage(
        &db,
        &LlmCallUsageScope::new(Some("run-1"), Some("session-1")),
    )
    .unwrap();

    assert_eq!(rows, vec![usage("request-1")]);
}

#[test]
fn same_row_reuse_is_atomic_but_conflicting_reuse_fails() {
    let db = Arc::new(test_sqlite_db("llm-call-usage-conflict"));
    let first = usage("request-1");
    let workers: Vec<_> = (0..8)
        .map(|_| {
            let db = Arc::clone(&db);
            let first = first.clone();
            std::thread::spawn(move || insert_llm_call_usage(&db, &first).map(|_| ()))
        })
        .collect();
    for worker in workers {
        worker.join().unwrap().unwrap();
    }

    let mut conflict = first.clone();
    conflict.output_tokens = UsageAvailability::Known(8);
    assert_eq!(
        insert_llm_call_usage(&db, &conflict).unwrap(),
        InsertLlmCallUsageOutcome::Conflict
    );
}

#[test]
fn concurrent_different_rows_are_all_persisted() {
    let db = Arc::new(test_sqlite_db("llm-call-usage-concurrent-different"));
    let workers: Vec<_> = (0..8)
        .map(|index| {
            let db = Arc::clone(&db);
            std::thread::spawn(move || {
                insert_llm_call_usage(&db, &usage(&format!("request-{index}"))).map(|_| ())
            })
        })
        .collect();
    for worker in workers {
        worker.join().unwrap().unwrap();
    }

    let rows = list_llm_call_usage(
        &db,
        &LlmCallUsageScope::new(Some("run-1"), Some("session-1")),
    )
    .unwrap();
    assert_eq!(rows.len(), 8);
}
#[test]
fn oversized_u64_values_are_rejected_before_cozo_write() {
    let db = test_sqlite_db("llm-call-usage-overflow");
    let mut record = usage("request-overflow");
    record.input_tokens = UsageAvailability::Known(i64::MAX as u64 + 1);

    let error = insert_llm_call_usage(&db, &record).unwrap_err();

    assert!(
        error
            .to_string()
            .contains("input_tokens exceeds Cozo Int range")
    );
}

#[test]
fn independent_sqlite_handles_classify_identical_request_id_writes() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("usage.db");
    let first =
        crate::cozo_guard::open_sqlite_guarded(path.to_str().unwrap(), "open first").unwrap();
    let second =
        crate::cozo_guard::open_sqlite_guarded(path.to_str().unwrap(), "open second").unwrap();
    crate::schema::ensure_learning_schema(&first).unwrap();
    crate::schema::ensure_learning_schema(&second).unwrap();
    let barrier = Arc::new(Barrier::new(3));
    let record = usage("shared-request");
    let workers: Vec<_> = [first, second]
        .into_iter()
        .map(|db| {
            let barrier = Arc::clone(&barrier);
            let record = record.clone();
            std::thread::spawn(move || {
                barrier.wait();
                insert_llm_call_usage(&db, &record).unwrap()
            })
        })
        .collect();
    barrier.wait();
    let outcomes: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect();

    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| **outcome == InsertLlmCallUsageOutcome::Created)
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| **outcome == InsertLlmCallUsageOutcome::Reused)
            .count(),
        1
    );
}

#[test]
fn independent_sqlite_handles_classify_conflicting_request_id_writes() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("usage.db");
    let first =
        crate::cozo_guard::open_sqlite_guarded(path.to_str().unwrap(), "open first").unwrap();
    let second =
        crate::cozo_guard::open_sqlite_guarded(path.to_str().unwrap(), "open second").unwrap();
    crate::schema::ensure_learning_schema(&first).unwrap();
    crate::schema::ensure_learning_schema(&second).unwrap();
    let barrier = Arc::new(Barrier::new(3));
    let mut conflict = usage("shared-request");
    conflict.output_tokens = UsageAvailability::Known(99);
    let workers: Vec<_> = [usage("shared-request"), conflict]
        .into_iter()
        .zip([first, second])
        .map(|(record, db)| {
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                insert_llm_call_usage(&db, &record).unwrap()
            })
        })
        .collect();
    barrier.wait();
    let outcomes: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect();

    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| **outcome == InsertLlmCallUsageOutcome::Created)
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| **outcome == InsertLlmCallUsageOutcome::Conflict)
            .count(),
        1
    );
}

#[test]
fn max_i64_counts_rounds_and_denominators_roundtrip() {
    let db = test_sqlite_db("llm-call-usage-i64-max");
    let mut record = usage("request-max");
    let max = i64::MAX as u64;
    record.turn = Some(max);
    record.round = Some(max);
    record.input_tokens = UsageAvailability::Known(max);
    record.output_tokens = UsageAvailability::Known(max);
    record.cache_creation_input_tokens = UsageAvailability::Known(max);
    record.cache_read_input_tokens = UsageAvailability::Known(max);
    record.context_input_tokens = Some(max);
    record.effective_denominator = Some(max);

    assert_eq!(
        insert_llm_call_usage(&db, &record).unwrap(),
        InsertLlmCallUsageOutcome::Created
    );
    assert_eq!(
        list_llm_call_usage(
            &db,
            &LlmCallUsageScope::new(Some("run-1"), Some("session-1"))
        )
        .unwrap(),
        vec![record]
    );
}

#[test]
fn oversized_persisted_numbers_reject_without_mutating_the_ledger() {
    let db = test_sqlite_db("llm-call-usage-overflow-fields");
    let overflow = i64::MAX as u64 + 1;
    for (name, mutate) in [
        (
            "turn",
            Box::new(|record: &mut LlmCallUsageRecord| record.turn = Some(overflow))
                as Box<dyn Fn(&mut LlmCallUsageRecord)>,
        ),
        (
            "round",
            Box::new(|record: &mut LlmCallUsageRecord| record.round = Some(overflow)),
        ),
        (
            "output_tokens",
            Box::new(|record: &mut LlmCallUsageRecord| {
                record.output_tokens = UsageAvailability::Known(overflow)
            }),
        ),
        (
            "cache_creation",
            Box::new(|record: &mut LlmCallUsageRecord| {
                record.cache_creation_input_tokens = UsageAvailability::Known(overflow)
            }),
        ),
        (
            "cache_read",
            Box::new(|record: &mut LlmCallUsageRecord| {
                record.cache_read_input_tokens = UsageAvailability::Known(overflow)
            }),
        ),
        (
            "context",
            Box::new(|record: &mut LlmCallUsageRecord| {
                record.context_input_tokens = Some(overflow)
            }),
        ),
        (
            "denominator",
            Box::new(|record: &mut LlmCallUsageRecord| {
                record.effective_denominator = Some(overflow)
            }),
        ),
    ] {
        let mut record = usage(&format!("request-overflow-{name}"));
        mutate(&mut record);
        assert!(
            insert_llm_call_usage(&db, &record).is_err(),
            "{name} must reject overflow"
        );
    }
    assert!(
        list_llm_call_usage(&db, &LlmCallUsageScope::default())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn scope_requires_matching_nonempty_run_and_session_and_preserves_zero_denominator() {
    let db = test_sqlite_db("llm-call-usage-scoped-query");
    let matching = usage("matching");
    let mut missing = usage("missing");
    missing.run_id = None;
    missing.session_id = None;
    missing.effective_denominator = Some(0);
    let mut mismatched = usage("mismatched");
    mismatched.session_id = Some("other-session".into());
    mismatched.input_tokens = UsageAvailability::Unavailable;
    for record in [&matching, &missing, &mismatched] {
        insert_llm_call_usage(&db, record).unwrap();
    }

    let rows = list_llm_call_usage(
        &db,
        &LlmCallUsageScope::new(Some("run-1"), Some("session-1")),
    )
    .unwrap();
    assert_eq!(rows, vec![matching]);
    assert_eq!(rows[0].effective_burn(), Some(18));
    assert!(
        LlmCallUsageRecord {
            input_tokens: UsageAvailability::Unavailable,
            ..rows[0].clone()
        }
        .effective_burn()
        .is_none()
    );
    assert_eq!(missing.effective_denominator, Some(0));
}
