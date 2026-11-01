use occipital_lang::{Address, DiagnosticCode};

#[test]
fn canonical_address_is_the_only_openable_representation() {
    let address = Address::parse("HTTPS://Faß.DE:443/é?q='#'").unwrap();
    assert_eq!(address.as_str(), "https://xn--fa-hia.de/%C3%A9?q=%27#'");
    assert_eq!(address.display().as_str(), address.as_str());
    assert_eq!(
        Address::parse("file:///private/file"),
        Err(DiagnosticCode::Scheme)
    );
}
