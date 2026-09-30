//! Host-owned per-call turn bound (Issue-213 C2c); never parsed from an agent
//! tool request.
//!
//! `SubagentRequest::DEFAULT_MAX_TURNS` is effectively unlimited, and three
//! callers read it, so lowering it would change the ceiling for the whole
//! product. A workflow host instead bounds the one call it dispatches: it sets
//! this scope around the dispatch, and the workflow session builder reads it
//! in place of the default. Outside a scope nothing changes.

tokio::task_local! { static OVERRIDE: u32; }

/// The bound the enclosing host call set, if any.
pub fn current() -> Option<u32> {
    OVERRIDE.try_with(|value| *value).ok()
}

/// Run `work` bounded to `max_turns` turns. A bound already in effect is only
/// ever narrowed: a nested scope cannot raise what its caller set.
pub async fn scope<T>(max_turns: u32, work: impl std::future::Future<Output = T>) -> T {
    let bound = current().map_or(max_turns, |outer| outer.min(max_turns));
    OVERRIDE.scope(bound.max(1), work).await
}

/// `work` under `max_turns` when there is one, unchanged otherwise.
pub async fn inherit<T>(max_turns: Option<u32>, work: impl std::future::Future<Output = T>) -> T {
    match max_turns {
        Some(max_turns) => scope(max_turns, work).await,
        None => work.await,
    }
}

/// The turn bound a session built now should run under: the host's bound,
/// clamped to `[1, hard_cap]`, or `default` outside any host scope.
pub fn resolve(default: u32, hard_cap: u32) -> u32 {
    current().map_or(default, |bound| bound.clamp(1, hard_cap.max(1)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn outside_a_scope_the_default_stands() {
        assert_eq!(current(), None);
        assert_eq!(resolve(100_000, 100_000), 100_000);
    }

    #[tokio::test]
    async fn a_scope_bounds_and_a_nested_scope_only_narrows() {
        let seen = scope(40, async {
            let inner = scope(500, async { resolve(100_000, 100_000) }).await;
            (resolve(100_000, 100_000), inner)
        })
        .await;
        assert_eq!(seen, (40, 40));
        assert_eq!(scope(0, async { resolve(9, 9) }).await, 1);
        assert_eq!(scope(7, async { resolve(9, 5) }).await, 5);
    }
}
