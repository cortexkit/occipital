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

use std::collections::{BTreeMap, HashMap};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    RwLock,
};

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
    Activating,
    Activated,
    Aborted,
    Released,
}

impl Step {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Prepared => "prepared",
            Self::Exported => "exported",
            Self::Imported | Self::Activating => "imported",
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
    /// The release's `holder`: a pseudonym for the new holder, kept only so a
    /// repeat that names another is refused. It's never shown.
    pub holder: Option<String>,
    /// The new computer's box id, when the release named it (`holderBox`).
    /// A refused send names the computer only from this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub holder_box: Option<String>,
    /// The source's `prepare` holds the sessions before it writes and admits
    /// the stream; until both are done, a repeated `prepare` finishes them.
    #[serde(default)]
    pub pending: bool,
    /// Wall-clock timestamp retained for diagnostics and legacy ordering.
    pub at_ms: i64,
}

/// Why a send to a session is held.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Hold {
    /// The session's agent is moving to another computer now.
    Moving,
    /// The session's agent lives on another computer: `holder` is its box id,
    /// or empty when the release named none.
    Elsewhere { holder: String },
}

/// The cuts, in their folder, and the holds they make.
pub struct Cuts {
    dir: PathBuf,
    state: RwLock<State>,
    readable: AtomicBool,
    completed: tokio::sync::Notify,
}

#[derive(Default)]
struct State {
    in_flight: HashMap<String, usize>,
    ordered: BTreeMap<u64, (PathBuf, Cut)>,
    holds: HashMap<String, (String, Hold)>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Saved {
    version: u32,
    sequence: u64,
    cut: Cut,
}

fn invalid(reason: impl ToString) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, reason.to_string())
}

impl Saved {
    fn parse(bytes: &[u8]) -> std::io::Result<Self> {
        let saved: Self = serde_json::from_slice(bytes).map_err(invalid)?;
        if saved.version != 2 || saved.sequence == 0 {
            return Err(invalid("unsupported move record version or sequence"));
        }
        Ok(saved)
    }
    fn write(&self, path: &Path) -> std::io::Result<()> {
        let temporary = path.with_extension("tmp");
        let mut file = File::create(&temporary)?;
        file.write_all(&serde_json::to_vec(self)?)?;
        file.sync_all()?;
        fs::rename(temporary, path)?;
        crate::durable::sync_dir(
            path.parent()
                .ok_or_else(|| invalid("missing move folder"))?,
        )
    }
}

impl Cuts {
    pub(crate) fn unopened(data_root: &Path) -> Self {
        Self {
            dir: data_root.join(DIR),
            state: Default::default(),
            readable: AtomicBool::new(false),
            completed: Default::default(),
        }
    }

    #[cfg(test)]
    pub(crate) fn open(data_root: &Path) -> std::io::Result<Self> {
        let cuts = Self::unopened(data_root);
        cuts.recover()?;
        Ok(cuts)
    }

    pub(crate) fn readable(&self) -> bool {
        self.readable.load(Ordering::Acquire)
    }

    pub(crate) fn recover(&self) -> std::io::Result<()> {
        let mut state = self.state.write().unwrap_or_else(|p| p.into_inner());
        if self.readable() {
            return Ok(());
        }
        private_dir(&self.dir)?;
        let mut ordered = BTreeMap::new();
        let mut legacy = Vec::new();
        for entry in fs::read_dir(&self.dir)? {
            let path = entry?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let bytes = fs::read(&path)?;
            match Saved::parse(&bytes) {
                Ok(saved) => {
                    if ordered.insert(saved.sequence, (path, saved.cut)).is_some() {
                        return Err(invalid("duplicate move sequence"));
                    }
                }
                Err(_) => {
                    let cut: Cut = serde_json::from_slice(&bytes).map_err(invalid)?;
                    if cut.step == Step::Activating {
                        return Err(invalid("unversioned activating move record"));
                    }
                    legacy.push((path, cut));
                }
            }
        }
        if !legacy.is_empty() {
            self.back_up(&legacy)?;
            legacy.sort_by(|(a, x), (b, y)| (x.at_ms, a).cmp(&(y.at_ms, b)));
            for (path, cut) in legacy {
                let sequence = next(&ordered)?;
                Saved {
                    version: 2,
                    sequence,
                    cut: cut.clone(),
                }
                .write(&path)?;
                ordered.insert(sequence, (path, cut));
            }
        }
        state.ordered = ordered;
        state.recompute();
        self.readable.store(true, Ordering::Release);
        Ok(())
    }

