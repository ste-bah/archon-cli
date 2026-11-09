//! Scoped interruption handling: restore prior handlers after check teardown.
use std::io;
use std::sync::Mutex;
use std::sync::atomic::{AtomicI32, AtomicUsize, Ordering};

// The runtime break-event value is absent from the dependency constants.
#[cfg(windows)]
const BREAK_SIGNAL: i32 = 21;
#[cfg(windows)]
const SIGNALS: [i32; 3] = [libc::SIGINT, libc::SIGTERM, BREAK_SIGNAL];
#[cfg(unix)]
const SIGNALS: [i32; 4] = [libc::SIGINT, libc::SIGTERM, libc::SIGHUP, libc::SIGQUIT];
static PENDING: AtomicI32 = AtomicI32::new(0);
static INTERRUPTIONS: AtomicUsize = AtomicUsize::new(0);
static STATE: Mutex<State> = Mutex::new(State {
    users: 0,
    generation: 0,
    previous: None,
});

#[cfg(unix)]
type Handler = libc::sigaction;
#[cfg(windows)]
type Handler = libc::sighandler_t;

struct State {
    users: usize,
    generation: usize,
    previous: Option<[Handler; SIGNALS.len()]>,
}

extern "C" fn interrupted(signal: i32) {
    PENDING.store(signal, Ordering::SeqCst);
    #[cfg(windows)]
    {
        // The runtime resets its handler on delivery; rearm the same signal.
        // SAFETY: the callback has the required signature and static lifetime.
        let _ = unsafe { libc::signal(signal, interrupted as *const () as Handler) };
    }
    INTERRUPTIONS.fetch_add(1, Ordering::SeqCst);
}

fn action() -> Handler {
    #[cfg(unix)]
    {
        // SAFETY: all-zero is a valid initial action; initialize its mask and
        // callback explicitly before passing it to the operating system.
        let mut action: Handler = unsafe { std::mem::zeroed() };
        action.sa_sigaction = interrupted as *const () as usize;
        action.sa_flags = libc::SA_RESTART;
        // SAFETY: the mask is writable and belongs to this local action.
        unsafe { libc::sigemptyset(&mut action.sa_mask) };
        action
    }
    #[cfg(windows)]
    {
        interrupted as *const () as Handler
    }
}

