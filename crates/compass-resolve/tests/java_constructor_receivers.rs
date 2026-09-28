use std::collections::{BTreeMap, HashMap};
use std::error::Error;
use std::path::Path;

use compass_languages::{CandidateRelation, Engine, EvidenceLimits, validate_evidence};
use compass_resolve::resolve;

const SOURCES: [(&str, &str); 3] = [
    (
        "lib/Cleaner.java",
        r#"package lib;
public class Cleaner {
    public boolean check(String value) { return true; }
    public boolean check(int value) { return false; }
    public Other next() { return new Other(); }
    public String toString() { return "cleaner"; }
}
"#,
    ),
    (
        "lib/Other.java",
        r#"package lib;
public class Other {
    public boolean check(String value) { return false; }
}
"#,
    ),
    (
        "app/Use.java",
        r#"package app;
import lib.Cleaner;
public class Use {
    public boolean direct() { new Cleaner().check("first"); return new Cleaner().check("second"); }
    public boolean parenthesized() { return (new Cleaner()).check("ok"); }
    public boolean qualified() { return new lib.Cleaner().check("ok"); }
    public boolean commented() { return (/* receiver */ new Cleaner(/* args */)).check("ok"); }
    public boolean nested() { return (((new Cleaner()))).check("ok"); }
    public boolean typeNamespace(String Cleaner) { return new Cleaner().check("ok"); }
    public boolean tooDeep() { return (((((((((new Cleaner()))))))))).check("ok"); }
    public String array() { return new Cleaner[0].toString(); }
    public String casted() { return ((Object) new Cleaner()).toString(); }
    public boolean qualifiedAnonymous() { return new lib.Cleaner() {
        public boolean check(String value) { return false; }
    }.check("ok"); }
    public boolean overloaded() { return new Cleaner().check(1); }
    public boolean anonymous() { return new Cleaner() {
        public boolean check(String value) { return false; }
    }.check("ok"); }
    public boolean chained() { return new Cleaner().next().check("ok"); }
    public boolean explicitOuter(Outer outer) { return outer.new Cleaner().check("ok"); }
}
class Outer {
    class Cleaner { public boolean check(String value) { return false; } }
}
"#,
    ),
];

