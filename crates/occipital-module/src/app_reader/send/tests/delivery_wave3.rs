//! Thalamus's wave-3 send states (`tests/data/thalamus-host-parity-wave3/`,
//! checksums pinned in the Broca reader's `host_parity_wave3` tests).
//!
//! On an interrupt with a steer still waiting, Pi runs the steer as its own
//! turn (`delivered.as: turn`), where OpenCode 2 reports it as a step of the
//! cancelled run. Claude Code delivers a steer into the running turn as a step.
//! Either way the message reached the agent, so the app's waiting list drops it.

use super::super::*;
use serde_json::Value;
use std::path::Path;

fn read(set: &str, name: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/thalamus-host-parity-wave3")
        .join(set)
        .join(name);
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

#[test]
fn every_steer_and_late_send_reads_as_delivered() {
    let steer = read("pi-delivery-groups-v1", "interrupt-with-steer.json");
    let mut answers = vec![
        steer["steer_send_status"].clone(),
        steer["interrupt_send_status"].clone(),
    ];
    for name in [
        "cc-call-yield-send-status.json",
        "cc-call-yield-late-send-status.json",
        "cc-notice-yield-send-status.json",
        "cc-notice-yield-late-send-status.json",
    ] {
        answers.push(read("claude-code-groups-v1", name));
    }
    for answer in &answers {
        assert_eq!(head_fate(Ok(answer)), Fate::Delivered, "{answer}");
    }
}
