//! Activity tokens and a newest-page read that retries once if its watermark changes.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Mutex,
};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug)]
pub struct ActivityTokens {
    transcript_activity_watermark_seq: AtomicU64,
    transcript_activity_boot_nonce: u32,
    transcript_activity_watermarks: Mutex<HashMap<String, String>>,
}

impl Default for ActivityTokens {
    fn default() -> Self {
        // Mix subsecond clock bits and the PID into the default boot nonce.
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        Self::new(now.subsec_nanos() ^ std::process::id().rotate_left(16))
    }
}

impl ActivityTokens {
    /// Callers can supply their own boot nonce instead of using the default.
    pub fn new(boot_nonce: u32) -> Self {
        Self {
            transcript_activity_watermark_seq: AtomicU64::new(0),
            transcript_activity_boot_nonce: boot_nonce,
            transcript_activity_watermarks: Mutex::new(HashMap::new()),
        }
    }

    pub fn next_transcript_activity_token(&self) -> String {
        let sequence = self
            .transcript_activity_watermark_seq
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_add(1);
        // Clients persist tokens across restarts, but the sequence starts at zero again.
        // Including a distinct boot nonce prevents those old tokens from matching a fresh
        // sequence and incorrectly answering `unchanged` without reading the page.
        let nonce = self.transcript_activity_boot_nonce;
        format!("activity-{nonce:08x}-{sequence}")
    }

    pub fn note_transcript_activity(&self, session_id: &str) {
        let token = self.next_transcript_activity_token();
        self.transcript_activity_watermarks
            .lock()
            .expect("transcript activity watermarks poisoned")
            .insert(session_id.to_string(), token);
    }

    pub fn current_transcript_activity_token(&self, session_id: &str) -> Option<String> {
        self.transcript_activity_watermarks
            .lock()
            .expect("transcript activity watermarks poisoned")
            .get(session_id)
            .cloned()
    }

    pub fn transcript_activity_token_matches(&self, session_id: &str, token: &str) -> bool {
        self.current_transcript_activity_token(session_id)
            .as_deref()
            == Some(token)
    }

    pub fn capture_transcript_activity_observation(
        &self,
        session_id: &str,
    ) -> TranscriptActivityObservation {
        let watermark = self.current_transcript_activity_token(session_id);
        let captured_token = watermark
            .clone()
            .unwrap_or_else(|| self.next_transcript_activity_token());
        TranscriptActivityObservation {
            watermark,
            captured_token,
        }
    }

    pub fn commit_stable_transcript_activity_token(
        &self,
        session_id: &str,
        before: &TranscriptActivityObservation,
        after_watermark: Option<&str>,
    ) -> Option<String> {
        if before.watermark.as_deref() != after_watermark {
            return None;
        }
        if before.watermark.is_none() {
            let mut watermarks = self
                .transcript_activity_watermarks
                .lock()
                .expect("transcript activity watermarks poisoned");
            if !watermarks.contains_key(session_id) {
                // A quiet session starts with no event-fed watermark. Publish its captured token
                // only after the event watermark stays unchanged across the provider read.
                watermarks.insert(session_id.to_string(), before.captured_token.clone());
            }
        }
        Some(before.captured_token.clone())
    }

    pub async fn read_newest_transcript_page_stably<T, E, Read, ReadFuture>(
        &self,
        session_id: &str,
        mut read_page: Read,
    ) -> Result<StableTranscriptPage<T>, E>
    where
        Read: FnMut() -> ReadFuture,
        ReadFuture: std::future::Future<Output = Result<T, E>>,
    {
        let mut before = self.capture_transcript_activity_observation(session_id);
        for provider_calls in 1..=2 {
            let page = read_page().await?;
            let after = self.current_transcript_activity_token(session_id);
            if let Some(activity_token) =
                self.commit_stable_transcript_activity_token(session_id, &before, after.as_deref())
            {
                return Ok(StableTranscriptPage {
                    page,
                    activity_token: Some(activity_token),
                    provider_calls,
                });
            }
            if provider_calls == 2 {
                self.invalidate_transcript_activity_token(session_id);
                self.note_second_transcript_page_instability(session_id);
                return Ok(StableTranscriptPage {
                    page,
                    activity_token: None,
                    provider_calls,
                });
            }
            before = self.capture_transcript_activity_observation(session_id);
        }
        unreachable!("the transcript page observation loop has exactly two attempts")
    }

