use compass_agent_graph::{Digest, canonical_bytes, canonical_digest};
use serde_json::json;

#[test]
fn object_key_order_does_not_change_canonical_bytes_or_digest()
-> Result<(), Box<dyn std::error::Error>> {
    let left = json!({"z": 1, "nested": {"b": 2, "a": 1}, "a": [3, 2, 1]});
    let right = json!({"a": [3, 2, 1], "nested": {"a": 1, "b": 2}, "z": 1});

    assert_eq!(canonical_bytes(&left)?, canonical_bytes(&right)?);
    assert_eq!(
        canonical_digest("test-domain", &left)?,
        canonical_digest("test-domain", &right)?
    );
    assert_ne!(
        canonical_digest("test-domain", &left)?,
        canonical_digest("other-domain", &left)?
    );
    Ok(())
}

#[test]
fn digests_require_lowercase_sha256() {
    assert!(Digest::parse("a".repeat(64)).is_ok());
    assert!(Digest::parse("A".repeat(64)).is_err());
    assert!(Digest::parse("a".repeat(63)).is_err());
    assert!(Digest::parse("g".repeat(64)).is_err());
}
