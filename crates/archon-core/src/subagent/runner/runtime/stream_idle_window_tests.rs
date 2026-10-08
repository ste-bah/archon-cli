use super::*;

/// Jumps this thread's clocks as a sleep of `slept` does: wall and boot.
fn sleep_for(slept: Duration) {
    WALL_JUMP.with(|jump| jump.set(slept));
    BOOT_JUMP.with(|jump| jump.set(slept));
}

fn reset_clocks() {
    sleep_for(Duration::ZERO);
    NO_BOOT_CLOCK.with(|none| none.set(false));
}

#[tokio::test]
async fn work_that_finishes_inside_the_window_is_returned() {
    let out = within(Duration::from_secs(5), async { 7 }).await;
    assert_eq!(out, Ok(7));
}

#[tokio::test]
async fn the_monotonic_clock_still_ends_a_silent_window() {
    let started = std::time::Instant::now();
    let out = within(Duration::from_millis(60), std::future::pending::<()>()).await;
    assert_eq!(out, Err(IdleExpired { slept: false }));
    assert!(started.elapsed() >= Duration::from_millis(60));
}

#[tokio::test]
async fn a_sleep_ends_the_window_soon_after_the_wake() {
    // A provider stream that went silent: open, never sending.
    let (_tx, mut rx) = tokio::sync::mpsc::channel::<u8>(1);
    let jump = async {
        tokio::time::sleep(Duration::from_millis(30)).await;
        sleep_for(Duration::from_secs(7 * 3600));
    };
    let started = std::time::Instant::now();
    let (out, ()) = tokio::join!(within(Duration::from_secs(3600), rx.recv()), jump);
    reset_clocks();
    assert_eq!(out, Err(IdleExpired { slept: true }));
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "the window must end within a re-check of the wake, took {:?}",
        started.elapsed()
    );
}

/// Fails before the fix: the wall clock alone ended the window, so a step
/// forward cut a stream that was still within its limit.
#[tokio::test]
async fn a_forward_step_of_the_wall_clock_does_not_cut_a_healthy_stream() {
    let step = async {
        tokio::time::sleep(Duration::from_millis(30)).await;
        WALL_JUMP.with(|jump| jump.set(Duration::from_secs(7 * 3600)));
    };
    let (out, ()) = tokio::join!(
        within(Duration::from_secs(3600), async {
            tokio::time::sleep(Duration::from_millis(300)).await;
            1
        }),
        step
    );
    reset_clocks();
    assert_eq!(out, Ok(1), "a clock step is not a sleep");
}

/// Jumps this thread's wall clock 7 h forward after `after`.
async fn step_wall_clock_after(after: Duration) {
    tokio::time::sleep(after).await;
    WALL_JUMP.with(|jump| jump.set(Duration::from_secs(7 * 3600)));
}

#[tokio::test]
async fn without_a_boot_clock_a_wall_jump_ends_the_window_only_after_the_floor() {
    NO_BOOT_CLOCK.with(|none| none.set(true));
    // Work shorter than the floor survives the jump.
    let (short, ()) = tokio::join!(
        within(Duration::from_secs(3600), async {
            tokio::time::sleep(STEP_FLOOR / 3).await;
            2
        }),
        step_wall_clock_after(Duration::from_millis(10))
    );
    WALL_JUMP.with(|jump| jump.set(Duration::ZERO));
    // A silent stream still ends soon after the floor.
    let started = std::time::Instant::now();
    let (silent, ()) = tokio::join!(
        within(Duration::from_secs(3600), std::future::pending::<()>()),
        step_wall_clock_after(Duration::from_millis(10))
    );
    let took = started.elapsed();
    reset_clocks();
    assert_eq!(short, Ok(2));
    assert_eq!(silent, Err(IdleExpired { slept: true }));
    assert!(
        took >= STEP_FLOOR && took < Duration::from_secs(2),
        "{took:?}"
    );
}

#[tokio::test]
async fn a_wall_clock_set_backwards_does_not_end_the_window() {
    WALL_JUMP.with(|jump| jump.set(Duration::from_secs(3600)));
    let set_back = async {
        tokio::time::sleep(Duration::from_millis(30)).await;
        WALL_JUMP.with(|jump| jump.set(Duration::ZERO));
    };
    let (out, ()) = tokio::join!(
        within(Duration::from_millis(200), async {
            tokio::time::sleep(Duration::from_millis(100)).await;
            1
        }),
        set_back
    );
    reset_clocks();
    assert_eq!(out, Ok(1));
}

/// Fails before the fix: Apple's `CLOCK_MONOTONIC` is the wall clock minus the
/// boot time, seconds away from the continuous (sleep-counting) clock, so it
/// could not tell a step of the wall clock from a sleep.
#[cfg(target_vendor = "apple")]
#[test]
fn the_apple_boot_clock_is_the_continuous_clock() {
    #[repr(C)]
    struct Timebase {
        numer: u32,
        denom: u32,
    }
    unsafe extern "C" {
        fn mach_continuous_time() -> u64;
        fn mach_timebase_info(info: *mut Timebase) -> i32;
    }
    let mut timebase = Timebase { numer: 0, denom: 0 };
    // SAFETY: `timebase` is a valid, writable struct of the C layout.
    assert_eq!(unsafe { mach_timebase_info(&mut timebase) }, 0);
    let boot = read_boot_clock().expect("a boot clock on Apple platforms");
    // SAFETY: no arguments; it cannot fail.
    let ticks = unsafe { mach_continuous_time() } as u128;
    let continuous = Duration::from_nanos(
        (ticks * u128::from(timebase.numer) / u128::from(timebase.denom)) as u64,
    );
    let apart = boot.abs_diff(continuous);
    assert!(
        apart < Duration::from_millis(250),
        "boot {boot:?} vs continuous {continuous:?}"
    );
}
