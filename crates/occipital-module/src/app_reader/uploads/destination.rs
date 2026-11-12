//! Placing a finished upload inside the session's project, under
//! `<project>/.cortexkit/uploads/<YYYY-MM-DD>/`, on Windows, Linux and macOS alike.
//!
//! The project belongs to the user and to whatever the agent ran in it, so nothing on
//! the way below the project root is trusted. The root comes from the session's
//! Entorhinal binding and is trusted, so it may be reached through links. On Linux
//! and macOS, it is resolved once and pinned; each child directory is then opened
//! without following links and checked through its handle. All writes,
//! listings, removals and the exclusive rename are relative to those handles, so a
//! later symlink swap cannot redirect them. Windows retains the path-based checks
//! (including refusal of every reparse point) and exclusive move; its final path
//! lookup is not yet handle-relative. The bytes go into a temporary file
//! inside the destination folder first (the project may be on another volume than
//! the blob), are made durable and verified there, and are then moved to a name
//! nobody holds yet, so an existing file is never replaced.
//!
//! Privacy: on Unix, folders are created 0700 and files 0600, so nothing is ever
//! executable. Windows has no mode bits and no execute bit; new folders and files
//! there take the default ACL inherited from the user's profile.

use std::{
    collections::HashSet,
    fs::{File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod anchored;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) use anchored::place;
#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
pub(crate) use anchored::test_hook;

/// Longest cleaned file name, in bytes. Every supported file system allows at least
/// 255 (UTF-8 bytes or UTF-16 units); the margin leaves room for a `-N` suffix.
pub(crate) const MAX_NAME_BYTES: usize = 180;
/// How many `-N` suffixes a finish tries before it gives up on a name.
const MAX_NAME_ATTEMPTS: u32 = 10_000;
const COPY_BUFFER: usize = 1 << 20;

/// Characters Windows refuses in a file name. `/` and `\` never get this far: the
/// name is cut to its last path component first.
const FORBIDDEN: [char; 9] = ['<', '>', ':', '"', '/', '\\', '|', '?', '*'];

/// Device names Windows reserves whatever extension follows them.
const RESERVED: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

#[derive(Debug)]
pub(crate) enum PlaceError {
    /// The folder chain is not one this module may write into.
    Unsafe(String),
    /// The bytes in the destination are not the declared file.
    Checksum(String),
    Io(io::Error),
}

impl From<io::Error> for PlaceError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// What a finished upload became.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Placed {
    pub path: PathBuf,
    /// The file's identity on its volume: the inode on Unix, the file index on Windows.
    pub inode: u64,
    pub mtime_ns: i64,
}

/// Whether `name`'s part before its first dot is a Windows device name, compared
/// case-insensitively and ignoring trailing spaces, as Windows compares it.
pub(crate) fn is_reserved(name: &str) -> bool {
    let device = name.split('.').next().unwrap_or_default().trim_end();
    RESERVED
        .iter()
        .any(|reserved| reserved.eq_ignore_ascii_case(device))
}

fn trim_edges(name: &str) -> &str {
    name.trim()
        .trim_start_matches('.')
        .trim_start()
        .trim_end_matches(['.', ' '])
        .trim_end()
}

/// The name a phone-supplied file name is stored under, valid on Windows, Linux and
/// macOS: its last path component, without control characters (NUL included) or
/// `<>:"/\|?*`, without leading dots or trailing dots and spaces, not a Windows
/// device name, and at most [`MAX_NAME_BYTES`] long with its extension kept where it
/// fits. An empty result becomes `upload`.
pub(crate) fn clean_name(raw: &str) -> String {
    let base = raw.rsplit(['/', '\\']).next().unwrap_or_default();
    let kept: String = base
        .chars()
        .filter(|c| !c.is_control() && !FORBIDDEN.contains(c))
        .collect();
    let mut name = bound(trim_edges(&kept));
    if name.is_empty() {
        name = "upload".into();
    }
    if is_reserved(&name) {
        name = bound(&format!("_{name}"));
    }
    name
}

/// Cut `name` to [`MAX_NAME_BYTES`], keeping a short extension whole.
fn bound(name: &str) -> String {
    if name.len() <= MAX_NAME_BYTES {
        return name.into();
    }
    let (stem, extension) = split_extension(name);
    // A long "extension" is just part of the name, so only short ones are kept whole.
    let extension = extension.filter(|e| e.len() <= 16);
    let room = MAX_NAME_BYTES - extension.map_or(0, |e| e.len() + 1);
    let source = if extension.is_some() { stem } else { name };
    let mut end = room.min(source.len());
    while !source.is_char_boundary(end) {
        end -= 1;
    }
    let stem = trim_edges(&source[..end]);
    match extension {
        Some(extension) => format!("{stem}.{extension}"),
        None => stem.into(),
    }
}

