//! File operations the standard library doesn't offer on every platform: a move
//! that never replaces an existing file, and a file's identity on its volume.
//! Both work on Windows, Linux and macOS.

use std::{fs::File, io, path::Path};

/// Move `from` to `to` only if nothing is at `to` yet; `Ok(false)` when something is.
/// Both paths must be on the same volume.
///
/// On Unix this links `to` to the file and then removes `from`: link(2) never
/// replaces an existing name and exists on every Unix, unlike a no-replace rename
/// (renameat2 on Linux, renamex_np on macOS). On Windows it is `MoveFileExW` without
/// `MOVEFILE_REPLACE_EXISTING`, with `MOVEFILE_WRITE_THROUGH` so it returns only once
/// the move is on disk.
pub fn move_new(from: &Path, to: &Path) -> io::Result<bool> {
    #[cfg(unix)]
    {
        match std::fs::hard_link(from, to) {
            Ok(()) => {
                std::fs::remove_file(from)?;
                Ok(true)
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(false),
            Err(error) => Err(error),
        }
    }
    #[cfg(windows)]
    {
        windows::move_new(from, to)
    }
}

/// `file`'s identity on its volume: the inode number on Unix, the file index on
/// Windows. A replacement created while the original still exists has a different
/// identity; once a file is deleted its number may be reused.
pub fn identity(file: &File) -> io::Result<u64> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(file.metadata()?.ino())
    }
    #[cfg(windows)]
    {
        windows::file_index(file)
    }
}

#[cfg(windows)]
mod windows {
    use std::{
        fs::File,
        io,
        os::windows::{ffi::OsStrExt, io::AsRawHandle},
        path::Path,
    };

    const MOVEFILE_WRITE_THROUGH: u32 = 0x8;
    const ERROR_FILE_EXISTS: i32 = 80;
    const ERROR_ALREADY_EXISTS: i32 = 183;

    #[repr(C)]
    #[derive(Default)]
    struct FileTime {
        low: u32,
        high: u32,
    }

    /// BY_HANDLE_FILE_INFORMATION, field for field.
    #[repr(C)]
    #[derive(Default)]
    struct ByHandleFileInformation {
        attributes: u32,
        creation: FileTime,
        last_access: FileTime,
        last_write: FileTime,
        volume_serial: u32,
        size_high: u32,
        size_low: u32,
        links: u32,
        index_high: u32,
        index_low: u32,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
        fn GetFileInformationByHandle(
            file: *mut core::ffi::c_void,
            info: *mut ByHandleFileInformation,
        ) -> i32;
    }

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain([0]).collect()
    }

    pub(super) fn move_new(from: &Path, to: &Path) -> io::Result<bool> {
        let (from, to) = (wide(from), wide(to));
        // SAFETY: both strings are NUL-terminated and outlive the call. Without
        // MOVEFILE_REPLACE_EXISTING the move fails when the target exists.
        if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), MOVEFILE_WRITE_THROUGH) } != 0 {
            return Ok(true);
        }
        let error = io::Error::last_os_error();
        match error.raw_os_error() {
            Some(ERROR_FILE_EXISTS | ERROR_ALREADY_EXISTS) => Ok(false),
            _ => Err(error),
        }
    }

    pub(super) fn file_index(file: &File) -> io::Result<u64> {
        let mut info = ByHandleFileInformation::default();
        // SAFETY: the handle is open for the call and `info` is writable storage with
        // the structure's exact layout.
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok((u64::from(info.index_high) << 32) | u64::from(info.index_low))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_move_never_replaces_an_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let (from, to) = (dir.path().join("from"), dir.path().join("to"));
        std::fs::write(&from, b"new").unwrap();
        std::fs::write(&to, b"old").unwrap();
        assert!(!move_new(&from, &to).unwrap());
        assert_eq!(std::fs::read(&to).unwrap(), b"old");
        assert_eq!(std::fs::read(&from).unwrap(), b"new");
        std::fs::remove_file(&to).unwrap();
        assert!(move_new(&from, &to).unwrap());
        assert_eq!(std::fs::read(&to).unwrap(), b"new");
        assert!(!from.exists());
    }

    #[test]
    fn a_moved_file_keeps_its_identity_and_a_replacement_does_not() {
        let dir = tempfile::tempdir().unwrap();
        let (from, to) = (dir.path().join("from"), dir.path().join("to"));
        std::fs::write(&from, b"x").unwrap();
        let before = identity(&File::open(&from).unwrap()).unwrap();
        assert!(move_new(&from, &to).unwrap());
        assert_eq!(identity(&File::open(&to).unwrap()).unwrap(), before);
        let replacement = dir.path().join("replacement");
        std::fs::write(&replacement, b"x").unwrap();
        // Created while the moved file still exists, so it can't reuse its identity.
        std::fs::remove_file(&to).unwrap();
        std::fs::rename(&replacement, &to).unwrap();
        assert_ne!(identity(&File::open(&to).unwrap()).unwrap(), before);
    }
}
