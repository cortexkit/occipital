//! The volume space available to the calling user.

use std::{io, path::Path};

/// Available bytes for an ordinary user, excluding filesystem-reserved blocks.
#[cfg(unix)]
pub fn available_bytes(directory: &Path) -> io::Result<u64> {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(directory.as_os_str().as_bytes())?;
    let mut info = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: path is NUL-terminated and info points to sufficient writable storage.
    if unsafe { libc::statvfs(path.as_ptr(), info.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful statvfs initialized the output structure.
    let info = unsafe { info.assume_init() };
    Ok((u128::from(info.f_bavail) * u128::from(info.f_frsize)).min(u128::from(u64::MAX)) as u64)
}
/// Available bytes for the calling user on the volume holding `directory`, after
/// quotas: GetDiskFreeSpaceExW's free-bytes-available-to-caller.
#[cfg(windows)]
pub fn available_bytes(directory: &Path) -> io::Result<u64> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    extern "system" {
        fn GetDiskFreeSpaceExW(
            directory: *const u16,
            available_to_caller: *mut u64,
            total: *mut u64,
            total_free: *mut u64,
        ) -> i32;
    }
    // Like statvfs, the query refuses a path that doesn't exist.
    std::fs::metadata(directory)?;
    let wide: Vec<u16> = directory.as_os_str().encode_wide().chain([0]).collect();
    let mut available = 0u64;
    // SAFETY: `wide` is NUL-terminated and outlives the call; the two totals may be
    // null, and `available` is writable storage for the one output we read.
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut available,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(available)
}
#[cfg(not(any(unix, windows)))]
pub fn available_bytes(_: &Path) -> io::Result<u64> {
    Err(io::Error::other("store-volume space query unsupported"))
}

#[cfg(all(test, any(unix, windows)))]
mod tests {
    use super::*;
    #[test]
    fn free_space_uses_the_existing_path_and_refuses_a_missing_one() {
        let dir = tempfile::tempdir().unwrap();
        assert!(available_bytes(dir.path()).unwrap() > 0);
        assert!(available_bytes(&dir.path().join("missing")).is_err());
    }
}
