//! A directory's file identity on Windows: its volume serial number and
//! 128-bit file ID, read from an open handle to the directory itself.
use super::NodeIdentity;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::path::Path;

use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::Storage::FileSystem::{
    FILE_FLAG_BACKUP_SEMANTICS, FILE_ID_128, FILE_ID_INFO, FileIdInfo, GetFileInformationByHandleEx,
};

pub(super) fn of(path: &Path) -> Option<NodeIdentity> {
    // A directory opens only with backup semantics; read access is enough.
    let directory = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
        .ok()?;
    for_handle(directory.as_raw_handle())
}

fn for_handle(handle: HANDLE) -> Option<NodeIdentity> {
    let mut information = FILE_ID_INFO {
        VolumeSerialNumber: 0,
        FileId: FILE_ID_128 {
            Identifier: [0; 16],
        },
    };
    // SAFETY: information is a correctly sized, writable FILE_ID_INFO. The
    // caller keeps the directory open; the API rejects invalid handles.
    let filled = unsafe {
        GetFileInformationByHandleEx(
            handle,
            FileIdInfo,
            (&raw mut information).cast(),
            std::mem::size_of::<FILE_ID_INFO>() as u32,
        )
    };
    if filled == 0 {
        // The legacy 64-bit index is not guaranteed unique on ReFS. Without
        // FileIdInfo we cannot prove exact restoration on this file system.
        return None;
    }
    Some(node(information))
}

fn node(information: FILE_ID_INFO) -> NodeIdentity {
    (
        information.VolumeSerialNumber,
        u128::from_le_bytes(information.FileId.Identifier),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placement_file_id_info_preserves_all_identifier_and_volume_bits() {
        let id = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
        let volume = (1u64 << 48) | 7;
        assert_eq!(
            node(FILE_ID_INFO {
                VolumeSerialNumber: volume,
                FileId: FILE_ID_128 { Identifier: id },
            }),
            (volume, u128::from_le_bytes(id)),
        );
    }

    #[test]
    fn placement_failed_file_id_info_query_has_no_identity() {
        assert_eq!(
            for_handle(windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE),
            None,
        );
    }
}
