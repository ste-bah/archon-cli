//! Windows Job Objects that the caller owns (Issues 242 and 273).
//!
//! A child is created suspended, put in a fresh job, and only then resumed,
//! so no process it starts can be outside the job. The job is created with
//! `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`: closing its last handle, which a
//! dropped [`Job`] or a dead owner does, kills every process in it.
//!
//! `TerminateJobObject` does not wait, so terminating is not confirming.
//! [`Job::kill_and_confirm`] terminates and then waits, within a bound, until
//! the job's accounting reports no active process, and says how many were
//! still active if the bound ran out.
//!
//! `process-wrap` keeps its job handle private and its wait ends at the first
//! completion packet of any kind, so it can confirm neither; that is why the
//! callers own the job here.

use std::ffi::c_void;
use std::io;
use std::os::windows::io::RawHandle;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, ERROR_FILE_NOT_FOUND, GetLastError, HANDLE,
    INVALID_HANDLE_VALUE, SetLastError,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JobObjectBasicAccountingInformation, JobObjectExtendedLimitInformation, OpenJobObjectW,
    QueryInformationJobObject, SetInformationJobObject, TerminateJobObject,
};
use windows_sys::Win32::System::Threading::{
    CREATE_SUSPENDED, OpenThread, ResumeThread, THREAD_SUSPEND_RESUME, TerminateProcess,
};

/// The process-creation flag a confined child must be spawned with (pass it
/// to `creation_flags`), so that it runs no code before [`Job::adopt_suspended`].
pub const CREATE_SUSPENDED_FLAG: u32 = CREATE_SUSPENDED;

/// `JOB_OBJECT_QUERY` access: enough to read a job's accounting.
const JOB_OBJECT_QUERY: u32 = 0x0004;
const CONFIRM_POLL: Duration = Duration::from_millis(10);

/// An owned Job Object. Dropping it kills every process still in it.
#[derive(Debug)]
pub struct Job {
    handle: HANDLE,
    name: Option<String>,
}

// SAFETY: a job handle is a kernel object handle, usable from any thread.
unsafe impl Send for Job {}
// SAFETY: every method only passes the handle to thread-safe kernel calls.
unsafe impl Sync for Job {}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

