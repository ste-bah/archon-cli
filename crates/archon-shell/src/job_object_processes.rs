//! Job membership plus process creation times, retained across job-handle close.
use std::{
    io,
    time::{Duration, Instant},
};
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_INVALID_PARAMETER, ERROR_MORE_DATA, FILETIME, HANDLE, WAIT_FAILED,
    WAIT_OBJECT_0,
};
use windows_sys::Win32::Storage::FileSystem::SYNCHRONIZE;
use windows_sys::Win32::System::JobObjects::{
    IsProcessInJob, JOBOBJECT_BASIC_PROCESS_ID_LIST, JobObjectBasicProcessIdList,
    QueryInformationJobObject,
};
use windows_sys::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, WaitForSingleObject,
};

/// The creation time of a live process, or `None` only for a proven exit.
pub fn identity_of(pid: u32) -> io::Result<Option<u64>> {
    identity_in(pid, None)
}

fn identity_in(pid: u32, job: Option<HANDLE>) -> io::Result<Option<u64>> {
    // SAFETY: plain arguments; the resulting handle is closed below.
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE, 0, pid) };
    if handle.is_null() {
        let error = io::Error::last_os_error();
        return if error.raw_os_error() == Some(ERROR_INVALID_PARAMETER as i32) {
            Ok(None)
        } else {
            Err(error)
        };
    }
    let result = (|| {
        // SAFETY: valid handle, zero wait only polls.
        match unsafe { WaitForSingleObject(handle, 0) } {
            WAIT_OBJECT_0 => return Ok(None),
            WAIT_FAILED => return Err(io::Error::last_os_error()),
            _ => {}
        }
        if let Some(job) = job {
            let mut belongs = 0;
            // SAFETY: valid process/job handles and output pointer.
            if unsafe { IsProcessInJob(handle, job, &mut belongs) } == 0 {
                return Err(io::Error::last_os_error());
            }
            if belongs == 0 {
                return Ok(None);
            }
        }
        let mut created = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        let mut exited = created;
        let mut kernel = created;
        let mut user = created;
        // SAFETY: valid handle and four separate output structures.
        if unsafe { GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) }
            == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(Some(
            (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime),
        ))
    })();
    // SAFETY: opened once above, closed once here.
    unsafe { CloseHandle(handle) };
    result
}

pub(super) fn in_job(handle: HANDLE, bound: Duration) -> io::Result<Vec<(u32, u64)>> {
    let deadline = Instant::now() + bound;
    let offset = std::mem::offset_of!(JOBOBJECT_BASIC_PROCESS_ID_LIST, ProcessIdList);
    let mut capacity = 32usize;
    loop {
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "job process enumeration exceeded its deadline",
            ));
        }
        let bytes = offset
            .checked_add(
                capacity
                    .checked_mul(std::mem::size_of::<usize>())
                    .ok_or_else(|| io::Error::other("job process list too large"))?,
            )
            .ok_or_else(|| io::Error::other("job process list too large"))?;
        let length =
            u32::try_from(bytes).map_err(|_| io::Error::other("job process list too large"))?;
        // usize storage provides the alignment the flexible array requires.
        let mut storage = vec![0usize; bytes.div_ceil(std::mem::size_of::<usize>())];
        let list = storage
            .as_mut_ptr()
            .cast::<JOBOBJECT_BASIC_PROCESS_ID_LIST>();
        // SAFETY: aligned, zeroed storage of `length` bytes, held through call.
        let queried = unsafe {
            QueryInformationJobObject(
                handle,
                JobObjectBasicProcessIdList,
                list.cast(),
                length,
                std::ptr::null_mut(),
            )
        };
        if queried == 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(ERROR_MORE_DATA as i32) {
                return Err(error);
            }
            capacity = capacity
                .checked_mul(2)
                .ok_or_else(|| io::Error::other("job process list too large"))?;
            continue;
        }
        // SAFETY: kernel wrote the fixed header into valid aligned storage.
        let count = unsafe { (*list).NumberOfProcessIdsInList as usize };
        // A successful call can still report an incomplete list. Both header
        // counts must agree before any membership is treated as complete.
        let assigned = unsafe { (*list).NumberOfAssignedProcesses as usize };
        if count < assigned {
            capacity = assigned.max(capacity.saturating_mul(2));
            continue;
        }
        if count > capacity {
            return Err(io::Error::other("job returned an incomplete process list"));
        }
        // SAFETY: the flexible array holds `count` usize entries within storage.
        let pids = unsafe {
            std::slice::from_raw_parts(
                storage.as_ptr().cast::<u8>().add(offset).cast::<usize>(),
                count,
            )
        };
        let mut identities = Vec::new();
        for &pid in pids {
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "job identity reads exceeded their deadline",
                ));
            }
            let pid = u32::try_from(pid).map_err(|_| io::Error::other("invalid job process id"))?;
            if let Some(start) = identity_in(pid, Some(handle))? {
                identities.push((pid, start));
            }
        }
        return Ok(identities);
    }
}
