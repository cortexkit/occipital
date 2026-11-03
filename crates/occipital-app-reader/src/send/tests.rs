use super::*;

/// Core's published idempotency samples, copied byte for byte from prefrontal
/// `test-vectors/phone-message-idempotency-v1/` at `6b36afc14`. The checksums are
/// that directory's `SHA256SUMS`.
pub(crate) const VECTORS: [(&str, &str, &str); 7] = [
    (
        "first-enqueue.json",
        include_str!("../../test-vectors/phone-message-idempotency-v1/first-enqueue.json"),
        "3dcbb398f3bd02519afc8dc31862e66d87cb878ace50f10aa5d609766fe5aa80",
    ),
    (
        "repeat.json",
        include_str!("../../test-vectors/phone-message-idempotency-v1/repeat.json"),
        "793cf8be7d400e85244c19e8fffe0e976aed9b4078cf8eea416e8667481ec7af",
    ),
    (
        "conflict.json",
        include_str!("../../test-vectors/phone-message-idempotency-v1/conflict.json"),
        "17d7d6480daf94995b5248aafada3b73521e641d0b66e34285611ef6b62e78f5",
    ),
    (
        "transcript-page.json",
        include_str!("../../test-vectors/phone-message-idempotency-v1/transcript-page.json"),
        "a187c1ee9d007fd807af50ca4af4198ad5c8c8259e2845914a68c82bf064a9d7",
    ),
    (
        "attachment-first-enqueue.json",
        include_str!(
            "../../test-vectors/phone-message-idempotency-v1/attachment-first-enqueue.json"
        ),
        "db5a07f1f2e86172f37a65c66a9b6ee61a70ccf49ce5af235b24cb0873a3d636",
    ),
    (
        "attachment-repeat.json",
        include_str!("../../test-vectors/phone-message-idempotency-v1/attachment-repeat.json"),
        "4f843612c96c1ebdc62d0d621086b3787ee7551bfdbd9741a8fc11c34fae9998",
    ),
    (
        "attachment-conflict.json",
        include_str!("../../test-vectors/phone-message-idempotency-v1/attachment-conflict.json"),
        "cd0a4b67e10702b9c8b23e4fd43ac1eff1afa72804603ad336827ad1222a6595",
    ),
];

const SUMS: &str = include_str!("../../test-vectors/phone-message-idempotency-v1/SHA256SUMS");

fn vector(name: &str) -> Value {
    let (_, bytes, _) = VECTORS.iter().find(|(file, _, _)| *file == name).unwrap();
    serde_json::from_str(bytes).unwrap()
}

#[test]
fn ported_vectors_keep_cores_checksums() {
    use sha2::{Digest, Sha256};
    for (name, bytes, checksum) in VECTORS {
        assert_eq!(
            format!("{:x}", Sha256::digest(bytes.as_bytes())),
            checksum,
            "{name}"
        );
        assert!(
            SUMS.lines()
                .any(|line| line == format!("{checksum}  {name}")),
            "{name} is listed in SHA256SUMS"
        );
    }
    assert_eq!(SUMS.lines().count(), VECTORS.len());
}

#[test]
fn cores_text_requests_parse_and_ignore_the_caller_fields_a_route_once_stamped() {
    for name in ["first-enqueue.json", "repeat.json", "conflict.json"] {
        let record = vector(name);
        assert_eq!(record["op"], OPERATION);
        let request = SendRequest::parse(&record["request"]).unwrap();
        assert_eq!(request.session, "phone-target");
        assert_eq!(
            request.client_message_id.as_deref(),
            record["request"]["clientMessageId"].as_str()
        );
        assert_eq!(request.mode, None);
        assert_eq!(
            request.content(),
            json!({"text": record["request"]["text"], "parts": []})
        );
    }
    // The repeat carries the same content as the first send; the conflict does not.
    let first = SendRequest::parse(&vector("first-enqueue.json")["request"]).unwrap();
    let repeat = SendRequest::parse(&vector("repeat.json")["request"]).unwrap();
    let conflict = SendRequest::parse(&vector("conflict.json")["request"]).unwrap();
    assert_eq!(first.content(), repeat.content());
    assert_ne!(first.content(), conflict.content());
}

/// Core's attachment records: the repeat re-uploaded the same photo under a new
/// upload id, and the conflict names different bytes. Matched by the files' bytes,
/// the repeat is the same message and the conflict is not.
#[test]
fn cores_attachment_requests_match_by_the_files_bytes() {
    let parse = |name: &str| SendRequest::parse(&vector(name)["request"]).unwrap();
    let first = parse("attachment-first-enqueue.json");
    let repeat = parse("attachment-repeat.json");
    let conflict = parse("attachment-conflict.json");
    assert_eq!(first.attachments(), ["<attachment-id-first>"]);
    assert_ne!(first.content(), repeat.content(), "the upload ids differ");
    let photo = ["a".repeat(64)];
    assert_eq!(
        first.content_by_bytes(&photo),
        repeat.content_by_bytes(&photo)
    );
    assert_ne!(
        first.content_by_bytes(&photo),
        conflict.content_by_bytes(&["b".repeat(64)])
    );
}

