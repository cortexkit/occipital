//! Making a written file, or a folder's new and removed entries, survive a
//! crash, the same way on Windows, Linux and macOS.
//!
//! Unix makes a folder entry durable only when the folder itself is synced.
//! Windows can't open a folder as a file, so it has no folder sync, and it
//! flushes a file (`FlushFileBuffers`) only through a handle that may write to
//! it; a read-only handle is refused with "access denied".

use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;

/// Flush the file at `path` to disk. It opens the file for writing, which
/// Windows needs to flush it, but changes nothing in it.
pub(crate) fn sync_file(path: &Path) -> io::Result<()> {
    OpenOptions::new().write(true).open(path)?.sync_all()
}

/// Make the entries added to or removed from `dir` durable. A no-op on
/// Windows, where a rename is durable once the renamed file was flushed.
pub(crate) fn sync_dir(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    File::open(dir)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = dir;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_written_file_and_its_folder_sync() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("record.json");
        std::fs::write(&path, b"{}").unwrap();
        sync_file(&path).unwrap();
        sync_dir(dir.path()).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"{}");
    }
}
