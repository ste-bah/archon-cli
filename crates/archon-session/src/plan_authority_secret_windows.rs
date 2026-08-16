use std::ffi::{OsStr, c_void};
use std::io;
use std::iter;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::ptr::null_mut;

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_FILE_EXISTS, GENERIC_READ, GENERIC_WRITE, GetLastError, HANDLE,
    INVALID_HANDLE_VALUE, LocalFree,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, GetSecurityInfo, SE_FILE_OBJECT,
};
#[cfg(test)]
use windows_sys::Win32::Security::SetFileSecurityW;
use windows_sys::Win32::Security::{
    ACCESS_ALLOWED_ACE, ACL, ACL_SIZE_INFORMATION, AclSizeInformation, CreateWellKnownSid,
    DACL_SECURITY_INFORMATION, EqualSid, GROUP_SECURITY_INFORMATION, GetAce, GetAclInformation,
    GetLengthSid, GetSecurityDescriptorControl, GetSecurityDescriptorDacl,
    GetSecurityDescriptorOwner, GetTokenInformation, INHERITED_ACE, OWNER_SECURITY_INFORMATION,
    PSECURITY_DESCRIPTOR, PSID, SE_DACL_PRESENT, SE_DACL_PROTECTED, SECURITY_ATTRIBUTES,
    TOKEN_QUERY, TOKEN_USER, TokenUser, WinLocalSystemSid,
};
use windows_sys::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, CREATE_NEW, CreateFileW, FILE_ALL_ACCESS, FILE_ATTRIBUTE_DIRECTORY,
    FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_SHARE_READ, FILE_SHARE_WRITE, FlushFileBuffers, GetFileInformationByHandle, OPEN_EXISTING,
    READ_CONTROL, ReadFile, WriteFile,
};
use windows_sys::Win32::System::SystemServices::ACCESS_ALLOWED_ACE_TYPE;
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

const SDDL_REVISION_1: u32 = 1;

pub(super) fn create_owner_only_file(path: &Path, bytes: &[u8; 32]) -> io::Result<[u8; 32]> {
    let user_sid = current_user_sid_sddl()?;
    let mut descriptor = null_mut();
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            wide(&super::owner_only_sddl(&user_sid)).as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            null_mut(),
        )
    } == 0
    {
        return Err(last_error("build plan approval secret ACL"));
    }
    let result = (|| {
        let mut attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor,
            bInheritHandle: 0,
        };
        let handle = unsafe {
            CreateFileW(
                wide_path(path)?.as_ptr(),
                GENERIC_READ | GENERIC_WRITE | READ_CONTROL,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                &mut attributes,
                CREATE_NEW,
                FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT,
                null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(create_error());
        }
        let result = (|| {
            validate_regular_file_handle(handle)?;
            write_handle(handle, bytes)?;
            validate_owner_only_handle(handle)?;
            Ok(*bytes)
        })();
        unsafe { CloseHandle(handle) };
        result
    })();
    unsafe { LocalFree(descriptor as *mut c_void) };
    result
}

pub(super) fn read_owner_only_file(path: &Path) -> io::Result<[u8; 32]> {
    let handle = open_existing_handle(path)?;
    let result = (|| {
        validate_regular_file_handle(handle)?;
        validate_owner_only_handle(handle)?;
        read_handle_secret(handle)
    })();
    unsafe { CloseHandle(handle) };
    result
}

fn open_existing_handle(path: &Path) -> io::Result<HANDLE> {
    let handle = unsafe {
        CreateFileW(
            wide_path(path)?.as_ptr(),
            GENERIC_READ | READ_CONTROL,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            null_mut(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT,
            null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        Err(io::Error::last_os_error())
    } else {
        Ok(handle)
    }
}

fn validate_regular_file_handle(handle: HANDLE) -> io::Result<()> {
    let mut information = unsafe { std::mem::zeroed::<BY_HANDLE_FILE_INFORMATION>() };
    if unsafe { GetFileInformationByHandle(handle, &mut information) } == 0 {
        return Err(last_error("query plan approval secret file type"));
    }
    if information.dwFileAttributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT) != 0
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "plan approval secret must be a regular non-reparse file",
        ));
    }
    Ok(())
}