fn split_extension(name: &str) -> (&str, Option<&str>) {
    match name.rfind('.') {
        Some(at) if at > 0 && at + 1 < name.len() => (&name[..at], Some(&name[at + 1..])),
        _ => (name, None),
    }
}

/// The `attempt`th name for `name`: the name itself, then `stem-2.ext`, `stem-3.ext`…
pub(crate) fn candidate(name: &str, attempt: u32) -> String {
    if attempt <= 1 {
        return name.into();
    }
    match split_extension(name) {
        (stem, Some(extension)) => format!("{stem}-{attempt}.{extension}"),
        (stem, None) => format!("{stem}-{attempt}"),
    }
}

/// The key two names collide under on a case-insensitive file system, the default
/// on Windows and macOS.
pub(crate) fn fold(name: &str) -> String {
    name.to_lowercase()
}

/// `YYYY-MM-DD` in UTC for a time in milliseconds since the Unix epoch.
pub(crate) fn utc_date(ms: i64) -> String {
    // Howard Hinnant's days-to-civil conversion, exact for the proleptic Gregorian calendar.
    let days = ms.div_euclid(86_400_000);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

/// Whether `meta` (from `symlink_metadata`) is a link of any kind: a symlink, or on
/// Windows any reparse point, which covers junctions and mount points too.
pub(crate) fn is_link(meta: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return true;
        }
    }
    meta.file_type().is_symlink()
}

/// Why `meta` (from `symlink_metadata` on `path`) is not a folder this module may
/// write into, or `None` when it is. On Unix, `owner` is the user id the folder
/// must belong to, supplied by the caller.
fn unsafe_folder(meta: &std::fs::Metadata, path: &Path, owner: u32) -> Option<String> {
    if is_link(meta) {
        return Some(format!("{} is a link", path.display()));
    }
    if !meta.is_dir() {
        return Some(format!("{} is not a folder", path.display()));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.uid() != owner {
            return Some(format!("{} belongs to another user", path.display()));
        }
    }
    #[cfg(not(unix))]
    let _ = owner;
    None
}

/// Create the folder `path`, owner-only on Unix. On Windows it takes the default ACL
/// inherited from its parent, which in a user's profile is that user's.
pub(crate) fn create_private_dir(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new().mode(0o700).create(path)
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir(path)
    }
}

