//! What a move has done here, kept per cut in a file of its own, and the sends
//! it holds.
//!
//! Each cut's state is one JSON file, replaced atomically and synced before an
//! op replies, so a restart never lets a send land past a cut or forgets a
//! released copy. A cut's stream waits beside it while the move needs it.
//!
//! The sends a move holds are worked out from the cuts, never stored apart:
//! a source cut that is prepared or exported holds its sessions as moving; a
//! released one holds them as living on the new computer, until a later
//! receiving cut brings the agent back.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use serde::{Deserialize, Serialize};

use super::payload::Session;

/// The folder in the module's data folder that holds every cut.
pub(crate) const DIR: &str = "moves";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Side {
    Source,
    Receiver,
}

/// How far a cut has got here (the contract's step).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Step {
    Prepared,
    Exported,
    Imported,
    Activated,
    Aborted,
    Released,
}

impl Step {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Prepared => "prepared",
            Self::Exported => "exported",
            Self::Imported => "imported",
            Self::Activated => "activated",
            Self::Aborted => "aborted",
            Self::Released => "released",
        }
    }
}

/// The stream a cut carries through the door.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Item {
    pub session_ref: String,
    pub end_offset: u64,
    pub digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Cut {
    pub agent: String,
    /// Absent for a cut aborted before any other op reached it here.
    pub side: Option<Side>,
    pub step: Step,
    /// Absent when the agent had no rows here to carry.
    pub item: Option<Item>,
    /// The sessions the cut moves: held on the source, brought back on the
    /// receiver.
    pub sessions: Vec<Session>,
    /// The computer that holds the sessions after a release.
    pub holder: Option<String>,
    /// The source's `prepare` holds the sessions before it writes and admits
    /// the stream; until both are done, a repeated `prepare` finishes them.
    #[serde(default)]
    pub pending: bool,
    /// When the cut last changed step, which orders a release against a later
    /// move back.
    pub at_ms: i64,
}

/// Why a send to a session is held.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Hold {
    /// The session's agent is moving to another computer now.
    Moving,
    /// The session's agent lives on another computer, named by its holder.
    Elsewhere { holder: String },
}

/// The cuts, in their folder, and the holds they make.
pub struct Cuts {
    dir: PathBuf,
    /// Each held session, with the agent whose move holds it.
    holds: RwLock<HashMap<String, (String, Hold)>>,
}

impl Cuts {
    /// Open the folder in `data_root`, made if missing, and work out the
    /// holds from the cuts already there.
    pub(crate) fn open(data_root: &Path) -> std::io::Result<Self> {
        let dir = data_root.join(DIR);
        private_dir(&dir)?;
        let cuts = Self {
            dir,
            holds: RwLock::new(HashMap::new()),
        };
        cuts.recompute()?;
        Ok(cuts)
    }

    pub(crate) fn load(&self, cut_id: &str) -> std::io::Result<Option<Cut>> {
        match fs::read(self.file(cut_id)) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Replace `cut_id`'s state with `cut`, synced to disk, then update the
    /// holds.
    pub(crate) fn save(&self, cut_id: &str, cut: &Cut) -> std::io::Result<()> {
        let path = self.file(cut_id);
        let temporary = path.with_extension("tmp");
        let mut file = File::create(&temporary)?;
        file.write_all(&serde_json::to_vec(cut)?)?;
        file.sync_all()?;
        fs::rename(&temporary, &path)?;
        File::open(&self.dir)?.sync_all()?;
        self.recompute()
    }

    /// Where a cut's stream waits: written by the source's `prepare`, read
    /// into it by the receiver's `import`.
    pub(crate) fn stream_path(&self, cut_id: &str) -> PathBuf {
        self.dir.join(format!("{cut_id}.stream"))
    }

    /// Why a send to `session` is held, if it is.
    pub fn hold(&self, session: &str) -> Option<Hold> {
        self.holds
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(session)
            .map(|(_, hold)| hold.clone())
    }

    fn file(&self, cut_id: &str) -> PathBuf {
        self.dir.join(format!("{cut_id}.json"))
    }

    /// Work the holds out again from every cut, in the order they changed. A
    /// receiving cut activated after a release brings the agent back, so it
    /// lifts every hold the agent's earlier moves away left: those name the
    /// sessions that moved, which a receiver can't list (only those with rows
    /// travel in the stream).
    fn recompute(&self) -> std::io::Result<()> {
        let mut cuts = Vec::new();
        for entry in fs::read_dir(&self.dir)? {
            let path = entry?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let bytes = fs::read(&path)?;
            let cut: Cut = serde_json::from_slice(&bytes)
                .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
            cuts.push(cut);
        }
        cuts.sort_by_key(|cut| cut.at_ms);
        let mut holds: HashMap<String, (String, Hold)> = HashMap::new();
        for cut in cuts {
            let hold = match (cut.side, cut.step) {
                (Some(Side::Source), Step::Prepared | Step::Exported) => Hold::Moving,
                (Some(Side::Source), Step::Released) => Hold::Elsewhere {
                    holder: cut.holder.clone().unwrap_or_default(),
                },
                (Some(Side::Receiver), Step::Activated) => {
                    holds.retain(|_, (agent, _)| *agent != cut.agent);
                    continue;
                }
                _ => continue,
            };
            for session in &cut.sessions {
                holds.insert(session.session.clone(), (cut.agent.clone(), hold.clone()));
            }
        }
        *self
            .holds
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = holds;
        Ok(())
    }
}

/// Make `dir` if missing, readable only by this user.
fn private_dir(dir: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}
