use super::*;
use std::io::Read as _;

#[test]
fn overlong_lines_are_rejected_before_the_reader_reaches_their_end() {
    let mut remaining = std::io::repeat(b'x').take(32 << 20);
    let result = read(std::io::BufReader::new(&mut remaining), |_| Ok(()));
    assert!(matches!(result, Err(Unreadable::Broken(_))));
    assert!(remaining.limit() > 15 << 20, "reader consumed the whole line");
}