#[test]
fn malformed_requests_get_cores_codes_in_cores_order() {
    let cases = [
        // An invalid client id is reported before anything else is read.
        (
            json!({"clientMessageId": "has space"}),
            "invalid_client_message_id",
            "clientMessageId must contain 1-64 ASCII letters, digits, '-' or '_'",
        ),
        (
            json!({"session": "s", "text": "t", "clientMessageId": 7}),
            "invalid_client_message_id",
            "clientMessageId must be a string",
        ),
        (
            json!({"session": "s", "text": "t", "clientMessageId": ""}),
            "invalid_client_message_id",
            "clientMessageId must contain 1-64 ASCII letters, digits, '-' or '_'",
        ),
        (
            json!({"session": "s", "text": "t", "clientMessageId": "a".repeat(65)}),
            "invalid_client_message_id",
            "clientMessageId must contain 1-64 ASCII letters, digits, '-' or '_'",
        ),
        (
            json!({"text": "t"}),
            "invalid_request",
            "missing string param 'session'",
        ),
        (
            json!({"session": "s"}),
            "invalid_request",
            "missing string param 'text'",
        ),
        (
            json!({"session": "s", "text": "t", "send_id": "space denied"}),
            "invalid_send_id",
            "send_id may contain only ASCII letters, digits, '.', '_', and '-'",
        ),
        (
            json!({"session": "  ", "text": "t"}),
            "invalid_request",
            "session must not be empty",
        ),
        (
            json!({"session": "s", "text": "t", "mode": "later"}),
            "invalid_request",
            "mode 'later' is unknown; expected queue, steer or interrupt",
        ),
        (
            json!({"session": "s", "text": "t", "mode": 1}),
            "invalid_request",
            "mode must be a string when present: queue, steer or interrupt",
        ),
        (
            json!({"session": "s", "text": "t", "parts": "photo"}),
            "invalid_request",
            "parts must be an array of file parts",
        ),
        (
            json!({"session": "s", "text": "t", "parts": [{"type": "file"}]}),
            "invalid_request",
            "each part must be {type: \"file\", attachment_id} naming a finished upload",
        ),
        (
            json!({"session": "s", "text": "t", "parts": [{"type": "text", "attachment_id": "a"}]}),
            "invalid_request",
            "each part must be {type: \"file\", attachment_id} naming a finished upload",
        ),
        (
            json!({"session": "s", "text": "t", "parts": [
                {"type": "file", "attachment_id": "a"}, {"type": "file", "attachment_id": "a"}]}),
            "invalid_request",
            "attachment a is named twice",
        ),
        (
            json!({"session": "s", "text": "t", "parts": (0..11)
                .map(|i| json!({"type": "file", "attachment_id": format!("a{i}")}))
                .collect::<Vec<_>>()}),
            "invalid_request",
            "a message may name at most 10 files",
        ),
        (
            json!({"session": "s", "text": "  "}),
            "invalid_request",
            "message must include non-empty text or at least one file part",
        ),
    ];
    for (params, code, message) in cases {
        let refusal = SendRequest::parse(&params).unwrap_err();
        assert_eq!(
            (refusal.code, refusal.message.as_str()),
            (code, message),
            "{params}"
        );
    }
}

#[test]
fn modes_parse_and_an_absent_mode_stays_absent() {
    for (mode, expected) in [
        ("queue", Mode::Queue),
        ("steer", Mode::Steer),
        ("interrupt", Mode::Interrupt),
    ] {
        let request =
            SendRequest::parse(&json!({"session": "s", "text": "t", "mode": mode})).unwrap();
        assert_eq!(request.mode, Some(expected));
        assert_eq!(expected.as_str(), mode);
    }
    let request = SendRequest::parse(&json!({"session": "s", "text": "t"})).unwrap();
    assert_eq!(request.mode, None);
    // Empty parts carry nothing and are part of the compared content as sent.
    let request = SendRequest::parse(&json!({"session": "s", "text": "t", "parts": null})).unwrap();
    assert_eq!(request.content(), json!({"text": "t", "parts": null}));
    assert!(request.attachments().is_empty());
}

/// A file part names a finished upload by its `attachment_id`; the files keep the
/// message's order, and a message holding only files needs no text.
#[test]
fn file_parts_name_their_uploads_in_order() {
    let parts = json!([
        {"type": "file", "attachment_id": "b", "mime": "image/png", "filename": "b.png"},
        {"type": "file", "attachment_id": "a"},
    ]);
    let request = SendRequest::parse(&json!({"session": "s", "text": "", "parts": parts})).unwrap();
    assert_eq!(request.attachments(), ["b", "a"]);
    assert_eq!(request.content(), json!({"text": "", "parts": parts}));
}

/// Core's `send_id_validation_accepts_only_the_bounded_opaque_charset`.
#[test]
fn send_id_validation_accepts_only_the_bounded_opaque_charset() {
    assert_eq!(
        validate_enqueue_send_id(Some(&json!("ios.42_retry-token"))).unwrap(),
        Some("ios.42_retry-token".to_string())
    );
    assert_eq!(validate_enqueue_send_id(None).unwrap(), None);

    for invalid in [
        json!(""),
        json!("space denied"),
        json!("slash/denied"),
        json!("ü"),
        json!(42),
    ] {
        assert!(
            validate_enqueue_send_id(Some(&invalid)).is_err(),
            "invalid send_id must be refused: {invalid}"
        );
    }
    assert!(validate_enqueue_send_id(Some(&json!("a".repeat(129)))).is_err());
}
