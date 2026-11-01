//! Serving and checking the sample sets the apps pin, in
//! `occipital-app-reader/test-vectors/`.
//!
//! A sample test sends each request through the app reader's own serving path,
//! as a phone's route would, and keeps the reply bytes the app receives. It then
//! compares every file with the committed set, so the reader can't drift from
//! what the apps pinned. With `OCCIPITAL_WRITE_SAMPLES` set to a folder, it
//! writes the set into `<folder>/<set name>/` instead. A published set never
//! changes: a reader change that alters its bytes needs a new set under a new
//! name.

use super::*;
use std::path::PathBuf;
use subc_protocol::{BindIdentity, Flags, Frame, FrameType, Principal, Priority};

/// A phone's route, as Callosum binds it.
fn phone() -> super::super::super::RouteBinding {
    (
        Some(Principal::Reserved {
            module_id: "callosum".into(),
        }),
        RouteTarget::ManagementSurface {
            module_id: crate::MODULE_ID.into(),
        },
        None,
        Some(BindIdentity::new("fed-phone:test", "caller")),
        None,
    )
}

pub(super) fn decoded(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).unwrap()
}

/// One sample set being served: the requests sent so far and the reply bytes
/// kept for each.
pub(super) struct Served {
    set: &'static str,
    connection: super::super::super::tests::Connection,
    tx: mpsc::Sender<Frame>,
    rx: mpsc::Receiver<Frame>,
    files: Vec<(String, Vec<u8>)>,
}

impl Served {
    pub(super) fn new(set: &'static str, app: Arc<super::super::super::AppReader>) -> Self {
        let (tx, rx) = mpsc::channel(64);
        Self {
            set,
            connection: super::super::super::tests::Connection::new(app),
            tx,
            rx,
            files: Vec::new(),
        }
    }

    /// Send `request` (`{method, params}`) as request `corr` on the phone's route.
    pub(super) async fn send(&mut self, corr: u64, request: &Value) {
        let frame = Frame::build(
            FrameType::Request,
            Flags::new(false, Priority::Interactive, false),
            1,
            1,
            corr,
            request.to_string().into_bytes(),
        )
        .unwrap();
        self.connection
            .dispatch(
                frame,
                Some(phone()),
                request["method"].as_str().unwrap().into(),
                request["params"].clone(),
                None,
                &self.tx,
            )
            .await
            .unwrap();
    }

    /// The next frame for request `corr`, as the app receives it, or `None` if
    /// none comes within `wait`.
    pub(super) async fn next(&mut self, corr: u64, wait: Duration) -> Option<(FrameType, Vec<u8>)> {
        let rx = &mut self.rx;
        tokio::time::timeout(wait, async {
            loop {
                let frame = rx.recv().await.unwrap();
                if frame.header.corr == corr {
                    return (frame.header.ty, frame.body);
                }
            }
        })
        .await
        .ok()
    }

    /// The app's Cancel for request `corr`.
    pub(super) fn cancel(&mut self, corr: u64) {
        self.connection.cancel((1, 1, corr));
    }

    /// Send `request` and return its one reply, which must be a response.
    pub(super) async fn read(&mut self, corr: u64, request: &Value) -> Vec<u8> {
        self.send(corr, request).await;
        let (ty, reply) = self.next(corr, Duration::from_secs(5)).await.unwrap();
        assert_eq!(
            ty,
            FrameType::Response,
            "{}",
            String::from_utf8_lossy(&reply)
        );
        reply
    }

    /// Keep `reply` as the sample `name`, beside the request that produced it.
    pub(super) fn keep(&mut self, name: &str, request: &Value, reply: Vec<u8>) {
        self.files.push((
            format!("{name}.request.json"),
            request.to_string().into_bytes(),
        ));
        self.files.push((format!("{name}.json"), reply));
    }

    /// Compare every kept file and the checksums with the committed set, or
    /// write the set when `OCCIPITAL_WRITE_SAMPLES` names a folder.
    pub(super) fn check(mut self) {
        self.files.sort();
        let sums: String = self
            .files
            .iter()
            .map(|(name, bytes)| format!("{:x}  {name}\n", Sha256::digest(bytes)))
            .collect();
        if let Some(folder) = std::env::var_os("OCCIPITAL_WRITE_SAMPLES") {
            let folder = PathBuf::from(folder).join(self.set);
            std::fs::create_dir_all(&folder).unwrap();
            for (name, bytes) in &self.files {
                std::fs::write(folder.join(name), bytes).unwrap();
            }
            std::fs::write(folder.join("SHA256SUMS"), &sums).unwrap();
            return;
        }
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../occipital-app-reader/test-vectors")
            .join(self.set);
        for (name, bytes) in &self.files {
            assert_eq!(
                &std::fs::read(root.join(name)).unwrap(),
                bytes,
                "{name} is no longer what the reader serves"
            );
        }
        assert_eq!(
            std::fs::read_to_string(root.join("SHA256SUMS")).unwrap(),
            sums
        );
    }
}