/// Make `path`'s new or removed entries durable. Unix makes a directory entry durable
/// only when the directory itself is synced. Windows has no directory sync: there
/// the file is flushed with `FlushFileBuffers` (`sync_all`) and moved with
/// `MOVEFILE_WRITE_THROUGH`, which returns only once the move is on disk.
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn sync_dir(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// Check `path` is a folder this module may write into, creating it when missing.
/// Returns whether it was created.
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn private_dir(path: &Path, owner: u32) -> Result<bool, PlaceError> {
    let mut created = false;
    for _ in 0..2 {
        match std::fs::symlink_metadata(path) {
            Ok(meta) => {
                return match unsafe_folder(&meta, path, owner) {
                    Some(reason) => Err(PlaceError::Unsafe(reason)),
                    None => Ok(created),
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                // Creating never follows a link: a link appearing here makes it fail
                // with AlreadyExists, and the next look refuses it.
                match create_private_dir(path) {
                    Ok(()) => {
                        created = true;
                        if let Some(parent) = path.parent() {
                            sync_dir(parent)?;
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                    Err(error) => return Err(error.into()),
                }
            }
            Err(error) => return Err(error.into()),
        }
    }
    Err(PlaceError::Unsafe(format!(
        "{} kept changing while it was checked",
        path.display()
    )))
}

/// Check the existing folders of the chain again without creating anything.
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn recheck(chain: &[PathBuf], owner: u32) -> Result<(), PlaceError> {
    for path in chain {
        let meta = std::fs::symlink_metadata(path)?;
        if let Some(reason) = unsafe_folder(&meta, path, owner) {
            return Err(PlaceError::Unsafe(reason));
        }
    }
    Ok(())
}

/// A new file at `path`, owner-only on Unix. Creation fails on anything already
/// there, a link included, so it never writes through one.
pub(crate) fn create_private_file(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    options.open(path)
}

/// Create the uploads folder's `.gitignore` containing `*`, so git ignores the
/// uploads; an existing `.gitignore` there is left as it is.
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn ignore_uploads(uploads: &Path) -> io::Result<()> {
    let path = uploads.join(".gitignore");
    match create_private_file(&path) {
        Ok(mut file) => {
            file.write_all(b"*\n")?;
            file.sync_all()?;
            sync_dir(uploads)
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error),
    }
}

/// The SHA-256 and length of everything `file` holds, read from its start.
fn digest(file: &mut File) -> io::Result<(String, u64)> {
    use std::io::Seek;
    file.seek(io::SeekFrom::Start(0))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; COPY_BUFFER];
    let mut size = 0u64;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        size += read as u64;
    }
    Ok((format!("{:x}", hasher.finalize()), size))
}

/// Check the destination's own bytes after syncing, not the source blob's bytes.
fn verify(file: &mut File, request: &Request<'_>) -> Result<(), PlaceError> {
    let (sha256, size) = digest(file)?;
    if size != request.size || sha256 != request.sha256 {
        return Err(PlaceError::Checksum(format!(
            "the file holds {size} bytes with SHA-256 {sha256}, not {} bytes with {}",
            request.size, request.sha256
        )));
    }
    Ok(())
}

/// The file's identity on its volume: the inode on Unix, the file index on Windows.
pub(crate) fn file_identity(file: &File) -> io::Result<u64> {
    occipital_files::identity(file)
}

/// `meta`'s modification time in nanoseconds since the Unix epoch.
pub(crate) fn mtime_ns(meta: &std::fs::Metadata) -> io::Result<i64> {
    let modified = meta.modified()?;
    Ok(match modified.duration_since(std::time::UNIX_EPOCH) {
        Ok(after) => i64::try_from(after.as_nanos()).unwrap_or(i64::MAX),
        Err(before) => -i64::try_from(before.duration().as_nanos()).unwrap_or(i64::MAX),
    })
}

/// One finish's destination request.
pub(crate) struct Request<'a> {
    pub project_root: &'a Path,
    pub date: &'a str,
    pub name: &'a str,
    pub upload_id: &'a str,
    pub size: u64,
    pub sha256: &'a str,
    /// The user id destination folders must belong to on Unix. Unused on Windows,
    /// where folders take the user profile's default ACL.
    pub owner: u32,
}

/// Copy `blob` into the project and return the file it became. A refusal may leave
/// the folders made for uploads and the uploads folder's `.gitignore`; the temporary
/// file is removed (a failed removal is retried by the next finish, which reuses
/// the same temporary name).
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(crate) fn place(blob: &mut File, request: &Request<'_>) -> Result<Placed, PlaceError> {
    // The project root comes from the session's Entorhinal binding, not from the
    // phone, so it is the one folder that may be reached through a link.
    if !request.project_root.is_dir() {
        return Err(PlaceError::Unsafe(format!(
            "project root {} is not a folder",
            request.project_root.display()
        )));
    }
    let cortexkit = request.project_root.join(".cortexkit");
    let uploads = cortexkit.join("uploads");
    let folder = uploads.join(request.date);
    private_dir(&cortexkit, request.owner)?;
    private_dir(&uploads, request.owner)?;
    ignore_uploads(&uploads)?;
    private_dir(&folder, request.owner)?;
    let chain = [cortexkit, uploads, folder.clone()];

    // A fixed temporary name per upload, so a retried finish replaces its own
    // leftover instead of piling up new ones.
    let temporary = folder.join(format!(".{}.partial", request.upload_id));
    if let Ok(meta) = std::fs::symlink_metadata(&temporary) {
        if is_link(&meta) || !meta.is_file() {
            return Err(PlaceError::Unsafe(format!(
                "{} is not this module's file",
                temporary.display()
            )));
        }
        std::fs::remove_file(&temporary)?;
    }
    let mut file = create_private_file(&temporary).map_err(|error| {
        if error.kind() == io::ErrorKind::AlreadyExists {
            PlaceError::Unsafe(format!("{} appeared while finishing", temporary.display()))
        } else {
            error.into()
        }
    })?;
    let result = (|| {
        io::copy(blob, &mut file)?;
        file.sync_all()?;
        // Verified from the destination's own bytes, after they are durable there.
        verify(&mut file, request)?;
        let inode = file_identity(&file)?;
        let mtime_ns = mtime_ns(&file.metadata()?)?;
        // Closed before the move: Windows can't move a file another handle holds
        // without delete sharing.
        drop(file);
        recheck(&chain, request.owner)?;
        // Names already in the folder, folded: on a case-insensitive file system
        // `Clip.MOV` takes the place of `clip.mov`, so neither may be chosen twice.
        let taken: HashSet<String> = std::fs::read_dir(&folder)?
            .filter_map(Result::ok)
            .map(|entry| fold(&entry.file_name().to_string_lossy()))
            .collect();
        for attempt in 1..=MAX_NAME_ATTEMPTS {
            let name = candidate(request.name, attempt);
            if taken.contains(&fold(&name)) {
                continue;
            }
            let path = folder.join(&name);
            // Never replaces a file: a name taken since the listing is just skipped.
            if occipital_files::move_new(&temporary, &path)? {
                sync_dir(&folder)?;
                return Ok(Placed {
                    path,
                    inode,
                    mtime_ns,
                });
            }
        }
        Err(PlaceError::Unsafe(format!(
            "every name for {} in {} is taken",
            request.name,
            folder.display()
        )))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}