fn validate_owner_only_handle(handle: HANDLE) -> io::Result<()> {
    let information =
        OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION;
    let mut descriptor: PSECURITY_DESCRIPTOR = null_mut();
    let status = unsafe {
        GetSecurityInfo(
            handle,
            SE_FILE_OBJECT,
            information,
            null_mut(),
            null_mut(),
            null_mut(),
            null_mut(),
            &mut descriptor,
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    let user_sid = current_user_sid()?;
    let result = validate_protected_owner_system_dacl(descriptor, user_sid.as_ptr() as PSID);
    unsafe { LocalFree(descriptor as *mut c_void) };
    result
}

fn validate_protected_owner_system_dacl(
    descriptor: PSECURITY_DESCRIPTOR,
    user_sid: PSID,
) -> io::Result<()> {
    let mut owner = null_mut();
    let mut defaulted = 0;
    if unsafe { GetSecurityDescriptorOwner(descriptor, &mut owner, &mut defaulted) } == 0 {
        return Err(last_error("query plan approval secret owner"));
    }
    if unsafe { EqualSid(owner, user_sid) } == 0 {
        return Err(reject_acl());
    }

    let mut control = 0_u16;
    let mut revision = 0_u32;
    if unsafe { GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) } == 0 {
        return Err(last_error("query plan approval secret ACL control"));
    }
    if control & (SE_DACL_PRESENT | SE_DACL_PROTECTED) != (SE_DACL_PRESENT | SE_DACL_PROTECTED) {
        return Err(reject_acl());
    }

    let mut present = 0;
    let mut dacl: *mut ACL = null_mut();
    if unsafe { GetSecurityDescriptorDacl(descriptor, &mut present, &mut dacl, &mut defaulted) }
        == 0
    {
        return Err(last_error("query plan approval secret ACL"));
    }
    if present == 0 || dacl.is_null() {
        return Err(reject_acl());
    }

    let mut info = unsafe { std::mem::zeroed::<ACL_SIZE_INFORMATION>() };
    if unsafe {
        GetAclInformation(
            dacl,
            &mut info as *mut ACL_SIZE_INFORMATION as *mut c_void,
            std::mem::size_of::<ACL_SIZE_INFORMATION>() as u32,
            AclSizeInformation,
        )
    } == 0
    {
        return Err(last_error("query plan approval secret ACE count"));
    }
    if info.AceCount != 2 {
        return Err(reject_acl());
    }

    let system_sid = local_system_sid()?;
    let mut user_ace = false;
    let mut system_ace = false;
    for index in 0..info.AceCount {
        let mut ace = null_mut();
        if unsafe { GetAce(dacl, index, &mut ace) } == 0 {
            return Err(last_error("read plan approval secret ACE"));
        }
        let allow = unsafe { &*(ace as *const ACCESS_ALLOWED_ACE) };
        if allow.Header.AceType as u32 != ACCESS_ALLOWED_ACE_TYPE
            || allow.Header.AceFlags as u32 & INHERITED_ACE != 0
            || allow.Mask != FILE_ALL_ACCESS
        {
            return Err(reject_acl());
        }
        let sid = &allow.SidStart as *const u32 as PSID;
        if unsafe { EqualSid(sid, user_sid) } != 0 && !user_ace {
            user_ace = true;
        } else if unsafe { EqualSid(sid, system_sid.as_ptr() as PSID) } != 0 && !system_ace {
            system_ace = true;
        } else {
            return Err(reject_acl());
        }
    }
    if user_ace && system_ace {
        Ok(())
    } else {
        Err(reject_acl())
    }
}

fn local_system_sid() -> io::Result<Vec<u8>> {
    let mut len = 0_u32;
    unsafe { CreateWellKnownSid(WinLocalSystemSid, null_mut(), null_mut(), &mut len) };
    if len == 0 {
        return Err(last_error("query LocalSystem SID"));
    }
    let mut sid = vec![0_u8; len as usize];
    if unsafe {
        CreateWellKnownSid(
            WinLocalSystemSid,
            null_mut(),
            sid.as_mut_ptr() as PSID,
            &mut len,
        )
    } == 0
    {
        return Err(last_error("create LocalSystem SID"));
    }
    Ok(sid)
}

fn current_user_sid() -> io::Result<Vec<u8>> {
    let mut token: HANDLE = null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(last_error("open current user token"));
    }
    let result = (|| {
        let mut needed = 0_u32;
        unsafe { GetTokenInformation(token, TokenUser, null_mut(), 0, &mut needed) };
        if needed == 0 {
            return Err(last_error("query current user token"));
        }
        let mut buffer = vec![0_u8; needed as usize];
        if unsafe {
            GetTokenInformation(
                token,
                TokenUser,
                buffer.as_mut_ptr() as *mut c_void,
                needed,
                &mut needed,
            )
        } == 0
        {
            return Err(last_error("read current user token"));
        }
        let sid = unsafe { (*(buffer.as_ptr() as *const TOKEN_USER)).User.Sid };
        let len = unsafe { GetLengthSid(sid) } as usize;
        Ok(unsafe { std::slice::from_raw_parts(sid as *const u8, len) }.to_vec())
    })();
    unsafe { CloseHandle(token) };
    result
}

