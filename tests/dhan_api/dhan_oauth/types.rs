use super::*;

#[test]
fn rejects_empty_or_unsafe_configuration() {
    assert!(Config::new("", "key", "secret").is_err());
    assert!(Config::new("123", "key\n", "secret").is_err());
    assert!(Config::new("123", "key", " ").is_err());
    assert!(Config::new("123", "key", "secret").is_ok());
}
