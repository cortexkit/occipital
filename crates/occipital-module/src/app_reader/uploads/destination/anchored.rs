//! Safe rustix wrappers keep every destination lookup relative to a pinned directory.

use super::*;
use rustix::fs::{self, AtFlags, FileType, Mode, OFlags, RenameFlags};
use std::os::unix::fs::MetadataExt;

const DIRECTORY_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);

fn checked(file: File, path: &Path, owner: u32) -> Result<File, PlaceError> {
    if let Some(reason) = unsafe_folder(&file.metadata()?, path, owner) {
        return Err(PlaceError::Unsafe(reason));
    }
    Ok(file)
}

fn directory_error(error: rustix::io::Errno, path: &Path) -> PlaceError {
    match error {
        rustix::io::Errno::LOOP => PlaceError::Unsafe(format!("{} is a link", path.display())),
        rustix::io::Errno::NOTDIR => {
            PlaceError::Unsafe(format!("{} is not a folder", path.display()))
        }
        rustix::io::Errno::NOENT => {
            PlaceError::Unsafe(format!("{} is not a safe folder", path.display()))
        }
        _ => io::Error::from(error).into(),
    }
}

fn open_child(parent: &File, name: &str, path: &Path, owner: u32) -> Result<File, PlaceError> {
    let fd = fs::openat(parent, name, DIRECTORY_FLAGS, Mode::empty()).map_err(|error| {
        // Linux reports ENOTDIR for O_DIRECTORY | O_NOFOLLOW on a symlink.
        // Inspect only to preserve the refusal message, never to authorize a write.
        if error == rustix::io::Errno::NOTDIR {
            if let Ok(meta) = fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW) {
                if FileType::from_raw_mode(meta.st_mode) == FileType::Symlink {
                    return PlaceError::Unsafe(format!("{} is a link", path.display()));
                }
            }
        }
        directory_error(error, path)
    })?;
    checked(File::from(fd), path, owner)
}

fn private_child(parent: &File, name: &str, path: &Path, owner: u32) -> Result<File, PlaceError> {
    match fs::mkdirat(parent, name, Mode::from_raw_mode(0o700)) {
        Ok(()) => parent.sync_all()?,
        Err(rustix::io::Errno::EXIST) => {}
        Err(error) => return Err(directory_error(error, path)),
    }
    open_child(parent, name, path, owner)
}

fn private_file(parent: &File, name: &str) -> io::Result<File> {
    fs::openat(
        parent,
        name,
        OFlags::RDWR | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::from_raw_mode(0o600),
    )
    .map(File::from)
    .map_err(io::Error::from)
}

fn ignore_uploads(uploads: &File) -> io::Result<()> {
    match private_file(uploads, ".gitignore") {
        Ok(mut file) => {
            file.write_all(b"*\n")?;
            file.sync_all()?;
            uploads.sync_all()
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error),
    }
}

/// Refuse a detached or redirected chain rather than returning a path to another
/// directory. The subsequent rename still uses the pinned handle, not this check.
fn recheck(
    chain: &[File],
    paths: &[PathBuf],
    names: &[&str],
    owner: u32,
) -> Result<(), PlaceError> {
    for (index, name) in names.iter().enumerate() {
        let current = open_child(&chain[index], name, &paths[index], owner)?.metadata()?;
        let held = chain[index + 1].metadata()?;
        if (current.dev(), current.ino()) != (held.dev(), held.ino()) {
            return Err(PlaceError::Unsafe(format!(
                "{} changed while finishing",
                paths[index].display()
            )));
        }
    }
    Ok(())
}

