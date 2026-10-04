//! A directory's file identity on Windows: its volume serial number and file
//! index, read from an open handle to the directory itself.
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::path::Path;

use windows_sys::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, FILE_FLAG_BACKUP_SEMANTICS, GetFileInformationByHandle,
};

pub(super) fn of(path: &Path) -> Option<(u64, u64)> {
    // A directory opens only with backup semantics; read access is enough.
    let directory = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
        .ok()?;
    let mut information = unsafe { std::mem::zeroed::<BY_HANDLE_FILE_INFORMATION>() };
    // SAFETY: the handle is open for the duration of the call and the
    // structure is a plain value the call fills in.
    let filled = unsafe { GetFileInformationByHandle(directory.as_raw_handle(), &mut information) };
    if filled == 0 {
        return None;
    }
    let index =
        (u64::from(information.nFileIndexHigh) << 32) | u64::from(information.nFileIndexLow);
    Some((u64::from(information.dwVolumeSerialNumber), index))
}
