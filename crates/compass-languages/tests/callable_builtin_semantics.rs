use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::path::Path;

use compass_languages::{CandidateRelation, Engine, Extraction, SemanticRole};

fn calls(extraction: &Extraction) -> Vec<(&str, &str)> {
    extraction
        .edges
        .iter()
        .filter(|edge| edge.string("relation") == "calls")
        .map(|edge| (edge.source.as_str(), edge.target.as_str()))
        .collect()
}

fn callable_labels(extraction: &Extraction) -> HashMap<&str, String> {
    extraction
        .nodes
        .iter()
        .map(|node| {
            (
                node.id.as_str(),
                node.label()
                    .trim()
                    .trim_matches(['(', ')'])
                    .trim_start_matches('.')
                    .to_owned(),
            )
        })
        .collect()
}

fn assert_local_builtin_collisions_resolve(
    path: &Path,
    source: &[u8],
    expected_sites: usize,
) -> Result<(), Box<dyn Error>> {
    let extraction = Engine::default().extract_source(path, source)?;
    let labels = callable_labels(&extraction);
    let local_calls = calls(&extraction)
        .into_iter()
        .filter_map(|(_, target)| labels.get(target).map(String::as_str))
        .filter(|label| matches!(*label, "open" | "list" | "map" | "filter"))
        .collect::<Vec<_>>();

    assert_eq!(
        local_calls.len(),
        expected_sites,
        "nodes={:?}\nedges={:?}\nraw_calls={:?}",
        extraction.nodes,
        extraction.edges,
        extraction.raw_calls
    );
    assert_eq!(
        local_calls.iter().copied().collect::<HashSet<_>>(),
        HashSet::from(["open", "list", "map", "filter"])
    );
    for edge in extraction
        .edges
        .iter()
        .filter(|edge| edge.string("relation") == "calls")
    {
        let start = edge
            .attributes
            .get("start_byte")
            .and_then(|value| value.as_u64());
        let end = edge
            .attributes
            .get("end_byte")
            .and_then(|value| value.as_u64());
        assert!(
            matches!((start, end), (Some(start), Some(end)) if start < end),
            "call has no exact source range: {edge:?}"
        );
        assert!(!edge.string("extractor").is_empty(), "edge={edge:?}");
    }
    Ok(())
}

fn assert_universal_builtin_collisions_are_resolvable(
    path: &Path,
    source: &[u8],
    expected_sites: usize,
) -> Result<(), Box<dyn Error>> {
    let extraction = Engine::default().extract_source(path, source)?;
    let evidence = extraction
        .semantic_evidence
        .as_ref()
        .ok_or("missing universal semantic evidence")?;
    let local_calls = evidence
        .occurrences
        .iter()
        .filter(|occurrence| {
            occurrence.role == SemanticRole::Call
                && matches!(
                    occurrence.spelling.as_str(),
                    "open" | "list" | "map" | "filter"
                )
        })
        .collect::<Vec<_>>();

    assert_eq!(local_calls.len(), expected_sites);
    assert_eq!(
        local_calls
            .iter()
            .map(|occurrence| occurrence.spelling.as_str())
            .collect::<HashSet<_>>(),
        HashSet::from(["open", "list", "map", "filter"])
    );
    assert!(local_calls.iter().all(|occurrence| {
        occurrence.range.start_byte < occurrence.range.end_byte
            && evidence.candidates.iter().any(|candidate| {
                candidate.relation == CandidateRelation::Calls
                    && candidate.occurrence_id.as_deref() == Some(&occurrence.id)
                    && !candidate.constraints.allow_external
            })
    }));
    assert!(extraction.nodes.is_empty());
    assert!(extraction.edges.is_empty());
    assert!(extraction.raw_calls.is_none());
    Ok(())
}

#[test]
fn local_builtin_spellings_resolve_before_filtering_in_rust_and_java() -> Result<(), Box<dyn Error>>
{
    assert_universal_builtin_collisions_are_resolvable(
        Path::new("collisions.rs"),
        br#"
fn open() {}
fn list() {}
fn map() {}
fn filter() {}
fn caller() {
    open();
    open();
    list();
    map();
    filter();
}
"#,
        5,
    )?;
    assert_universal_builtin_collisions_are_resolvable(
        Path::new("Collisions.java"),
        br#"
class Collisions {
    static void open() {}
    static void list() {}
    static void map() {}
    static void filter() {}
    static void caller() {
        open();
        list();
        map();
        filter();
    }
}
"#,
        4,
    )
}

#[test]
fn local_javascript_and_python_declarations_override_builtin_spellings()
-> Result<(), Box<dyn Error>> {
    assert_local_builtin_collisions_resolve(
        Path::new("collisions.js"),
        br#"
function open() {}
function list() {}
function map() {}
function filter() {}
function caller() {
  open();
  list();
  map();
  filter();
}
"#,
        4,
    )?;
    let extraction = Engine::default().extract_source(
        Path::new("collisions.py"),
        br#"
def open(): pass
def list(): pass
def map(): pass
def filter(): pass
def caller():
    open()
    list()
    map()
    filter()
"#,
    )?;
    let evidence = extraction
        .semantic_evidence
        .as_ref()
        .ok_or("missing Python semantic evidence")?;
    let local_calls = evidence
        .occurrences
        .iter()
        .filter(|occurrence| {
            occurrence.role == SemanticRole::Call
                && matches!(
                    occurrence.spelling.as_str(),
                    "open" | "list" | "map" | "filter"
                )
        })
        .collect::<Vec<_>>();
    assert_eq!(local_calls.len(), 4);
    assert_eq!(
        local_calls
            .iter()
            .map(|occurrence| occurrence.spelling.as_str())
            .collect::<HashSet<_>>(),
        HashSet::from(["open", "list", "map", "filter"])
    );
    assert!(local_calls.iter().all(|occurrence| {
        occurrence.range.start_byte < occurrence.range.end_byte
            && evidence.candidates.iter().any(|candidate| {
                candidate.relation == CandidateRelation::Calls
                    && candidate.occurrence_id.as_deref() == Some(&occurrence.id)
            })
    }));
    Ok(())
}

#[test]
fn unresolved_javascript_and_python_builtins_remain_suppressed() -> Result<(), Box<dyn Error>> {
    for (path, source) in [
        (
            Path::new("builtins.js"),
            b"function caller(){ Map(); parseInt('1'); Array(); }\n".as_slice(),
        ),
        (
            Path::new("builtins.py"),
            b"def caller():\n    open('x')\n    list()\n    map(str, [])\n    filter(None, [])\n"
                .as_slice(),
        ),
    ] {
        let extraction = Engine::default().extract_source(path, source)?;
        assert!(
            calls(&extraction).is_empty(),
            "{path:?}: edges={:?}",
            extraction.edges
        );
        assert!(
            extraction.raw_calls.iter().flatten().all(|call| !matches!(
                call.callee.as_str(),
                "Map" | "parseInt" | "Array" | "open" | "list" | "map" | "filter"
            )),
            "{path:?}: raw_calls={:?}",
            extraction.raw_calls
        );
    }
    Ok(())
}
