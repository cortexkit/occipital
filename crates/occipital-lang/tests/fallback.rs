use occipital_lang::program_fallback;

#[test]
fn public_fallback_accepts_unchecked_bytes() {
    let result = program_fallback(
        b"<Unknown title='Still useful' broken {",
        "Title",
        "Summary",
    );
    assert_eq!(result.text.as_str(), "Title\nSummary\nStill useful\n");
    assert_eq!(result.fuel, (result.scanned_bytes + 27) as u64);
}