fn current_user_sid_sddl() -> io::Result<String> {
    let sid = current_user_sid()?;
    let mut text = null_mut();
    if unsafe {
        windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW(
            sid.as_ptr() as PSID,
            &mut text,
        )
    } == 0
    {
        return Err(last_error("convert current user SID"));
    }
    let len = wide_len(text);
    let result = String::from_utf16(unsafe { std::slice::from_raw_parts(text, len) })
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid current user SID"));
    unsafe { LocalFree(text as *mut c_void) };
    result
}

fn read_handle_secret(handle: HANDLE) -> io::Result<[u8; 32]> {
    let mut bytes = [0_u8; 32];
    let mut read = 0_u32;
    if unsafe { ReadFile(handle, bytes.as_mut_ptr(), 32, &mut read, null_mut()) } == 0 || read != 32
    {
        return Err(last_error("read plan approval secret"));
    }
    let mut trailing = [0_u8; 1];
    if unsafe { ReadFile(handle, trailing.as_mut_ptr(), 1, &mut read, null_mut()) } == 0 {
        return Err(last_error("read plan approval secret length"));
    }
    if read == 0 {
        Ok(bytes)
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "plan approval secret has an invalid length",
        ))
    }
}

fn write_handle(handle: HANDLE, bytes: &[u8]) -> io::Result<()> {
    let mut written = 0_u32;
    if unsafe { WriteFile(handle, bytes.as_ptr(), 32, &mut written, null_mut()) } == 0
        || written != 32
    {
        return Err(last_error("write plan approval secret"));
    }
    if unsafe { FlushFileBuffers(handle) } == 0 {
        return Err(last_error("flush plan approval secret"));
    }
    Ok(())
}

fn wide_path(path: &Path) -> io::Result<Vec<u16>> {
    let text = path
        .as_os_str()
        .encode_wide()
        .chain(iter::once(0))
        .collect::<Vec<_>>();
    if text.len() <= 1 {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "empty plan approval secret path",
        ))
    } else {
        Ok(text)
    }
}

fn wide(value: &str) -> Vec<u16> {
    OsStr::new(value)
        .encode_wide()
        .chain(iter::once(0))
        .collect()
}

fn wide_len(value: *const u16) -> usize {
    let mut current = value;
    unsafe {
        while *current != 0 {
            current = current.add(1);
        }
        current.offset_from(value) as usize
    }
}

fn create_error() -> io::Error {
    let error = unsafe { GetLastError() };
    if error == ERROR_FILE_EXISTS {
        io::Error::new(
            io::ErrorKind::AlreadyExists,
            "plan approval secret already exists",
        )
    } else {
        io::Error::from_raw_os_error(error as i32)
    }
}

fn reject_acl() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "plan approval secret ACL is not protected owner-and-SYSTEM only",
    )
}

fn last_error(context: &str) -> io::Error {
    io::Error::other(format!("{context}: {}", io::Error::last_os_error()))
}

#[cfg(test)]
pub(super) fn validate_owner_only_acl(path: &Path) -> io::Result<()> {
    let handle = open_existing_handle(path)?;
    let result = (|| {
        validate_regular_file_handle(handle)?;
        validate_owner_only_handle(handle)
    })();
    unsafe { CloseHandle(handle) };
    result
}

#[cfg(test)]
pub(super) fn replace_acl_for_test(path: &Path, sddl: &str) -> io::Result<()> {
    let mut descriptor = null_mut();
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            wide(sddl).as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            null_mut(),
        )
    } == 0
    {
        return Err(last_error("build permissive plan approval secret ACL"));
    }
    let result = (|| {
        if unsafe {
            SetFileSecurityW(
                wide_path(path)?.as_ptr(),
                DACL_SECURITY_INFORMATION,
                descriptor,
            )
        } == 0
        {
            Err(last_error("replace plan approval secret ACL"))
        } else {
            Ok(())
        }
    })();
    unsafe { LocalFree(descriptor as *mut c_void) };
    result
}