#[test]
fn java_constructor_receivers_preserve_overloads_occurrences_and_unknown_results()
-> Result<(), Box<dyn Error>> {
    let sources = SOURCES
        .into_iter()
        .map(|(path, source)| (path.to_owned(), source.to_owned()))
        .collect::<HashMap<_, _>>();
    let mut engine = Engine::default();
    let mut extractions = SOURCES
        .into_iter()
        .map(|(path, source)| engine.extract_source(Path::new(path), source.as_bytes()))
        .collect::<Result<Vec<_>, _>>()?;
    let evidence = extractions[2]
        .semantic_evidence
        .as_ref()
        .ok_or("missing Java evidence")?;
    validate_evidence(evidence, EvidenceLimits::default())?;
    let enclosing_owner = evidence
        .declarations
        .iter()
        .find(|declaration| declaration.name == "explicitOuter")
        .ok_or("missing enclosing-instance caller")?;
    let enclosing_constructions = evidence
        .candidates
        .iter()
        .filter(|candidate| {
            candidate.relation == CandidateRelation::Constructs
                && candidate.source_declaration_id == enclosing_owner.id
        })
        .collect::<Vec<_>>();
    assert_eq!(enclosing_constructions.len(), 1);
    let enclosing = enclosing_constructions[0];
    assert!(enclosing.binding_id.is_none());
    assert!(enclosing.constraints.qualified_name.is_none());
    assert!(!enclosing.constraints.allow_external);
    let enclosing_occurrence = evidence
        .occurrences
        .iter()
        .find(|occurrence| Some(&occurrence.id) == enclosing.occurrence_id.as_ref())
        .ok_or("missing enclosing-instance occurrence")?;
    assert_eq!(enclosing_occurrence.qualifier.as_deref(), Some("outer"));
    let mut call_sites = BTreeMap::<String, Vec<_>>::new();
    for candidate in &evidence.candidates {
        if candidate.relation != CandidateRelation::Calls
            || !matches!(candidate.target_spelling.as_str(), "check" | "toString")
        {
            continue;
        }
        let owner = evidence
            .declarations
            .iter()
            .find(|declaration| declaration.id == candidate.source_declaration_id)
            .ok_or("missing call owner")?;
        let occurrence = evidence
            .occurrences
            .iter()
            .find(|occurrence| Some(&occurrence.id) == candidate.occurrence_id.as_ref())
            .ok_or("missing call occurrence")?;
        let source_text = sources
            .get(&occurrence.range.source_file)
            .ok_or("missing occurrence source")?;
        let start = usize::try_from(occurrence.range.start_byte)?;
        let end = usize::try_from(occurrence.range.end_byte)?;
        assert_eq!(
            source_text.get(start..end),
            Some(candidate.target_spelling.as_str()),
            "call occurrences must retain the exact method-name span"
        );
        call_sites
            .entry(owner.name.clone())
            .or_default()
            .push(occurrence);
        let expected = match owner.name.as_str() {
            "direct" | "parenthesized" | "qualified" | "overloaded" | "commented" | "nested"
            | "typeNamespace" => Some("lib.Cleaner::check"),
            "anonymous" | "chained" | "tooDeep" | "array" | "casted" | "qualifiedAnonymous"
            | "explicitOuter" => None,
            other => return Err(format!("unexpected check owner: {other}").into()),
        };
        assert_eq!(
            candidate.constraints.qualified_name.as_deref(),
            expected,
            "{}",
            owner.name
        );
    }
    assert_eq!(call_sites.len(), 14);
    assert_eq!(call_sites["direct"].len(), 2);
    assert_ne!(call_sites["direct"][0].id, call_sites["direct"][1].id);

    let resolved = resolve(&extractions, &sources);
    assert!(resolved.error.is_none(), "{:?}", resolved.error);
    assert!(
        !resolved.edges.iter().any(|edge| {
            edge.string("rule").starts_with("universal-construction-")
                && resolved.nodes.iter().any(|node| {
                    node.id == edge.source
                        && node.string("qualified_name") == "app.Use::explicitOuter"
                })
                && resolved.nodes.iter().any(|node| {
                    node.id == edge.target && node.string("qualified_name") == "lib.Cleaner"
                })
        }),
        "outer.new Cleaner() must not instantiate the unrelated imported Cleaner"
    );
    let mut calls = BTreeMap::<String, Vec<String>>::new();
    for edge in &resolved.edges {
        // Raw construction projection also uses `calls` before publication
        // normalizes it to `instantiates`; inspect method-call candidates here.
        if edge.string("relation") != "calls" || !edge.string("rule").starts_with("universal-call-")
        {
            continue;
        }
        let source = resolved
            .nodes
            .iter()
            .find(|node| node.id == edge.source)
            .ok_or("missing source")?;
        let target = resolved
            .nodes
            .iter()
            .find(|node| node.id == edge.target)
            .ok_or("missing target")?;
        if source.string("qualified_name").starts_with("app.Use::") {
            assert_eq!(edge.string("_origin"), "ast");
            assert_eq!(edge.string("confidence"), "EXTRACTED");
            calls
                .entry(source.string("qualified_name"))
                .or_default()
                .push(format!(
                    "{}|{}",
                    target.string("qualified_name"),
                    target.string("signature")
                ));
        }
    }
    for targets in calls.values_mut() {
        targets.sort();
    }
    assert_eq!(
        calls,
        BTreeMap::from([
            (
                "app.Use::direct".to_owned(),
                vec!["lib.Cleaner::check|check(String)".to_owned(); 2]
            ),
            (
                "app.Use::parenthesized".to_owned(),
                vec!["lib.Cleaner::check|check(String)".to_owned()]
            ),
            (
                "app.Use::qualified".to_owned(),
                vec!["lib.Cleaner::check|check(String)".to_owned()]
            ),
            (
                "app.Use::commented".to_owned(),
                vec!["lib.Cleaner::check|check(String)".to_owned()]
            ),
            (
                "app.Use::nested".to_owned(),
                vec!["lib.Cleaner::check|check(String)".to_owned()]
            ),
            (
                "app.Use::typeNamespace".to_owned(),
                vec!["lib.Cleaner::check|check(String)".to_owned()]
            ),
            (
                "app.Use::overloaded".to_owned(),
                vec!["lib.Cleaner::check|check(int)".to_owned()]
            ),
            (
                "app.Use::chained".to_owned(),
                vec!["lib.Cleaner::next|next()".to_owned()]
            ),
        ])
    );
    extractions.reverse();
    let reversed = resolve(&extractions, &sources);
    let canonical = |graph: &compass_languages::Extraction| {
        let mut edges = graph
            .edges
            .iter()
            .map(serde_json::to_string)
            .collect::<Result<Vec<_>, serde_json::Error>>()?;
        edges.sort();
        Ok::<_, serde_json::Error>(edges)
    };
    assert_eq!(canonical(&resolved)?, canonical(&reversed)?);
    Ok(())
}
