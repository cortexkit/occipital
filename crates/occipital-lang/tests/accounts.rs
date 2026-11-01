use occipital_lang::{AccountScope, RetainedAllocation, SharedAccount};

#[test]
fn host_reported_memory_is_shared_and_released_without_spending_fuel() {
    let chat = SharedAccount::chat();
    let app = SharedAccount::app();
    let allocation = RetainedAllocation {
        requested: 100,
        unused_capacity: 9,
    };
    let chat_lease = chat.report_allocation(allocation).unwrap();
    let app_lease = app.report_allocation(allocation).unwrap();
    assert_eq!(chat.scope(), AccountScope::Chat);
    assert_eq!(app.scope(), AccountScope::App);
    assert_eq!(chat.clone().usage().retained, 185);
    assert_eq!(app.clone().usage().retained, 185);
    assert_eq!(chat.usage().spent_fuel, 0);
    assert_eq!(chat.usage().expanded, 0);
    drop((chat_lease, app_lease));
    assert_eq!(chat.usage().retained, 0);
    assert_eq!(app.usage().retained, 0);
}
