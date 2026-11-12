use serde_json::json;

use super::{format_fact, FactError, FormattedFact, Tone};

fn words(fact: serde_json::Value) -> (String, Tone) {
    let FormattedFact { text, tone, .. } = format_fact(&fact).unwrap();
    (text, tone)
}

fn code(fact: serde_json::Value) -> &'static str {
    format_fact(&fact).unwrap_err().code()
}

/// An estimate shows no more digits than it means: thousands and millions
/// from 1,000 up, one decimal while under ten, each step chosen after
/// rounding; an estimate of zero is "none". Exact counts keep every digit.
#[test]
fn estimates_are_shortened_and_zero_is_none() {
    let approx = |value: serde_json::Value, unit: &str| {
        words(json!({"label": "L", "value": value, "unit": unit, "approx": true})).0
    };
    for (value, text) in [
        (json!(840), "about 840 tokens"),
        (json!(999), "about 999 tokens"),
        (json!(1000), "about 1k tokens"),
        (json!(6708), "about 6.7k tokens"),
        (json!(6750), "about 6.8k tokens"),
        (json!(9960), "about 10k tokens"),
        (json!(114_730), "about 115k tokens"),
        (json!(999_400), "about 999k tokens"),
        (json!(999_600), "about 1M tokens"),
        (json!(1_240_000), "about 1.2M tokens"),
        (json!(15_400_000), "about 15M tokens"),
        (json!(2_500_000_000_i64), "about 2,500M tokens"),
        (json!(-6708), "about -6.7k tokens"),
        (json!(0), "none"),
        (json!(0.0), "none"),
    ] {
        assert_eq!(approx(value.clone(), "tokens"), text, "{value}");
    }
    assert_eq!(
        words(json!({"label": "L", "value": 12_345, "noun": "parts", "approx": true})).0,
        "about 12k parts"
    );
    assert_eq!(approx(json!(12_345), "count"), "about 12k");
    assert_eq!(approx(json!(0), "bytes"), "none");
    assert_eq!(approx(json!(8_000_000), "bytes"), "about 7.6 MiB");
    assert_eq!(approx(json!(0.42), "ratio"), "about 42%");
    assert_eq!(
        words(json!({"label": "L", "value": 114_730, "unit": "tokens"})).0,
        "114,730 tokens"
    );
    assert_eq!(
        words(json!({"label": "L", "value": 0, "unit": "tokens"})).0,
        "0 tokens"
    );
}

#[test]
fn numbers_are_worded_by_their_unit() {
    let cases = [
        (
            json!({"label": "Queued drops", "value": 41, "noun": "parts"}),
            "41 parts",
        ),
        (
            json!({"label": "Files", "value": 7400, "unit": "count"}),
            "7,400",
        ),
        (
            json!({"label": "Removes", "value": 18200, "unit": "tokens", "approx": true}),
            "about 18k tokens",
        ),
        (
            json!({"label": "Left", "value": 1, "unit": "tokens"}),
            "1 token",
        ),
        (
            json!({"label": "Rounded", "value": 1240.6, "unit": "tokens"}),
            "1,241 tokens",
        ),
        (
            json!({"label": "Size", "value": 8_000_000, "unit": "bytes"}),
            "7.6 MiB",
        ),
        (
            json!({"label": "Took", "value": 125_000, "unit": "duration"}),
            "2m 5s",
        ),
        (
            json!({"label": "Took", "value": 920, "unit": "duration"}),
            "920 ms",
        ),
        (
            json!({"label": "Context after", "value": 0.61, "unit": "ratio"}),
            "61%",
        ),
        (
            json!({"label": "Half up", "value": 0.615, "unit": "ratio"}),
            "62%",
        ),
        (
            json!({"label": "Over", "value": 1.2, "unit": "ratio"}),
            "120%",
        ),
    ];
    for (fact, expected) in cases {
        assert_eq!(
            words(fact.clone()),
            (expected.to_owned(), Tone::Neutral),
            "{fact}"
        );
    }
}

#[test]
fn a_number_may_name_its_tone() {
    assert_eq!(
        words(json!({"label": "Errors", "value": 3, "tone": "danger"})),
        ("3".to_owned(), Tone::Danger)
    );
}

#[test]
fn states_have_one_wording_and_one_tone() {
    let cases = [
        ("ready", "Ready", Tone::Success),
        ("building", "Building", Tone::Info),
        ("paused", "Paused", Tone::Warning),
        ("stale", "Out of date", Tone::Warning),
        ("unavailable", "Unavailable", Tone::Danger),
        ("off", "Off", Tone::Neutral),
    ];
    for (state, text, tone) in cases {
        assert_eq!(
            words(json!({"label": "Code map", "state": state})),
            (text.to_owned(), tone),
            "{state}"
        );
    }
    assert_eq!(
        words(json!({"label": "Code map", "state": "building", "progress": 0.4})),
        ("Building, 40%".to_owned(), Tone::Info)
    );
}

#[test]
fn the_label_is_kept_as_given() {
    let fact = format_fact(&json!({"label": "Shells running", "value": 2})).unwrap();
    assert_eq!(fact.label, "Shells running");
}

#[test]
fn members_that_dont_go_together_are_refused() {
    let conflicts = [
        json!({"label": "x"}),
        json!({"label": "x", "value": 1, "state": "ready"}),
        json!({"label": "x", "state": "ready", "unit": "count"}),
        json!({"label": "x", "state": "ready", "approx": true}),
        json!({"label": "x", "state": "ready", "tone": "danger"}),
        json!({"label": "x", "state": "ready", "noun": "parts"}),
        json!({"label": "x", "state": "ready", "progress": 0.5}),
        json!({"label": "x", "value": 1, "progress": 0.5}),
        json!({"label": "x", "value": 1, "unit": "tokens", "noun": "parts"}),
    ];
    for fact in conflicts {
        assert_eq!(code(fact.clone()), "fact_conflict", "{fact}");
    }
}

#[test]
fn out_of_range_values_are_refused() {
    let ranges = [
        json!({"label": "", "value": 1}),
        json!({"label": "x".repeat(65), "value": 1}),
        json!({"label": "x", "value": 1, "noun": ""}),
        json!({"label": "x", "value": 1, "noun": "n".repeat(25)}),
        json!({"label": "x", "state": "building", "progress": 1.5}),
        json!({"label": "x", "state": "building", "progress": -0.1}),
        json!({"label": "x", "value": 1e30}),
    ];
    for fact in ranges {
        assert_eq!(code(fact.clone()), "fact_range", "{fact}");
    }
}

#[test]
fn text_values_and_unknown_members_are_malformed() {
    let malformed = [
        json!({"label": "x", "value": "ready"}),
        json!({"label": "x", "value": 1, "unit": "minutes"}),
        json!({"label": "x", "state": "warming"}),
        json!({"label": "x", "value": 1, "colour": "red"}),
        json!({"value": 1}),
        json!([1]),
    ];
    for fact in malformed {
        assert!(
            matches!(format_fact(&fact), Err(FactError::Malformed(_))),
            "{fact}"
        );
    }
}