    pub fn invalidate_transcript_activity_token(&self, session_id: &str) {
        self.transcript_activity_watermarks
            .lock()
            .expect("transcript activity watermarks poisoned")
            .remove(session_id);
    }

    pub fn note_second_transcript_page_instability(&self, session_id: &str) {
        let count = TRANSCRIPT_PAGE_SECOND_INSTABILITY_COUNT
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_add(1);
        static LAST_LOG_AT: std::sync::OnceLock<Mutex<HashMap<String, i64>>> =
            std::sync::OnceLock::new();
        let now = now_ms();
        let mut last_log_at = LAST_LOG_AT
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .expect("transcript instability log throttle poisoned");
        let should_log = last_log_at.get(session_id).is_none_or(|last| {
            now.saturating_sub(*last) >= TRANSCRIPT_PAGE_INSTABILITY_LOG_INTERVAL_MS
        });
        if !should_log {
            return;
        }
        if last_log_at.len() >= TRANSCRIPT_PAGE_INSTABILITY_LOG_SESSION_CAP
            && !last_log_at.contains_key(session_id)
        {
            if let Some(oldest_session) = last_log_at
                .iter()
                .min_by_key(|(_, at)| *at)
                .map(|(session, _)| session.clone())
            {
                last_log_at.remove(&oldest_session);
            }
        }
        last_log_at.insert(session_id.to_string(), now);
        tracing::warn!(
            session = session_id,
            total = count,
            "unstable transcript page requested activity-token discard"
        );
    }

    /// Provider-owned JSON must not supply the server's change-probe metadata.
    pub fn strip_transcript_activity_token_metadata(reply: &mut Value) {
        if let Some(object) = reply.as_object_mut() {
            object.remove("activityToken");
            object.remove("discardActivityToken");
        }
    }

    pub fn attach_captured_transcript_activity_token(
        &self,
        _session_id: &str,
        activity_token: Option<&str>,
        reply: &mut Value,
    ) {
        Self::strip_transcript_activity_token_metadata(reply);
        if let Some(object) = reply.as_object_mut() {
            // Only a page whose watermark changed during both reads requests token discard.
            // A stable page carries its captured token and cannot inherit a discard signal from provider-owned JSON.
            if let Some(token) = activity_token {
                object.insert("activityToken".to_string(), json!(token));
            }
        }
    }

    pub fn attach_unstable_transcript_activity_discard(&self, session_id: &str, reply: &mut Value) {
        self.attach_captured_transcript_activity_token(session_id, None, reply);
        if let Some(object) = reply.as_object_mut() {
            // Transcript activity changed during both read attempts, so do not offer the
            // resulting page's version as a reliable baseline for the client's next claim.
            object.remove("version");
            // `discardActivityToken: true` explicitly tells the client to discard its stored token.
            // An explicit null decodes like a missing Optional field and cannot reliably request removal.
            object.insert("discardActivityToken".to_string(), json!(true));
        }
    }

    pub fn ensure_transcript_version(&self, session_id: &str) -> String {
        if self.current_transcript_activity_token(session_id).is_none() {
            self.note_transcript_activity(session_id);
        }
        self.current_transcript_activity_token(session_id)
            .expect("a transcript version was just minted")
    }
}

const TRANSCRIPT_PAGE_INSTABILITY_LOG_INTERVAL_MS: i64 = 60 * 60 * 1_000;

const TRANSCRIPT_PAGE_INSTABILITY_LOG_SESSION_CAP: usize = 4_096;