    fn back_up(&self, legacy: &[(PathBuf, Cut)]) -> std::io::Result<()> {
        let backup = self.dir.join("pre-v2");
        if backup.is_dir() {
            return Ok(());
        }
        // Publish the complete backup before changing the first file. After
        // a crash, a partial temporary copy is safe to rebuild.
        let temp = self.dir.join("pre-v2.tmp");
        if temp.exists() {
            fs::remove_dir_all(&temp)?;
        }
        private_dir(&temp)?;
        for (path, _) in legacy {
            let to = temp.join(
                path.file_name()
                    .ok_or_else(|| invalid("missing cut filename"))?,
            );
            fs::copy(path, &to)?;
            crate::durable::sync_file(&to)?;
        }
        crate::durable::sync_dir(&temp)?;
        fs::rename(temp, backup)?;
        crate::durable::sync_dir(&self.dir)
    }

    pub(crate) fn load(&self, cut_id: &str) -> std::io::Result<Option<Cut>> {
        if !self.readable() {
            return Err(invalid("move records unreadable"));
        }
        match fs::read(self.file(cut_id)) {
            Ok(bytes) => Ok(Some(Saved::parse(&bytes)?.cut)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    pub(crate) fn save(&self, cut_id: &str, cut: &Cut) -> std::io::Result<()> {
        let mut state = self.state.write().unwrap_or_else(|p| p.into_inner());
        if !self.readable() {
            return Err(invalid("move records unreadable"));
        }
        let sequence = next(&state.ordered)?;
        let path = self.file(cut_id);
        let saved = Saved {
            version: 2,
            sequence,
            cut: cut.clone(),
        };
        if let Err(error) = saved.write(&path) {
            self.readable.store(false, Ordering::Release);
            return Err(error);
        }
        state.ordered.retain(|_, (p, _)| p != &path);
        state.ordered.insert(sequence, (path, saved.cut));
        state.recompute();
        Ok(())
    }

    pub(crate) fn check_readable(&self) -> Result<(), occipital_app_reader::send::SendRefusal> {
        if self.readable() {
            Ok(())
        } else {
            Err(occipital_app_reader::send::SendRefusal {
                code: "provider_unavailable",
                message: "the move state is unreadable; retry once the move records can be read"
                    .into(),
                detail: Some(serde_json::json!({"retryable":true})),
            })
        }
    }

    pub(crate) fn admit(
        self: &std::sync::Arc<Self>,
        session: &str,
    ) -> Result<Admitted, occipital_app_reader::send::SendRefusal> {
        let mut state = self.state.write().unwrap_or_else(|p| p.into_inner());
        self.check_readable()?;
        if let Some((_, hold)) = state.holds.get(session) {
            return Err(super::refusal(hold));
        }
        *state.in_flight.entry(session.into()).or_default() += 1;
        Ok(Admitted {
            cuts: self.clone(),
            session: session.into(),
        })
    }

    pub(crate) async fn drain(&self, sessions: &[Session]) -> bool {
        tokio::time::timeout(std::time::Duration::from_secs(30), async {
            loop {
                let changed = self.completed.notified();
                if !sessions.iter().any(|s| {
                    self.state
                        .read()
                        .unwrap_or_else(|p| p.into_inner())
                        .in_flight
                        .contains_key(&s.session)
                }) {
                    break;
                }
                changed.await;
            }
        })
        .await
        .is_ok()
    }

    /// Where a cut's stream waits: written by the source's `prepare`, read
    /// into it by the receiver's `import`.
    pub(crate) fn stream_path(&self, cut_id: &str) -> PathBuf {
        self.dir.join(format!("{cut_id}.stream"))
    }

    /// Why a send to `session` is held, if it is.
    pub fn hold(&self, session: &str) -> Option<Hold> {
        self.state
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .holds
            .get(session)
            .map(|(_, hold)| hold.clone())
    }

    fn file(&self, cut_id: &str) -> PathBuf {
        self.dir.join(format!("{cut_id}.json"))
    }
}

fn next(ordered: &BTreeMap<u64, (PathBuf, Cut)>) -> std::io::Result<u64> {
    ordered
        .keys()
        .next_back()
        .copied()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| invalid("move sequence exhausted"))
}

impl State {
    fn recompute(&mut self) {
        let mut holds: HashMap<String, (String, Hold)> = HashMap::new();
        for (_, cut) in self.ordered.values() {
            let hold = match (cut.side, cut.step) {
                (Some(Side::Source), Step::Prepared | Step::Exported) => Hold::Moving,
                (Some(Side::Source), Step::Released) => Hold::Elsewhere {
                    holder: cut.holder_box.clone().unwrap_or_default(),
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
        self.holds = holds;
    }
}

/// Kept until the send has stored its result, or returned a refusal.
pub(crate) struct Admitted {
    cuts: std::sync::Arc<Cuts>,
    session: String,
}
impl Drop for Admitted {
    fn drop(&mut self) {
        let mut state = self.cuts.state.write().unwrap_or_else(|p| p.into_inner());
        if let Some(n) = state.in_flight.get_mut(&self.session) {
            *n -= 1;
            if *n == 0 {
                state.in_flight.remove(&self.session);
                self.cuts.completed.notify_waiters();
            }
        }
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

#[cfg(test)]
mod tests;