pub(crate) fn place(blob: &mut File, request: &Request<'_>) -> Result<Placed, PlaceError> {
    // The session binding's root is trusted and may be reached through links.
    // Resolve it once before opening and pinning the directory.
    let root_path = std::fs::canonicalize(request.project_root).map_err(|_| {
        PlaceError::Unsafe(format!(
            "project root {} is not a folder",
            request.project_root.display()
        ))
    })?;
    let root = fs::open(&root_path, DIRECTORY_FLAGS, Mode::empty())
        .map_err(|error| directory_error(error, request.project_root))?;
    let root = File::from(root);
    let cortexkit = request.project_root.join(".cortexkit");
    let uploads = cortexkit.join("uploads");
    let folder = uploads.join(request.date);
    let paths = [cortexkit, uploads, folder.clone()];
    let names = [".cortexkit", "uploads", request.date];
    let mut chain = vec![root];
    for (index, name) in names.iter().enumerate() {
        chain.push(private_child(
            &chain[index],
            name,
            &paths[index],
            request.owner,
        )?);
        if index == 1 {
            ignore_uploads(&chain[index + 1])?;
        }
    }
    let directory = &chain[3];
    // Reusing the upload's temporary name bounds leftovers after an interrupted finish.
    let temporary = format!(".{}.partial", request.upload_id);
    match fs::statat(directory, temporary.as_str(), AtFlags::SYMLINK_NOFOLLOW) {
        Ok(meta) => {
            if FileType::from_raw_mode(meta.st_mode) != FileType::RegularFile {
                return Err(PlaceError::Unsafe(format!(
                    "{} is not this module's file",
                    folder.join(&temporary).display()
                )));
            }
            fs::unlinkat(directory, temporary.as_str(), AtFlags::empty())
                .map_err(io::Error::from)?;
        }
        Err(rustix::io::Errno::NOENT) => {}
        Err(error) => return Err(io::Error::from(error).into()),
    }
    let mut file = private_file(directory, &temporary).map_err(|error| {
        if error.kind() == io::ErrorKind::AlreadyExists {
            PlaceError::Unsafe(format!(
                "{} appeared while finishing",
                folder.join(&temporary).display()
            ))
        } else {
            error.into()
        }
    })?;
    let result = (|| {
        io::copy(blob, &mut file)?;
        file.sync_all()?;
        verify(&mut file, request)?;
        let inode = file_identity(&file)?;
        let mtime_ns = mtime_ns(&file.metadata()?)?;
        drop(file);
        recheck(&chain, &paths, &names, request.owner)?;
        let mut taken = HashSet::new();
        for entry in fs::Dir::read_from(directory).map_err(io::Error::from)? {
            let entry = entry.map_err(io::Error::from)?;
            taken.insert(fold(&entry.file_name().to_string_lossy()));
        }
        for attempt in 1..=MAX_NAME_ATTEMPTS {
            let name = candidate(request.name, attempt);
            if taken.contains(&fold(&name)) {
                continue;
            }
            recheck(&chain, &paths, &names, request.owner)?;
            #[cfg(test)]
            test_hook::run(request.project_root);
            // Both names are single components in the same pinned directory. rustix
            // uses renameat2 on Linux and renameatx_np(RENAME_EXCL) on macOS.
            match fs::renameat_with(
                directory,
                temporary.as_str(),
                directory,
                name.as_str(),
                RenameFlags::NOREPLACE,
            ) {
                Ok(()) => {
                    // Replacing a directory path with a symlink cannot redirect the
                    // fd-relative rename. Also refuse a reply path that no longer
                    // names the held directory, removing only from that handle.
                    if let Err(error) = recheck(&chain, &paths, &names, request.owner) {
                        let _ = fs::unlinkat(directory, name.as_str(), AtFlags::empty());
                        directory.sync_all()?;
                        return Err(error);
                    }
                    directory.sync_all()?;
                    return Ok(Placed {
                        path: folder.join(name),
                        inode,
                        mtime_ns,
                    });
                }
                Err(rustix::io::Errno::EXIST) => continue,
                Err(error) => return Err(io::Error::from(error).into()),
            }
        }
        Err(PlaceError::Unsafe(format!(
            "every name for {} in {} is taken",
            request.name,
            folder.display()
        )))
    })();
    if result.is_err() {
        let _ = fs::unlinkat(directory, temporary.as_str(), AtFlags::empty());
    }
    result
}

#[cfg(test)]
pub(crate) mod test_hook {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    type Hook = (PathBuf, Box<dyn FnOnce() + Send>);
    static HOOKS: OnceLock<Mutex<Vec<Hook>>> = OnceLock::new();

    pub(crate) fn arm(project: &Path, hook: impl FnOnce() + Send + 'static) {
        HOOKS
            .get_or_init(Mutex::default)
            .lock()
            .unwrap()
            .push((project.into(), Box::new(hook)));
    }

    pub(super) fn run(project: &Path) {
        // Project keys prevent concurrent fixtures from consuming each other's hooks.
        let mut hooks = HOOKS.get_or_init(Mutex::default).lock().unwrap();
        if let Some(index) = hooks.iter().position(|(path, _)| path == project) {
            let (_, hook) = hooks.remove(index);
            drop(hooks);
            hook();
        }
    }
}
