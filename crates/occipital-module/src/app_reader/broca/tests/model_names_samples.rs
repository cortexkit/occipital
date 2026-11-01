//! The sample set `phone-reader-model-names-v1`, which the phone and desktop
//! apps test their decoders against at its tag. Its README says what each
//! file shows. The names come from fusiform's and
//! routing's own sample replies (`display/model_names/fixtures`), served
//! through the phone's route. `served_samples` says how the set is checked and
//! rewritten.

use super::served_samples::{decoded, Served};
use super::*;
use crate::display::model_names::tests::{fixture, Fake};
use crate::display::model_names::{ModelNames, OPERATION};
use subc_protocol::FrameType;

/// Model names read no session, so no session has a route.
struct NoSessions;

#[async_trait]
impl BrocaRoutes for NoSessions {
    async fn open(&self, _: &Resolved) -> Result<Arc<dyn BrocaRoute>, ReadError> {
        Err(ReadError::new("session_unknown", "no session in this test"))
    }
}

#[tokio::test]
async fn the_model_name_samples_are_what_the_reader_serves() {
    let fake = Arc::new(Fake::default());
    fake.catalog();
    fake.targets(Ok(fixture("routing-resolved.json")));
    let app = Arc::new(super::super::super::AppReader::new(Arc::new(
        BrocaSources::new(
            Arc::new(Resolver::new(Arc::new(super::ByHarness(None)))),
            Arc::new(NoSessions),
        ),
    )));
    app.set_model_names(Arc::new(ModelNames::new(fake.clone())));
    let mut served = Served::new("phone-reader-model-names-v1", app);

    let request = json!({"method": OPERATION, "params": {"ids": [
        "anthropic/claude-sonnet-4-5",
        "openai/gpt-6.1-sol:xhigh",
        "anthropic-platform/claude-sonnet-4-5:high",
        "cloudflare-workers-ai/@cf/cloudflare/clef",
        "claude-sonnet-4-5",
        "poe/google/veo-3",
    ]}});
    let reply = served.read(1, &request).await;
    let models = &decoded(&reply)["result"]["models"];
    assert_eq!(
        models["openai/gpt-6.1-sol:xhigh"]["variant"], "xhigh",
        "{models}"
    );
    assert_eq!(models["claude-sonnet-4-5"], json!({}));
    served.keep("01-names", &request, reply);

    let too_many: Vec<String> = (0..65).map(|n| format!("openai/model-{n}")).collect();
    let refused_request = json!({"method": OPERATION, "params": {"ids": too_many}});
    served.send(2, &refused_request).await;
    let (ty, refused) = served.next(2, Duration::from_secs(5)).await.unwrap();
    assert_eq!(
        ty,
        FrameType::Error,
        "{}",
        String::from_utf8_lossy(&refused)
    );
    served.keep("02-too-many-ids", &refused_request, refused);

    served.check();
}