impl Job {
    /// A fresh job, killed on close, optionally named (in the session's
    /// `Local\` namespace when the name says so) so another process can
    /// later ask whether it still runs. A name that already exists is
    /// refused: the job must be new, and ours.
    pub fn create(name: Option<&str>) -> io::Result<Self> {
        let encoded = name.map(wide);
        let pointer = encoded.as_ref().map_or(std::ptr::null(), |w| w.as_ptr());
        // A successful create is not documented to clear the last error, so
        // it is cleared first: only ERROR_ALREADY_EXISTS set by this call
        // may say the name was taken.
        // SAFETY: sets the calling thread's last-error value only.
        unsafe { SetLastError(0) };
        // SAFETY: a null security descriptor and a NUL-terminated name.
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), pointer) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: reads the calling thread's last error, set by the create.
        let existed = name.is_some() && unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        if existed {
            // Another owner's job: close this handle without the drop's
            // termination, which would end that owner's processes.
            // SAFETY: the handle was just opened and is closed once.
            unsafe { CloseHandle(handle) };
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("job object {} already exists", name.unwrap_or_default()),
            ));
        }
        let job = Self {
            handle,
            name: name.map(str::to_string),
        };
        // SAFETY: an all-zero limit structure is valid; only one flag is set.
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: the pointer and size describe `limits`, alive for the call.
        let set = unsafe {
            SetInformationJobObject(
                job.handle,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast::<c_void>(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if set == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(job)
    }

    /// The name given at creation.
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// Put a child spawned with [`CREATE_SUSPENDED_FLAG`] into the job, then
    /// let it run. If either step fails the child is terminated: it has run
    /// no code, and it must not run unconfined.
    pub fn adopt_suspended(&self, process: RawHandle, pid: u32) -> io::Result<()> {
        let process = process as HANDLE;
        // SAFETY: both handles are valid for the call.
        if unsafe { AssignProcessToJobObject(self.handle, process) } == 0 {
            let error = io::Error::last_os_error();
            // SAFETY: the caller's process handle is valid.
            unsafe { TerminateProcess(process, 1) };
            return Err(error);
        }
        resume_threads(pid).inspect_err(|_| {
            let _ = self.terminate();
        })
    }

    /// How many processes are in the job now.
    pub fn active_processes(&self) -> io::Result<u32> {
        active_processes(self.handle)
    }

    /// Kernel CPU totals plus process creation/exit activity for the whole job.
    pub fn activity_stamp(&self) -> io::Result<(i64, i64, u32, u32)> {
        // SAFETY: zeroed accounting is valid output space, with exactly its size.
        let mut info: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = unsafe { std::mem::zeroed() };
        let queried = unsafe {
            QueryInformationJobObject(
                self.handle,
                JobObjectBasicAccountingInformation,
                (&mut info as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
                std::mem::size_of_val(&info) as u32,
                std::ptr::null_mut(),
            )
        };
        if queried == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok((
            info.TotalUserTime,
            info.TotalKernelTime,
            info.TotalProcesses,
            info.ActiveProcesses,
        ))
    }

    /// Ask every process in the job to end. Does not wait.
    pub fn terminate(&self) -> io::Result<()> {
        // SAFETY: the handle is valid while `self` lives.
        if unsafe { TerminateJobObject(self.handle, 1) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// Terminate the job and wait, within `bound`, until no process in it is
    /// active. Returns how many still were when the bound ran out: zero means
    /// the job is confirmed empty. Each round terminates again, so a process
    /// started while the last one landed is ended too. Blocking: call it on
    /// a thread that may wait, never on an async runtime thread.
    pub fn kill_and_confirm(&self, bound: Duration) -> io::Result<u32> {
        kill_and_confirm(self.handle, bound)
    }
}

fn kill_and_confirm(handle: HANDLE, bound: Duration) -> io::Result<u32> {
    let deadline = Instant::now() + bound;
    loop {
        // SAFETY: the handle is valid while its owner holds it.
        if unsafe { TerminateJobObject(handle, 1) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let active = active_processes(handle)?;
        if active == 0 || Instant::now() >= deadline {
            return Ok(active);
        }
        std::thread::sleep(CONFIRM_POLL);
    }
}

/// How long a dropped job waits for its processes to end.
const DROP_CONFIRM_BOUND: Duration = Duration::from_secs(5);

/// A job handle on its way to being closed by a dedicated thread.
struct Closing(HANDLE);

// SAFETY: a job handle is a kernel object handle, usable from any thread.
unsafe impl Send for Closing {}

impl Drop for Job {
    /// Terminates the job and waits, within [`DROP_CONFIRM_BOUND`], until no
    /// process in it is active, before the handle closes: closing alone only
    /// requests termination, and a process can stay pending on I/O. The wait
    /// runs on a dedicated thread, because a drop may run on an async
    /// runtime thread that must not block; a caller that needs the answer
    /// calls [`Job::kill_and_confirm`] itself first.
    fn drop(&mut self) {
        let closing = Closing(self.handle);
        let spawned = std::thread::Builder::new()
            .name("archon-job-close".into())
            .spawn(move || {
                let closing = closing;
                let _ = kill_and_confirm(closing.0, DROP_CONFIRM_BOUND);
                // SAFETY: the handle is owned and closed exactly once.
                unsafe { CloseHandle(closing.0) };
            });
        if spawned.is_err() {
            // No thread: close now. Kill-on-close still ends every process.
            // SAFETY: the handle is owned and closed exactly once.
            unsafe { CloseHandle(self.handle) };
        }
    }
}

fn active_processes(handle: HANDLE) -> io::Result<u32> {
    // SAFETY: an all-zero accounting structure is valid output space.
    let mut info: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: the pointer and size describe `info`, alive for the call.
    let queried = unsafe {
        QueryInformationJobObject(
            handle,
            JobObjectBasicAccountingInformation,
            (&mut info as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast::<c_void>(),
            std::mem::size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
            std::ptr::null_mut(),
        )
    };
    if queried == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(info.ActiveProcesses)
}

/// Whether the job named `name` still has an active process. A job no
/// handle holds any more has gone, and with it, killed on close, every
/// process it held: that is `false`, not an error.
pub fn named_job_running(name: &str) -> io::Result<bool> {
    let encoded = wide(name);
    // SAFETY: a NUL-terminated name; the handle is closed below.
    let handle = unsafe { OpenJobObjectW(JOB_OBJECT_QUERY, 0, encoded.as_ptr()) };
    if handle.is_null() {
        // SAFETY: reads the calling thread's last error, set by the open.
        if unsafe { GetLastError() } == ERROR_FILE_NOT_FOUND {
            return Ok(false);
        }
        return Err(io::Error::last_os_error());
    }
    let active = active_processes(handle);
    // SAFETY: the handle was opened above and is closed once.
    unsafe { CloseHandle(handle) };
    Ok(active? > 0)
}

/// Resume every thread of the suspended process `pid`.
fn resume_threads(pid: u32) -> io::Result<()> {
    // SAFETY: a thread snapshot of the whole system; closed below.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    let result = resume_in(snapshot, pid);
    // SAFETY: the snapshot handle is closed once.
    unsafe { CloseHandle(snapshot) };
    result
}

fn resume_in(snapshot: HANDLE, pid: u32) -> io::Result<()> {
    // SAFETY: an all-zero entry with its size set is valid input.
    let mut entry: THREADENTRY32 = unsafe { std::mem::zeroed() };
    entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
    let mut resumed = 0usize;
    // SAFETY: `entry` is valid for every call.
    let mut more = unsafe { Thread32First(snapshot, &mut entry) } != 0;
    while more {
        if entry.th32OwnerProcessID == pid {
            // SAFETY: opens one thread of the child; closed below.
            let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) };
            if thread.is_null() {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: the thread handle is valid; it is closed once.
            let previous = unsafe { ResumeThread(thread) };
            let error = (previous == u32::MAX).then(io::Error::last_os_error);
            unsafe { CloseHandle(thread) };
            if let Some(error) = error {
                return Err(error);
            }
            resumed += 1;
        }
        // SAFETY: as above.
        more = unsafe { Thread32Next(snapshot, &mut entry) } != 0;
    }
    if resumed == 0 {
        return Err(io::Error::other(format!(
            "no thread of suspended process {pid} was found to resume"
        )));
    }
    Ok(())
}

#[cfg(test)]
#[path = "job_object_tests.rs"]
mod tests;