fn replace(signal: i32, action: &Handler) -> io::Result<Handler> {
    #[cfg(unix)]
    {
        // SAFETY: both actions are valid, live structures; the signal is one
        // of the supported termination signals, never an uncatchable signal.
        let mut previous = unsafe { std::mem::zeroed() };
        if unsafe { libc::sigaction(signal, action, &mut previous) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(previous)
    }
    #[cfg(windows)]
    {
        // SAFETY: the handler was obtained from the runtime or from action().
        let previous = unsafe { libc::signal(signal, *action) };
        if previous == libc::SIG_ERR as libc::sighandler_t {
            return Err(io::Error::last_os_error());
        }
        Ok(previous)
    }
}

fn disposition(handler: &Handler) -> usize {
    #[cfg(unix)]
    {
        handler.sa_sigaction
    }
    #[cfg(windows)]
    {
        *handler
    }
}

fn register(signal: i32) -> io::Result<Handler> {
    #[cfg(unix)]
    {
        // SAFETY: a null new action queries the disposition without changing it.
        let mut previous: Handler = unsafe { std::mem::zeroed() };
        if unsafe { libc::sigaction(signal, std::ptr::null(), &mut previous) } != 0 {
            return Err(io::Error::last_os_error());
        }
        if disposition(&previous) == libc::SIG_IGN {
            return Ok(previous);
        }
    }
    #[cfg(windows)]
    {
        // SAFETY: querying the runtime leaves its disposition unchanged.
        let previous = unsafe { libc::signal(signal, libc::SIG_GET) };
        if previous == libc::SIG_ERR as Handler {
            return Err(io::Error::last_os_error());
        }
        if disposition(&previous) == libc::SIG_IGN {
            return Ok(previous);
        }
    }
    replace(signal, &action())
}

fn install() -> io::Result<[Handler; SIGNALS.len()]> {
    let mut previous = std::array::from_fn(|_| action());
    for (index, signal) in SIGNALS.into_iter().enumerate() {
        match register(signal) {
            Ok(handler) => previous[index] = handler,
            Err(error) => {
                for restored in 0..index {
                    if let Err(restore) = replace(SIGNALS[restored], &previous[restored]) {
                        return Err(io::Error::other(format!(
                            "interrupt handler registration: {error}; restoration: {restore}"
                        )));
                    }
                }
                return Err(error);
            }
        }
    }
    Ok(previous)
}

pub(super) struct Monitor {
    generation: usize,
    active: bool,
    dispositions: [usize; SIGNALS.len()],
}

impl Monitor {
    pub(super) fn new() -> io::Result<Self> {
        let mut state = STATE
            .lock()
            .map_err(|_| io::Error::other("termination handler state poisoned"))?;
        if state.users == 0 {
            state.generation = INTERRUPTIONS.load(Ordering::SeqCst);
            state.previous = Some(install()?);
        }
        // A nested runner must see interruptions received during its caller's
        // setup, before the nested monitor was installed.
        let generation = state.generation;
        let dispositions = std::array::from_fn(|index| {
            disposition(&state.previous.as_ref().expect("active handlers")[index])
        });
        state.users += 1;
        Ok(Self {
            generation,
            active: true,
            dispositions,
        })
    }

    pub(super) fn interruption(&self, signal: i32, reraise_default: bool) -> Interrupted {
        if let Some(index) = SIGNALS.iter().position(|value| *value == signal) {
            let prior = self.dispositions[index];
            if reraise_default && prior == libc::SIG_DFL {
                reraise(signal);
            }
            if prior != libc::SIG_DFL && prior != libc::SIG_IGN {
                // Cleanup restored the caller's handler before handing back its signal.
                // SAFETY: the saved custom disposition owns this supported signal.
                unsafe { libc::raise(signal) };
            }
        }
        Interrupted { signal }
    }

    pub(super) fn pending(&self) -> Option<i32> {
        (INTERRUPTIONS.load(Ordering::SeqCst) != self.generation)
            .then(|| PENDING.load(Ordering::SeqCst))
    }
}

impl Monitor {
    pub(super) fn restore(&mut self) -> io::Result<()> {
        if !self.active {
            return Ok(());
        }
        let mut state = STATE
            .lock()
            .map_err(|_| io::Error::other("termination handler state poisoned"))?;
        state.users -= 1;
        self.active = false;
        let mut errors = Vec::new();
        if state.users == 0
            && let Some(previous) = state.previous.take()
        {
            for (signal, handler) in SIGNALS.into_iter().zip(previous) {
                if disposition(&handler) == libc::SIG_IGN {
                    continue;
                }
                if let Err(error) = replace(signal, &handler) {
                    errors.push(error.to_string());
                }
            }
        }
        if !errors.is_empty() {
            return Err(io::Error::other(format!(
                "termination handler restoration failed: {}",
                errors.join("; ")
            )));
        }
        Ok(())
    }
}

impl Drop for Monitor {
    fn drop(&mut self) {
        if let Err(error) = self.restore() {
            eprintln!("termination handler restoration failed: {error}");
        }
    }
}

#[derive(Debug)]
pub(crate) struct Interrupted {
    pub(crate) signal: i32,
}

impl std::fmt::Display for Interrupted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "verification interrupted by signal {}", self.signal)
    }
}

impl std::error::Error for Interrupted {}

/// Deliver the interruption only after the mutation and prior handlers are restored.
pub(super) fn reraise(signal: i32) -> ! {
    // SAFETY: the signal was delivered by the runtime and is supported here.
    unsafe { libc::raise(signal) };
    // Only a saved default disposition reaches this function.
    // The runtime default on some platforms terminates with status 3.
    std::process::exit(128 + signal);
}
