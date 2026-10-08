use super::*;

fn config() -> CozoGuardConfig {
    CozoGuardConfig {
        max_attempts: 2,
        initial_backoff: Duration::ZERO,
        max_backoff: Duration::ZERO,
        ..Default::default()
    }
}
fn progressing_read(mutability: ScriptMutability) {
    let mut calls = 0;
    let result = run_guarded("progressing peer", mutability, &config(), || {
        calls += 1;
        if calls <= 30 {
            Err(anyhow!("database is locked (code 5)"))
        } else {
            Ok(42)
        }
    });
    assert_eq!(result.unwrap(), 42);
    assert_eq!(calls, 31, "contention must never cap total attempts");
}
#[test]
fn progressing_sync_reader_has_no_total_attempt_limit() {
    progressing_read(ScriptMutability::Immutable);
}
#[test]
fn progressing_writer_has_no_total_attempt_limit() {
    progressing_read(ScriptMutability::Mutable);
}
#[tokio::test]
async fn progressing_async_reader_has_no_total_attempt_limit() {
    let mut calls = 0;
    let result = run_guarded_async(
        "progressing peer",
        ScriptMutability::Immutable,
        &config(),
        move || {
            calls += 1;
            if calls <= 30 {
                Err(anyhow!("database is locked (code 5)"))
            } else {
                Ok(42)
            }
        },
    )
    .await;
    assert_eq!(result.unwrap(), 42);
}

#[derive(Debug)]
struct WrappedPause(StoreBusy);
impl std::fmt::Display for WrappedPause {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "wrapped acquisition pause")
    }
}
impl std::error::Error for WrappedPause {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}
fn wrapped() -> anyhow::Error {
    WrappedPause(StoreBusy {
        context: "wrapped".into(),
        attempts: 1,
        detail: "acquisition paused".into(),
    })
    .into()
}
fn assert_typed_pause(error: anyhow::Error) {
    // A synthesized busy error after a retry budget is not the original pause.
    assert!(error.chain().any(|source| source.is::<WrappedPause>()));
    assert!(
        error.chain().any(|s| s.is::<StoreBusy>()),
        "typed source must survive: {error}"
    );
}
#[test]
fn wrapped_pause_is_returned_without_retrying_the_body() {
    let mut calls = 0;
    let result = run_guarded("wrapped", ScriptMutability::Immutable, &config(), || {
        calls += 1;
        if calls == 1 { Err(wrapped()) } else { Ok(42) }
    });
    assert_typed_pause(result.expect_err("an explicit pause must not restart the body"));
    assert_eq!(calls, 1);
}
#[test]
fn context_around_wrapped_pause_is_returned_without_retrying() {
    let mut calls = 0;
    let result = run_guarded("wrapped", ScriptMutability::Immutable, &config(), || {
        calls += 1;
        if calls == 1 {
            Err(wrapped().context("outer context"))
        } else {
            Ok(42)
        }
    });
    assert_typed_pause(result.expect_err("an explicit pause must not restart the body"));
    assert_eq!(calls, 1);
}
#[tokio::test]
async fn async_wrapped_pause_is_returned_without_retrying() {
    let mut calls = 0;
    let result = run_guarded_async(
        "wrapped",
        ScriptMutability::Immutable,
        &config(),
        move || {
            calls += 1;
            if calls == 1 { Err(wrapped()) } else { Ok(42) }
        },
    )
    .await;
    assert_typed_pause(result.expect_err("an explicit pause must not restart the body"));
}
