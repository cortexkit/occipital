use super::*;
use std::sync::Mutex;

fn reserved(module: &str) -> Principal {
    Principal::Reserved {
        module_id: module.into(),
    }
}

/// The `display.write` params AFT would have pushed for one record.
fn write() -> Value {
    json!({
        "runner": serde_json::to_value(reserved("broca")).unwrap(),
        "call_key": "call-1",
        "kind": "record",
        "schema_pin": "tp1:bash:0123:1",
        "body": {"durationMs": 5, "outcome": "success"},
    })
}

struct Fake {
    verified: Result<bool, String>,
    reply: Result<Value, CallFailure>,
    read: Mutex<Vec<(String, String)>>,
}

impl Fake {
    fn new(verified: Result<bool, String>, reply: Result<Value, CallFailure>) -> Self {
        Self {
            verified,
            reply,
            read: Mutex::default(),
        }
    }
}

#[async_trait]
impl Providers for Fake {
    async fn verified(&self, _: &str) -> Result<bool, String> {
        self.verified.clone()
    }

    async fn read(&self, module: &str, record_id: &str) -> Result<Value, CallFailure> {
        self.read
            .lock()
            .unwrap()
            .push((module.into(), record_id.into()));
        self.reply.clone()
    }
}

fn refusal(code: Option<&str>) -> CallFailure {
    CallFailure {
        code: code.map(str::to_owned),
        message: "refused".into(),
    }
}

fn open(
    dir: &cortexkit_test_support::ScratchDir,
) -> (Sink, Arc<super::super::store::DisplayStore>) {
    let descriptor = crate::store::tests::descriptor(dir);
    let store = Arc::new(super::super::store::DisplayStore::open(&descriptor, dir.path()).unwrap());
    (
        Sink {
            state: Some(State::Open(store.clone())),
            views: None,
            hints: None,
            notices: None,
        },
        store,
    )
}

/// A verified provider's record is stored as that provider's, the module this
/// module called, and the same record again is stored once.
#[tokio::test]
async fn a_verified_providers_record_is_stored_as_its_own() {
    let dir = cortexkit_test_support::ScratchDir::new("occipital-display-pull");
    let (sink, store) = open(&dir);
    let fake = Fake::new(Ok(true), Ok(write()));
    assert_eq!(pull(&fake, &sink, "aft", "rec-1").await, Pulled::Stored);
    assert_eq!(pull(&fake, &sink, "aft", "rec-1").await, Pulled::Stored);
    assert_eq!(
        *fake.read.lock().unwrap(),
        vec![("aft".to_owned(), "rec-1".to_owned()); 2]
    );
    let stored = store.writes(&reserved("broca"), "call-1").unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].writer, reserved("aft"));
}

/// A module the catalog doesn't list as daemon-verified is never read.
#[tokio::test]
async fn an_unverified_provider_is_never_read() {
    let dir = cortexkit_test_support::ScratchDir::new("occipital-display-pull-unverified");
    let (sink, store) = open(&dir);
    let fake = Fake::new(Ok(false), Ok(write()));
    assert!(matches!(
        pull(&fake, &sink, "aft", "rec-1").await,
        Pulled::Final(_)
    ));
    assert!(fake.read.lock().unwrap().is_empty());
    assert!(store
        .writes(&reserved("broca"), "call-1")
        .unwrap()
        .is_empty());
    // A catalog that can't be read is asked again later.
    let fake = Fake::new(Err("down".into()), Ok(write()));
    assert!(matches!(
        pull(&fake, &sink, "aft", "rec-1").await,
        Pulled::Retry(_)
    ));
}

/// The contract's refusals are final; a provider not ready, a route that
/// couldn't open, or a reply that was lost is tried again.
#[tokio::test]
async fn refusals_are_final_and_unreachable_providers_are_tried_again() {
    let dir = cortexkit_test_support::ScratchDir::new("occipital-display-pull-refused");
    let (sink, _store) = open(&dir);
    for code in [
        Some("display_record_unknown"),
        Some("display_read_denied"),
        Some("invalid_params"),
    ] {
        let fake = Fake::new(Ok(true), Err(refusal(code)));
        assert!(
            matches!(pull(&fake, &sink, "aft", "rec-1").await, Pulled::Final(_)),
            "{code:?}"
        );
    }
    for code in [Some("module_warming"), Some("target_unavailable"), None] {
        let fake = Fake::new(Ok(true), Err(refusal(code)));
        assert!(
            matches!(pull(&fake, &sink, "aft", "rec-1").await, Pulled::Retry(_)),
            "{code:?}"
        );
    }
}

/// A reply that isn't a write can never be stored, while a store that isn't
/// open yet keeps the record waiting.
#[tokio::test]
async fn a_malformed_reply_is_final_and_a_closed_store_waits() {
    let dir = cortexkit_test_support::ScratchDir::new("occipital-display-pull-malformed");
    let (sink, _store) = open(&dir);
    let mut malformed = write();
    malformed.as_object_mut().unwrap().remove("call_key");
    let fake = Fake::new(Ok(true), Ok(malformed));
    assert!(matches!(
        pull(&fake, &sink, "aft", "rec-1").await,
        Pulled::Final(_)
    ));
    let closed = Sink {
        state: None,
        views: None,
        hints: None,
        notices: None,
    };
    let fake = Fake::new(Ok(true), Ok(write()));
    assert!(matches!(
        pull(&fake, &closed, "aft", "rec-1").await,
        Pulled::Retry(_)
    ));
}

/// The verified list is read from the catalog once and kept while the
/// catalog's generation stays; a move reads it again, and a failed read keeps
/// nothing.
#[tokio::test(start_paused = true)]
async fn the_verified_list_is_read_once_per_catalog_generation() {
    let (set, generation) = CatalogGeneration::manual();
    set.send_replace(Some((1, 7)));
    let kept = VerifiedModules::default();
    let reads = Mutex::new(0);
    let read = |modules: &'static [&'static str]| {
        let reads = &reads;
        async move {
            *reads.lock().unwrap() += 1;
            Ok(modules.iter().map(|module| (*module).to_owned()).collect())
        }
    };
    assert_eq!(
        kept.contains("aft", &generation, read(&["aft"])).await,
        Ok(true)
    );
    // While the generation stays, the kept list answers and no read runs.
    assert_eq!(kept.contains("aft", &generation, read(&[])).await, Ok(true));
    assert_eq!(
        kept.contains("core", &generation, read(&["core"])).await,
        Ok(false)
    );
    assert_eq!(*reads.lock().unwrap(), 1);
    set.send_replace(Some((1, 8)));
    assert_eq!(
        kept.contains("aft", &generation, read(&[])).await,
        Ok(false)
    );
    assert_eq!(*reads.lock().unwrap(), 2);
    let failed = std::future::ready(Err("down".to_owned()));
    set.send_replace(Some((1, 9)));
    assert_eq!(
        kept.contains("aft", &generation, failed).await,
        Err("down".into())
    );
    assert_eq!(
        kept.contains("aft", &generation, read(&["aft"])).await,
        Ok(true)
    );
    assert_eq!(*reads.lock().unwrap(), 3);
}
