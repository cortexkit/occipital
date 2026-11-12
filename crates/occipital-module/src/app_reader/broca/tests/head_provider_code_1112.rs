//! Broca's `session.head` gaining `last_run_error.provider_code`, the optional
//! member runner contract 0.8.43's `LastRunError` defines, on both of Broca's
//! session stores ("legacy" and "store" in the file names). Real serve-path
//! bodies before and after, copied into `tests/data/broca-head-provider-code-1112/`
//! with the README and every body's checksum line; Broca's build logs beside
//! them in its set were left out.
//!
//! Each body's run state and error go through the module's handlers on newest
//! pages, claims, details and watches. A named code reaches the apps as
//! `lastRun.error.providerCode`; before the change, and for an error with no
//! code, `lastRun.error` carries the class and timing only.

use super::*;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/broca-head-provider-code-1112")
}

fn head(side: &str, name: &str) -> Value {
    let path = root().join(side).join(format!("{name}.json"));
    serde_json::from_slice::<Value>(&std::fs::read(path).unwrap()).unwrap()["result"].clone()
}

#[test]
fn the_head_provider_code_samples_keep_their_checksums() {
    let sums = std::fs::read_to_string(root().join("SHA256SUMS")).unwrap();
    // Sixteen bodies and the README.
    assert_eq!(sums.lines().count(), 17);
    for line in sums.lines() {
        let (expected, name) = line.split_once("  ").unwrap();
        let bytes = std::fs::read(root().join(name)).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), expected, "{name}");
    }
}

#[tokio::test]
async fn a_head_naming_its_run_error_code_reaches_the_apps() {
    for storage in ["legacy", "store"] {
        for (case, code, timing) in [
            ("billing_exhausted", Some("billing_exhausted"), None),
            (
                "account_limit_exhausted",
                Some("account_limit_exhausted"),
                Some((120.5, 1_800_000_120_500_u64)),
            ),
            ("permanent_no_code", None, None),
            ("completed", None, None),
        ] {
            for side in ["before", "after"] {
                let body = head(side, &format!("{storage}.{case}"));
                let state = body["last_run_state"].clone();
                let error = body.get("last_run_error").cloned();
                let mut expected = json!({"runId": state["run_id"], "state": state["state"],
                    "reason": state["reason"]});
                if let Some(error) = &error {
                    let mut app = json!({"class": error["class"]});
                    if let Some((retry, resets)) = timing {
                        app["retryAfterSecs"] = json!(retry);
                        app["resetsAtMs"] = json!(resets);
                    }
                    if let Some(code) = code.filter(|_| side == "after") {
                        app["providerCode"] = json!(code);
                    }
                    expected["error"] = app;
                } else {
                    assert_eq!(case, "completed", "{side} {storage}.{case}");
                }
                super::session_facts::assert_last_run_phone_bytes(
                    state,
                    error,
                    &serde_json::to_string(&expected).unwrap(),
                )
                .await;
            }
        }
    }
}
