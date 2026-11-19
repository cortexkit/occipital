use super::*;

#[test]
fn migration_backs_up_original_bytes_and_assigns_the_legacy_order_once() {
    let dir = tempfile::tempdir().unwrap();
    let moves = dir.path().join(DIR);
    fs::create_dir(&moves).unwrap();
    let legacy =
        |time| format!(r#"{{ "agent":"agent", "step":"aborted", "sessions":[], "at_ms":{time} }}"#);
    for (name, time) in [("c", 1), ("b", 2), ("a", 2)] {
        fs::write(moves.join(format!("{name}.json")), legacy(time)).unwrap();
    }
    let cuts = Cuts::open(dir.path()).unwrap();
    for (seq, name, time) in [(1, "c", 1), (2, "a", 2), (3, "b", 2)] {
        let path = moves.join(format!("{name}.json"));
        let bytes = fs::read(&path).unwrap();
        assert!(serde_json::from_slice::<Cut>(&bytes).is_err());
        let saved = Saved::parse(&bytes).unwrap();
        assert_eq!(saved.sequence, seq);
        assert_eq!(
            fs::read_to_string(moves.join(format!("pre-v2/{name}.json"))).unwrap(),
            legacy(time)
        );
    }
    let mut cut = cuts.load("b").unwrap().unwrap();
    cut.at_ms = -5;
    cuts.save("b", &cut).unwrap();
    drop(cuts);
    let reopened = Cuts::open(dir.path()).unwrap();
    assert_eq!(reopened.load("b").unwrap().unwrap().at_ms, -5);
    assert_eq!(
        fs::read_to_string(moves.join("pre-v2/b.json")).unwrap(),
        legacy(2)
    );
}
