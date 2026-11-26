//! Thalamus's `role.describe` for its three hosts once they declare
//! `send_promote` ("Send now": the runner moves a message still waiting for the
//! running turn into that turn, `session.promote`), recorded by Thalamus in
//! `tests/data/thalamus-send-promote-1116/` (files 10-12, copied unchanged). The
//! promote replies in the same set are pinned in `send::tests::promote_1116`.

use super::*;
use occipital_app_reader::source::SendGrant;
use std::path::PathBuf;

fn sample(name: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/thalamus-send-promote-1116")
        .join(name);
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

/// Each host's description passes this reader's check, and on a chat whose
/// grant includes `steer` the session facts' sender carries `promote: true`, so
/// the apps offer "Send now". A grant without `steer` never does.
#[test]
fn each_hosts_description_lets_the_apps_offer_send_now() {
    let head = |modes: Value| -> cortexkit_role_llm_runner::read::HeadMeta {
        serde_json::from_value(json!({
            "incarnation": "inc-a", "first_ordinal": 1, "history_version": 1, "updated_at": 1,
            "last_run_state": {"run_id": "r1", "state": "active"},
            "sender": {"modes": modes, "interrupt": false},
        }))
        .unwrap()
    };
    for name in [
        "10-role-describe-opencode2.json",
        "11-role-describe-pi.json",
        "12-role-describe-claude-code.json",
    ] {
        let describe = check_describe(&sample(name))
            .unwrap_or_else(|error| panic!("{name}: check_describe refused: {error:?}"));
        assert!(describe.declares("send_promote"), "{name}");
        match send_grant(
            &head(json!(["steer", "idle_start"])),
            Some(&describe),
            false,
        ) {
            SendGrant::Granted(sender) => {
                assert_eq!(
                    sender.get("promote"),
                    Some(&json!(true)),
                    "{name}: {sender:?}"
                );
            }
            other => panic!("{name}: {other:?}"),
        }
        match send_grant(&head(json!(["idle_start"])), Some(&describe), false) {
            SendGrant::Granted(sender) => {
                assert!(sender.get("promote").is_none(), "{name}: {sender:?}");
            }
            other => panic!("{name}: {other:?}"),
        }
    }
}