static TRANSCRIPT_PAGE_SECOND_INSTABILITY_COUNT: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptActivityObservation {
    watermark: Option<String>,
    captured_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StableTranscriptPage<T> {
    pub page: T,
    pub activity_token: Option<String>,
    pub provider_calls: u64,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn quiet_sessions_publish_only_a_stable_captured_token_and_retry_once() {
        let tokens = ActivityTokens::new(7);
        let mut calls = 0;
        let stable = tokens
            .read_newest_transcript_page_stably("quiet", || {
                calls += 1;
                std::future::ready(Ok::<_, ()>(calls))
            })
            .await
            .unwrap();
        assert_eq!(stable.page, 1);
        assert_eq!(stable.provider_calls, 1);
        assert_eq!(
            stable.activity_token,
            tokens.current_transcript_activity_token("quiet")
        );
        assert!(stable.activity_token.is_some());
        let mut calls = 0;
        let retried = tokens
            .read_newest_transcript_page_stably("moving", || {
                calls += 1;
                if calls == 1 {
                    tokens.note_transcript_activity("moving");
                }
                std::future::ready(Ok::<_, ()>(calls))
            })
            .await
            .unwrap();
        assert_eq!(retried.page, 2);
        assert_eq!(retried.provider_calls, 2);
        assert!(retried.activity_token.is_some());
        let mut calls = 0;
        assert_eq!(
            tokens
                .read_newest_transcript_page_stably("error", || {
                    calls += 1;
                    std::future::ready(Err::<(), _>("provider error"))
                })
                .await
                .unwrap_err(),
            "provider error"
        );
        assert_eq!(calls, 1);
        assert_eq!(tokens.current_transcript_activity_token("error"), None);
    }
    #[tokio::test]
    async fn transcript_page_captured_token_stays_older_after_the_stable_fence() {
        let runtime = ActivityTokens::new(1);
        let session_id = "ses_captured_older";
        runtime.note_transcript_activity(session_id);
        let captured = runtime
            .read_newest_transcript_page_stably(session_id, || {
                std::future::ready(Ok::<_, ()>(json!({ "messages": ["stable-page"] })))
            })
            .await
            .unwrap();
        let captured_token = captured.activity_token.clone().unwrap();

        runtime.note_transcript_activity(session_id);
        let later_token = runtime
            .current_transcript_activity_token(session_id)
            .expect("the later event advances the current token");
        let mut reply = captured.page;
        runtime.attach_captured_transcript_activity_token(
            session_id,
            captured.activity_token.as_deref(),
            &mut reply,
        );

        assert_eq!(reply["activityToken"], json!(captured_token));
        assert_ne!(reply["activityToken"], json!(later_token));
        assert!(reply.get("discardActivityToken").is_none());
    }

    #[tokio::test]
    async fn persisted_client_token_on_same_runtime_cannot_replay_after_discard() {
        let runtime = ActivityTokens::new(1);
        let session_id = "ses_persisted_same_runtime";

        // Model the original race: the provider fixes the page bytes, an event advances the
        // watermark, and the stale page is incorrectly labeled with that post-event token.
        let page_before_event = "page-before-event";
        runtime.note_transcript_activity(session_id);
        let persisted_stale_page_token = runtime
            .current_transcript_activity_token(session_id)
            .expect("the injected torn stamp uses the post-read token");
        let mislabeled_reply = json!({
            "messages": [page_before_event],
            "activityToken": persisted_stale_page_token,
        });
        assert_eq!(mislabeled_reply["messages"][0], json!("page-before-event"));
        assert_eq!(
            mislabeled_reply["activityToken"],
            json!(persisted_stale_page_token)
        );
        assert!(runtime.transcript_activity_token_matches(session_id, &persisted_stale_page_token));

        let mut unstable_calls = 0;
        let unstable = runtime
            .read_newest_transcript_page_stably(session_id, || {
                unstable_calls += 1;
                runtime.note_transcript_activity(session_id);
                std::future::ready(Ok::<_, ()>(format!("replacement-{unstable_calls}")))
            })
            .await
            .unwrap();
        let mut unstable_reply = json!({ "messages": [unstable.page] });
        runtime.attach_unstable_transcript_activity_discard(session_id, &mut unstable_reply);
        assert_eq!(unstable_calls, 2);
        assert_eq!(
            runtime.current_transcript_activity_token(session_id),
            None,
            "the discard path clears the server watermark before replying"
        );
        assert_eq!(unstable_reply["discardActivityToken"], json!(true));
        assert!(unstable_reply.get("activityToken").is_none());
        assert_ne!(page_before_event, unstable_reply["messages"][0]);

        assert!(
            !runtime.transcript_activity_token_matches(session_id, &persisted_stale_page_token),
            "the stale token no longer matches the current watermark, so it cannot serve unchanged"
        );
        let mut full_path_calls = 0;
        runtime
            .read_newest_transcript_page_stably(session_id, || {
                full_path_calls += 1;
                std::future::ready(Ok::<_, ()>("full-path-after-discard"))
            })
            .await
            .unwrap();
        assert_eq!(full_path_calls, 1);
    }

    #[tokio::test]
    async fn fresh_runtime_boot_invalidation_is_control_not_persisted_client_proof() {
        let runtime = ActivityTokens::new(1);
        let session_id = "ses_restart_control";
        runtime.note_transcript_activity(session_id);
        let persisted_token = runtime
            .current_transcript_activity_token(session_id)
            .expect("control starts with a persisted token");

        let restarted = ActivityTokens::new(2);
        assert!(
            !restarted.transcript_activity_token_matches(session_id, &persisted_token),
            "boot-empty watermarks reject the token without reproducing the long-lived-runtime race"
        );
        let mut provider_calls = 0;
        let control = restarted
            .read_newest_transcript_page_stably(session_id, || {
                provider_calls += 1;
                std::future::ready(Ok::<_, ()>("control-page-after-restart"))
            })
            .await
            .unwrap();
        assert_eq!(provider_calls, 1);
        assert_ne!(
            control.activity_token.as_deref(),
            Some(persisted_token.as_str())
        );
    }

    #[tokio::test]
    async fn second_instability_omits_token_clears_watermark_and_never_third_reads() {
        let runtime = ActivityTokens::new(1);
        let session_id = "ses_twice_unstable";
        runtime.note_transcript_activity(session_id);
        let previously_held_token = runtime
            .current_transcript_activity_token(session_id)
            .expect("client starts with a token");
        let mut provider_calls = 0;

        let observed = runtime
            .read_newest_transcript_page_stably(session_id, || {
                provider_calls += 1;
                let page = format!("useful-page-{provider_calls}");
                runtime.note_transcript_activity(session_id);
                std::future::ready(Ok::<_, ()>(page))
            })
            .await
            .unwrap();

        assert_eq!(
            provider_calls, 2,
            "the observation is bounded to one reread"
        );
        assert_eq!(observed.page, "useful-page-2");
        assert_eq!(observed.activity_token, None);
        assert_eq!(
            runtime.current_transcript_activity_token(session_id),
            None,
            "the unstable page clears every previously issued server watermark"
        );
        assert!(
            !runtime.transcript_activity_token_matches(session_id, &previously_held_token),
            "the pre-discard token misses because the server watermark moved to absent"
        );
        let mut reply = json!({
            "messages": [observed.page],
            "version": "torn-observation",
            "activityToken": null,
            "discardActivityToken": false,
        });
        runtime.attach_unstable_transcript_activity_discard(session_id, &mut reply);
        assert!(reply.get("version").is_none());
        assert!(
            !reply.as_object().unwrap().contains_key("activityToken"),
            "unstable pages omit activityToken entirely rather than emitting null"
        );
        assert_eq!(reply["discardActivityToken"], json!(true));

        runtime.note_transcript_activity(session_id);
        assert_eq!(
            provider_calls, 2,
            "a forced third bump cannot extend the loop"
        );
        assert!(
            !runtime.transcript_activity_token_matches(session_id, &previously_held_token),
            "the client's previous token cannot enter the unchanged arm"
        );
        let mut subsequent_provider_calls = 0;
        let subsequent = runtime
            .read_newest_transcript_page_stably(session_id, || {
                subsequent_provider_calls += 1;
                std::future::ready(Ok::<_, ()>("subsequent-full-page"))
            })
            .await
            .unwrap();
        assert_eq!(
            subsequent_provider_calls, 1,
            "re-presenting the previous token requires a full observation"
        );
        assert_eq!(subsequent.page, "subsequent-full-page");
    }

    /// A token persisted by a client across a module restart must never be
    /// honoured as `unchanged`: distinct boot nonces must separate even the first
    /// token for the same session and sequence.
    #[test]
    fn activity_tokens_from_different_boots_never_collide() {
        let a = ActivityTokens::new(1);
        let b = ActivityTokens::new(2);
        a.note_transcript_activity("ses_x");
        b.note_transcript_activity("ses_x");
        let token_a = a.current_transcript_activity_token("ses_x").unwrap();
        let token_b = b.current_transcript_activity_token("ses_x").unwrap();
        assert_ne!(
            token_a, token_b,
            "same session, same first sequence: only the boot nonce separates these"
        );
    }
}
