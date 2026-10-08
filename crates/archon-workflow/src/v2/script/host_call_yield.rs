//! A host call hands the script thread back to its runtime (Issue 364).
//!
//! A workflow.js script, its host-call futures, the provider stream readers
//! and every timer of its calls share one current-thread tokio runtime. The
//! QuickJS context drives the script inside ONE poll of that runtime: it runs
//! JavaScript jobs and polls host-call futures for as long as either makes
//! progress, and only returns to tokio when neither does. A host call whose
//! future finishes on its first poll (a refusal, a replayed record, a dry-run
//! record) therefore never returns to tokio, and a script that loops over such
//! calls holds the thread: no timer fires, no socket is read, no no-progress
//! window can expire, and the process sits at one full core.
//!
//! Every host call ends with one deferred yield. tokio wakes a deferred task
//! only after it has polled its I/O and timer driver, so between two host
//! calls the runtime always gets one turn of timers and sockets.
//!
//! This is defense in depth, not the cause of the Issue 364 spin: the live
//! bridge already awaits a tokio mutex in every call, and that await spends
//! tokio's cooperative budget and returns to the driver every few dozen
//! calls. The test below drives a bare host function, which the live bridge
//! is not. JavaScript that starves the thread without calling the host (a
//! microtask loop while a call is in flight) is ended by the script thread's
//! heartbeat ([`super::script_thread_heartbeat`]).

/// Yield the script thread to its runtime's driver once.
pub async fn yield_to_script_runtime() {
    tokio::task::yield_now().await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use rquickjs::function::{Async, Func};
    use rquickjs::{AsyncContext, AsyncRuntime, Promise};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    /// Run a script that loops over instant host calls for `run_ms` of wall
    /// time, beside a 1 ms timer task on the same thread. Returns how many
    /// times the timer fired while the script ran.
    fn timer_ticks_during_instant_host_calls(yield_each_call: bool, run_ms: u64) -> u64 {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async move {
            let ticks = Arc::new(AtomicU64::new(0));
            let ticker = {
                let ticks = Arc::clone(&ticks);
                tokio::spawn(async move {
                    loop {
                        tokio::time::sleep(Duration::from_millis(1)).await;
                        ticks.fetch_add(1, Ordering::Relaxed);
                    }
                })
            };
            // Let the ticker register its first timer before the script runs.
            tokio::time::sleep(Duration::from_millis(5)).await;
            let before = ticks.load(Ordering::Relaxed);
            let js = AsyncRuntime::new().expect("quickjs runtime");
            let context = AsyncContext::full(&js).await.expect("quickjs context");
            let source = format!(
                "(async () => {{ const t0 = Date.now(); let n = 0; \
                 while (Date.now() - t0 < {run_ms}) {{ await host(); n += 1; }} return n; }})()"
            );
            let calls = context
                .async_with(async move |ctx| {
                    ctx.globals()
                        .set(
                            "host",
                            Func::from(Async(move || async move {
                                if yield_each_call {
                                    yield_to_script_runtime().await;
                                }
                                Ok::<_, rquickjs::Error>(1)
                            })),
                        )
                        .expect("install host");
                    let promise: Promise = ctx.eval(source.as_str()).expect("eval");
                    promise.into_future::<u64>().await.expect("script result")
                })
                .await;
            let during = ticks.load(Ordering::Relaxed) - before;
            ticker.abort();
            assert!(calls > 0, "the script made host calls");
            during
        })
    }

    #[test]
    fn instant_host_calls_leave_the_script_thread_timers_running() {
        let ticks = timer_ticks_during_instant_host_calls(true, 300);
        // A 1 ms timer on a free thread fires ~300 times in 300 ms (~19 on a
        // 15.6 ms Windows timer); a starved thread fires it at most once.
        assert!(
            ticks >= 5,
            "the runtime's timers fired only {ticks} times while the script called the host"
        );
    }

    #[test]
    fn without_the_yield_instant_host_calls_starve_the_script_thread() {
        // The mechanism this module exists for: the QuickJS drive loop never
        // returns to tokio while each call finishes on its first poll.
        let ticks = timer_ticks_during_instant_host_calls(false, 300);
        assert!(
            ticks <= 1,
            "expected starvation without the yield, saw {ticks} timer ticks"
        );
    }
}
