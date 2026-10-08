use super::*;
use rquickjs::function::{Async, Func};
use rquickjs::{AsyncContext, AsyncRuntime, Promise};
use std::sync::atomic::AtomicUsize;

const WINDOW: Duration = Duration::from_millis(300);

struct Outcome {
    settled: Result<String, String>,
    cut: Option<Starvation>,
    records: Vec<Starvation>,
}

/// Runs `source` on its own script thread, as the host does, with only the
/// heartbeat as the interrupt handler: the CPU watchdog is paused, as it is
/// while a host call is in flight. `host()` is a host call that takes
/// `host_ms`. `None` when the script did not end within 20 s.
fn run_script(source: &'static str, host_ms: u64) -> Option<Outcome> {
    let (done, outcome) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let result = runtime.block_on(async move {
            let records = Arc::new(Mutex::new(Vec::new()));
            let in_flight = Arc::new(AtomicUsize::new(0));
            let heartbeat = {
                let (records, in_flight) = (Arc::clone(&records), Arc::clone(&in_flight));
                ScriptThreadHeartbeat::new(
                    WINDOW,
                    Box::new(move || {
                        let calls = in_flight.load(Ordering::SeqCst);
                        (0..calls)
                            .map(|_| serde_json::json!({ "id": "slow-host-call" }))
                            .collect()
                    }),
                    Box::new(move |record| records.lock().unwrap().push(record.clone())),
                )
            };
            let _ticker = heartbeat.spawn_ticker(WINDOW / 10);
            let _monitor = heartbeat.spawn_monitor(WINDOW / 8);
            let js = AsyncRuntime::new().expect("quickjs runtime");
            let for_interrupt = heartbeat.clone();
            js.set_interrupt_handler(Some(Box::new(move || for_interrupt.should_cut())))
                .await;
            let context = AsyncContext::full(&js).await.expect("quickjs context");
            let (for_host, for_wait) = (heartbeat.clone(), heartbeat.clone());
            let settled = context
                .async_with(async move |ctx| {
                    ctx.globals()
                        .set(
                            "host",
                            Func::from(Async(move || {
                                let (heartbeat, in_flight) =
                                    (for_host.clone(), Arc::clone(&in_flight));
                                in_flight.fetch_add(1, Ordering::SeqCst);
                                async move {
                                    heartbeat.beat();
                                    tokio::time::sleep(Duration::from_millis(host_ms)).await;
                                    in_flight.fetch_sub(1, Ordering::SeqCst);
                                    heartbeat.beat();
                                    Ok::<_, rquickjs::Error>(1)
                                }
                            })),
                        )
                        .expect("install host");
                    let promise: Promise = ctx.eval(source).map_err(|err| err.to_string())?;
                    tokio::select! {
                        biased;
                        settled = promise.into_future::<String>() => settled.map_err(|err| err.to_string()),
                        () = for_wait.cut_signal() => Err("cut".to_string()),
                    }
                })
                .await;
            let records = records.lock().unwrap().clone();
            Outcome {
                settled,
                cut: heartbeat.cut(),
                records,
            }
        });
        let _ = done.send(result);
    });
    outcome.recv_timeout(Duration::from_secs(20)).ok()
}

/// Fails before the fix (it never ends: the in-flight call is never polled
/// again, and nothing interrupts the job loop while the watchdog is paused).
#[test]
fn a_microtask_loop_during_an_in_flight_host_call_is_cut_and_named() {
    let outcome = run_script(
        "(async () => { const slow = host(); for (;;) { await null; } })()",
        10_000,
    )
    .expect("the starved script must end");
    assert!(outcome.settled.is_err(), "{:?}", outcome.settled);
    let cut = outcome.cut.expect("the starvation is recorded as a cut");
    assert_eq!(cut.detected_by, "interrupt_handler");
    assert_eq!(cut.state, "cut");
    assert_eq!(
        cut.in_flight,
        vec![serde_json::json!({ "id": "slow-host-call" })]
    );
    assert!(cut.no_progress_ms >= cut.window_ms, "{cut:?}");
}

/// Fails before the fix, the same way: a synchronous loop with a call in flight.
#[test]
fn a_pure_busy_loop_during_an_in_flight_host_call_is_cut() {
    let outcome = run_script(
        "(async () => { const slow = host(); for (;;) {} })()",
        10_000,
    )
    .expect("the starved script must end");
    assert!(outcome.settled.is_err(), "{:?}", outcome.settled);
    let cut = outcome.cut.expect("cut");
    assert_eq!(cut.in_flight.len(), 1);
}

/// A host call far longer than the window, with the runtime idle in it, is
/// never cut: the ticker beats on time.
#[test]
fn a_healthy_long_host_call_is_never_cut() {
    let outcome = run_script(
        "(async () => { await host(); return 'done'; })()",
        WINDOW.as_millis() as u64 * 4,
    )
    .expect("the script ends");
    assert_eq!(outcome.settled, Ok("done".to_string()));
    assert!(outcome.cut.is_none());
    assert!(outcome.records.is_empty(), "{:?}", outcome.records);
}

/// Time off the CPU (a loaded machine, a sleep) never cuts.
#[test]
fn a_thread_off_the_cpu_is_not_starved() {
    let heartbeat = ScriptThreadHeartbeat::new(WINDOW, Box::new(Vec::new), Box::new(|_| {}));
    std::thread::sleep(WINDOW * 2);
    if thread_cpu_time().is_some() {
        assert!(!heartbeat.should_cut());
    }
}

/// A thread that burns CPU without a beat (as a synchronous loop inside a
/// host future does, where no interrupt can reach) is recorded by the
/// monitor, and the recovery too.
#[test]
fn the_monitor_records_a_starved_thread_and_its_recovery() {
    if process_cpu_time().is_none() {
        return;
    }
    let records = Arc::new(Mutex::new(Vec::<Starvation>::new()));
    let sink = Arc::clone(&records);
    let heartbeat = ScriptThreadHeartbeat::new(
        WINDOW,
        Box::new(|| vec![serde_json::json!({ "id": "spinning-call" })]),
        Box::new(move |record| sink.lock().unwrap().push(record.clone())),
    );
    let _monitor = heartbeat.spawn_monitor(WINDOW / 8);
    let started = Instant::now();
    let mut spins: u64 = 0;
    while records.lock().unwrap().is_empty() && started.elapsed() < Duration::from_secs(10) {
        spins = std::hint::black_box(spins.wrapping_add(1));
    }
    let suspected = records
        .lock()
        .unwrap()
        .first()
        .cloned()
        .expect("the monitor recorded no suspected starvation within 10 s of spinning");
    assert_eq!(suspected.detected_by, "monitor");
    assert_eq!(suspected.state, "suspected");
    assert_eq!(suspected.in_flight[0]["id"], "spinning-call");
    assert!(suspected.no_progress_ms >= suspected.window_ms);

    heartbeat.beat();
    // The monitor thread records the recovery on its next poll; a loaded
    // machine can delay that poll, so wait for it rather than for a fixed time.
    let recovered_by = Instant::now() + Duration::from_secs(10);
    let last = loop {
        let last = records.lock().unwrap().last().cloned();
        let recovered = last
            .as_ref()
            .is_some_and(|record| record.state == "recovered");
        if recovered || Instant::now() >= recovered_by {
            break last;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let last = last.expect("records vanished after the suspected one");
    assert_eq!(
        last.state, "recovered",
        "the monitor recorded no recovery within 10 s of the beat; last record: {last:?}"
    );
    assert!(!heartbeat.should_cut());
    assert!(heartbeat.cut().is_none(), "a suspicion alone never cuts");
}
