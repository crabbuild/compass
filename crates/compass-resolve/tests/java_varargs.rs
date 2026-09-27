use std::collections::{BTreeMap, HashMap};
use std::error::Error;
use std::path::Path;

use compass_languages::{Engine, EvidenceLimits, validate_evidence};
use compass_resolve::resolve;

#[test]
fn java_varargs_resolves_compiler_supported_overloads() -> Result<(), Box<dyn Error>> {
    let source =
        include_str!("../../../benchmarks/agent_query/fixtures/java_varargs/VarargsDemo.java");
    let extraction =
        Engine::default().extract_source(Path::new("VarargsDemo.java"), source.as_bytes())?;
    validate_evidence(
        extraction
            .semantic_evidence
            .as_ref()
            .ok_or("missing evidence")?,
        EvidenceLimits::default(),
    )?;
    let sources = HashMap::from([("VarargsDemo.java".to_owned(), source.to_owned())]);
    let resolved = resolve(&[extraction], &sources);
    assert!(resolved.error.is_none(), "{:?}", resolved.error);
    let mut calls = BTreeMap::<String, Vec<String>>::new();
    for edge in &resolved.edges {
        if edge.string("relation") != "calls" || !edge.string("rule").starts_with("universal-call-")
        {
            continue;
        }
        let caller = resolved
            .nodes
            .iter()
            .find(|node| node.id == edge.source)
            .ok_or("missing caller")?;
        let target = resolved
            .nodes
            .iter()
            .find(|node| node.id == edge.target)
            .ok_or("missing target")?;
        assert_eq!(edge.string("_origin"), "ast");
        assert_eq!(edge.string("confidence"), "EXTRACTED");
        calls
            .entry(caller.string("qualified_name"))
            .or_default()
            .push(target.string("signature"));
    }
    assert_eq!(
        calls,
        BTreeMap::from([
            (
                "audit.VarargsDemo::text".to_owned(),
                vec!["join(String...)".to_owned()]
            ),
            (
                "audit.VarargsDemo::numbers".to_owned(),
                vec!["join(int...)".to_owned()]
            ),
            (
                "audit.VarargsDemo::flag".to_owned(),
                vec!["join(boolean)".to_owned()]
            ),
            (
                "audit.VarargsDemo::explicitArray".to_owned(),
                vec!["join(String...)".to_owned()]
            ),
            (
                "audit.VarargsDemo::mixed".to_owned(),
                vec!["prefixed(String,String...)".to_owned()]
            ),
        ])
    );
    Ok(())
}
