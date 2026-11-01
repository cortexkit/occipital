//! What Engram backs up: `engram-catalog.json` in the module's data folder.
//!
//! Engram reads every module's catalog at its nightly capture, so writing the
//! file is the whole registration. Its paths are relative to the data folder.
//!
//! - **The store** is captured whole while the module keeps writing, through
//!   SQLite's online backup. A restored older copy is safe to serve: the module
//!   never sends a message again on its own (it only asks the runner how a held
//!   send went), and a repeat from an app derives the same runner key from the
//!   app's message id, which the llm-runner contract answers with the existing
//!   send instead of delivering it again. An upload row
//!   whose partial file is missing lowers its committed bytes, and the app
//!   resumes from there.
//! - **Partial uploads** are left out. They are useless after their 24 hours.
//! - **Tool display records** (`display.db`: records, completions and views)
//!   are captured whole like the store. A provider writes each record only
//!   once, so a record lost in a restore would never come back. A restored
//!   older copy only lacks the newest records, whose calls then draw as the
//!   plain tool row the apps show without a record.
//! - **Tool display details** (`display-details.db`) are left out. They are
//!   large and can hold file contents, and only a tapped row reads one.

use std::io::{self, Write};
use std::path::Path;

/// The catalog's file name, beside the store.
pub const FILE: &str = "engram-catalog.json";

/// The catalog's bytes for a store file named `store_file`. A store that holds
/// anything new must be described here before it ships.
pub fn catalog(store_file: &str) -> Vec<u8> {
    let catalog = serde_json::json!({
        "schema_version": 1,
        "module_id": crate::manifest::MODULE_ID,
        "entries": [
            {
                "entry_id": "occipital/store",
                "class": "portable",
                "mechanism": "whole-db",
                "path": store_file,
                "writer_interaction": "backup-api-live",
            },
            {
                "entry_id": "occipital/upload-blobs",
                "class": "excluded",
                "path": crate::app_reader::uploads::BLOB_DIR,
                "reason": "partial uploads, useless after 24 hours",
            },
            {
                "entry_id": "occipital/display",
                "class": "portable",
                "mechanism": "whole-db",
                "path": crate::display::store::FILE,
                "writer_interaction": "backup-api-live",
            },
            {
                "entry_id": "occipital/display-details",
                "class": "excluded",
                "path": crate::display::store::DETAILS_FILE,
                "reason": "tool call details: large, and can hold file contents",
            },
        ],
    });
    let mut bytes = catalog.to_string().into_bytes();
    bytes.push(b'\n');
    bytes
}

/// Write the catalog into `data_root`, owner-only, unless it already holds
/// these bytes. The new file is written beside it, flushed and renamed over
/// it, so Engram never reads a half-written catalog.
pub fn write_catalog(data_root: &Path, store_file: &str) -> io::Result<()> {
    let bytes = catalog(store_file);
    let path = data_root.join(FILE);
    if std::fs::read(&path).is_ok_and(|current| current == bytes) {
        return Ok(());
    }
    let staged = data_root.join(format!(".{FILE}.new"));
    // A file left by a write that stopped part way would refuse the create.
    match std::fs::remove_file(&staged) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let mut file = crate::app_reader::uploads::destination::create_private_file(&staged)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&staged, &path)?;
    // On Unix a rename is durable only once its folder is synced.
    #[cfg(unix)]
    std::fs::File::open(data_root)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    /// The store and the display records are captured whole, live; partial
    /// uploads and display details are left out, each with a reason.
    #[test]
    fn the_catalog_captures_the_store_and_display_records_only() {
        let catalog: Value = serde_json::from_slice(&catalog("store.db")).unwrap();
        assert_eq!(catalog["schema_version"], 1);
        assert_eq!(catalog["module_id"], "occipital");
        let entries = catalog["entries"].as_array().unwrap();
        // Engram refuses the whole catalog when one id is outside the module's own.
        assert!(entries.iter().all(|entry| entry["entry_id"]
            .as_str()
            .unwrap()
            .starts_with("occipital/")));
        let (portable, excluded): (Vec<&Value>, Vec<&Value>) = entries
            .iter()
            .partition(|entry| entry["class"] == "portable");
        let paths = |entries: &[&Value]| -> Vec<String> {
            entries
                .iter()
                .map(|entry| entry["path"].as_str().unwrap().to_owned())
                .collect()
        };
        assert_eq!(paths(&portable), ["store.db", "display.db"]);
        for entry in &portable {
            assert_eq!(entry["mechanism"], "whole-db", "{entry}");
            assert_eq!(entry["writer_interaction"], "backup-api-live", "{entry}");
        }
        assert_eq!(paths(&excluded), ["upload-blobs", "display-details.db"]);
        for entry in &excluded {
            assert_eq!(entry["class"], "excluded", "{entry}");
            assert!(!entry["reason"].as_str().unwrap().is_empty());
            assert!(entry.get("mechanism").is_none());
        }
    }

    /// The paths are the ones the module really writes: the store file the
    /// daemon names, the folder uploads keep their partial files in, and the
    /// two display files.
    #[tokio::test]
    async fn the_catalog_paths_are_where_the_module_writes() {
        let dir = cortexkit_test_support::ScratchDir::new("occipital-backup-paths");
        let descriptor = crate::store::tests::descriptor(&dir);
        let store = crate::store::open(&descriptor).unwrap();
        let uploads =
            crate::app_reader::uploads::Uploads::open(store.shared(), store.data_root()).unwrap();
        drop(uploads);
        let display =
            crate::display::store::DisplayStore::open(&descriptor, store.data_root()).unwrap();
        drop(display);
        write_catalog(store.data_root(), store.file_name()).unwrap();
        let catalog: Value =
            serde_json::from_slice(&std::fs::read(store.data_root().join(FILE)).unwrap()).unwrap();
        for entry in catalog["entries"].as_array().unwrap() {
            let path = store.data_root().join(entry["path"].as_str().unwrap());
            assert!(
                path.exists(),
                "{} is not where the module writes",
                path.display()
            );
        }
    }

    #[test]
    fn the_catalog_is_owner_only_and_rewritten_only_when_it_changes() {
        let dir = cortexkit_test_support::ScratchDir::new("occipital-backup-write");
        write_catalog(dir.path(), "store.db").unwrap();
        let path = dir.path().join(FILE);
        assert_eq!(std::fs::read(&path).unwrap(), catalog("store.db"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        // Unchanged bytes leave the file alone; a rewrite would rename a new
        // file into place, with a new inode.
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let inode = std::fs::metadata(&path).unwrap().ino();
            write_catalog(dir.path(), "store.db").unwrap();
            assert_eq!(std::fs::metadata(&path).unwrap().ino(), inode);
        }
        // A changed catalog replaces the old one, through a staged file left
        // over from an earlier write that stopped.
        std::fs::write(dir.path().join(format!(".{FILE}.new")), b"stale").unwrap();
        write_catalog(dir.path(), "other.db").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), catalog("other.db"));
        assert!(!dir.path().join(format!(".{FILE}.new")).exists());
    }
}
