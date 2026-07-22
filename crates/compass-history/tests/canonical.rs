use compass_history::{
    BuildProfile, ExtractionFingerprint, ExtractionFingerprintInput, canonical_json_bytes,
    edge_key, hyperedge_key, node_key,
};
use prolly::decode_segments;
use serde_json::{Value, json};

#[test]
fn canonical_json_is_recursive_and_preserves_number_identity()
-> Result<(), Box<dyn std::error::Error>> {
    let left = json!({"z": 1, "a": {"y": 2, "x": 3}});
    let right = json!({"a": {"x": 3, "y": 2}, "z": 1});
    assert_eq!(canonical_json_bytes(&left)?, canonical_json_bytes(&right)?);
    assert_eq!(
        canonical_json_bytes(&left)?,
        br#"{"a":{"x":3,"y":2},"z":1}"#
    );
    assert_ne!(
        canonical_json_bytes(&json!(1))?,
        canonical_json_bytes(&json!(1.0))?
    );
    assert_eq!(canonical_json_bytes(&json!(-0.0))?, b"-0.0");
    let unicode = json!({"é": "雪", "a": "\n"});
    assert_eq!(
        canonical_json_bytes(&unicode)?,
        "{\"a\":\"\\n\",\"é\":\"雪\"}".as_bytes()
    );
    Ok(())
}

#[test]
fn canonical_number_boundaries_and_exponents_are_stable() -> Result<(), Box<dyn std::error::Error>>
{
    let values: Value =
        serde_json::from_str(r#"[-9223372036854775808,18446744073709551615,1.25e30,1.25e-30]"#)?;
    assert_eq!(
        canonical_json_bytes(&values)?,
        b"[-9223372036854775808,18446744073709551615,1.25e+30,1.25e-30]"
    );
    Ok(())
}

#[test]
fn typed_keys_are_segment_safe_and_direction_aware() -> Result<(), Box<dyn std::error::Error>> {
    assert_ne!(node_key("a\0b"), node_key("a"));
    assert_ne!(
        edge_key("a", "b\0c", "calls", true, None),
        edge_key("a\0b", "c", "calls", true, None)
    );
    assert_eq!(
        edge_key("b", "a", "calls", false, None),
        edge_key("a", "b", "calls", false, None)
    );
    assert_ne!(
        edge_key("b", "a", "calls", true, None),
        edge_key("a", "b", "calls", true, None)
    );
    let record = canonical_json_bytes(&json!({"members":["a","b"]}))?;
    assert_ne!(
        hyperedge_key(&record, Some(0)),
        hyperedge_key(&record, Some(1))
    );
    assert_eq!(decode_segments(&node_key("a\0雪"))?[2], "a\0雪".as_bytes());
    Ok(())
}

#[test]
fn fingerprints_are_order_independent_secret_free_and_strict()
-> Result<(), Box<dyn std::error::Error>> {
    let mut first = ExtractionFingerprintInput::new("0.1.0", "schema-1");
    first.insert("model", "claude-sonnet")?;
    first.insert("prompt_sha256", "abc123")?;
    let mut second = ExtractionFingerprintInput::new("0.1.0", "schema-1");
    second.insert("prompt_sha256", "abc123")?;
    second.insert("model", "claude-sonnet")?;
    let digest = first.digest()?;
    assert_eq!(digest, second.digest()?);
    assert_eq!(digest.as_hex().len(), 64);
    assert!(
        first
            .insert("api_key", "must-not-enter-fingerprint")
            .is_err()
    );
    assert!(!String::from_utf8(first.canonical_bytes()?)?.contains("api_key"));
    let encoded = serde_json::to_string(&digest)?;
    assert_eq!(
        serde_json::from_str::<ExtractionFingerprint>(&encoded)?,
        digest
    );
    assert!(
        digest
            .as_hex()
            .to_uppercase()
            .parse::<ExtractionFingerprint>()
            .is_err()
    );

    let mut profile = BuildProfile::default();
    profile.insert("Mode", "deep")?;
    assert_eq!(profile.digest()?.len(), 32);
    assert!(profile.insert("access_token", "secret").is_err());
    Ok(())
}
