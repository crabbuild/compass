use std::collections::BTreeSet;
use std::error::Error;
use std::path::PathBuf;

use compass_cypher::supported_features;

#[test]
fn every_supported_engine_feature_has_pinned_tck_evidence() -> Result<(), Box<dyn Error>> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let manifest_path = root.join("tests/opencypher-tck/manifest.toml");
    let manifest = std::fs::read_to_string(manifest_path)?;
    let parsed = toml::from_str::<toml::Value>(&manifest)?;
    let features = parsed
        .get("feature")
        .and_then(toml::Value::as_array)
        .ok_or("TCK manifest has no feature entries")?;
    let evidenced = features
        .iter()
        .filter(|feature| {
            feature
                .get("supported")
                .and_then(toml::Value::as_array)
                .is_some_and(|scenarios| !scenarios.is_empty())
        })
        .filter_map(|feature| feature.get("support").and_then(toml::Value::as_array))
        .flatten()
        .filter_map(toml::Value::as_str)
        .collect::<BTreeSet<_>>();
    for feature in supported_features()
        .iter()
        .filter(|feature| feature.supported)
    {
        assert!(
            evidenced.contains(feature.id),
            "supported feature '{}' has no selected TCK scenario",
            feature.id
        );
    }
    let rejected = features
        .iter()
        .filter_map(|feature| feature.get("rejected").and_then(toml::Value::as_array))
        .flatten()
        .count();
    assert!(
        rejected > 0,
        "manifest must record rejected mutation scenarios"
    );
    Ok(())
}
