//! The Pi chat sample set `phone-reader-pi-v1`, which the phone and desktop apps
//! test their decoders against. Its README says what each file shows. The
//! session's entries are Thalamus's recording of a real Pi agent session on its
//! Pi runner (`runner-pages-pi-v2`, see `pi_pages`), served through Occipital,
//! so the set shows a Pi chat in the apps' row shape; `served_samples` says how
//! the set is checked and rewritten.

use super::pi_pages::{messages, V2};
use super::served_samples::{decoded, Served};
use super::*;

/// Routes to the test session, which must be the Pi one.
struct PiHarness(Arc<FakeRoute>);

#[async_trait]
impl BrocaRoutes for PiHarness {
    async fn open(&self, resolved: &Resolved) -> Result<Arc<dyn BrocaRoute>, ReadError> {
        assert_eq!(resolved.harness, "pi");
        Ok(self.0.clone())
    }
}

#[tokio::test]
async fn the_pi_samples_are_what_the_reader_serves() {
    let route = FakeRoute::new();
    *route.entries.lock().unwrap() = messages(V2);
    *route.run_state.lock().unwrap() = Value::Null;
    let app = Arc::new(super::super::super::AppReader::new(Arc::new(
        BrocaSources::new(
            Arc::new(Resolver::new(Arc::new(super::ByHarness(Some(
                PI_RUNNER_MODULE_ID,
            ))))),
            Arc::new(PiHarness(route)),
        ),
    )));
    let mut served = Served::new("phone-reader-pi-v1", app);
    let session = json!({"session": "ses_head", "harness": "pi"});
    let request = |method: &str, extra: Value| {
        let mut params = session.clone();
        for (name, value) in extra.as_object().unwrap() {
            params[name] = value.clone();
        }
        json!({"method": method, "params": params})
    };

    // 01: the newest page. Pi's empty system entry is left out, and every tool
    // call is one app tool part with its result folded in.
    let page_request = request(super::super::super::TRANSCRIPT_PAGE, json!({"limit": 50}));
    let page = served.read(1, &page_request).await;
    let messages = decoded(&page)["result"]["messages"].clone();
    let rows = messages.as_array().unwrap();
    assert!(!rows.is_empty(), "{page:?}");
    assert!(rows.iter().all(|row| row["role"] != "system"), "{messages}");
    let parts = rows
        .iter()
        .flat_map(|row| row["parts"].as_array().unwrap().iter());
    for part in parts {
        assert!(part.get("part").is_none(), "{part}");
        assert!(part["partKey"].is_string(), "{part}");
    }
    let call = rows
        .iter()
        .find(|row| {
            row["parts"]
                .as_array()
                .unwrap()
                .iter()
                .any(|part| part["type"] == "tool")
        })
        .expect("the recording has a tool call")
        .clone();
    served.keep("01-pi-newest-page", &page_request, page);

    // 02: the exact read of the message with the call, whole.
    let exact_request = request(
        super::super::super::TRANSCRIPT_PAGE,
        json!({"messageOrdinal": call["ordinal"]}),
    );
    let exact = served.read(2, &exact_request).await;
    assert_eq!(
        decoded(&exact)["result"]["messages"][0]["messageId"],
        call["messageId"]
    );
    served.keep("02-pi-exact-read", &exact_request, exact);

    // 03: a display read before any watch attached the session's live text: the
    // session is found under Pi's runner, and nothing is held for it yet.
    let display_request = request(super::super::super::DISPLAY_READ, json!({}));
    let display = served.read(3, &display_request).await;
    assert!(decoded(&display)["result"].get("absent").is_some());
    served.keep("03-pi-display-read", &display_request, display);

    served.check();
}
