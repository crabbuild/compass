use std::collections::{BTreeSet, HashMap};
use std::error::Error;
use std::path::Path;

use compass_graph::{BuildEvidence, normalize_v1};
use compass_languages::{CandidateRelation, Engine, Extraction, HierarchyConstraint};
use compass_model::code_graph::EdgeKind;
use compass_resolve::resolve;

fn extract(file: &str, source: &str) -> Result<Extraction, Box<dyn Error>> {
    Ok(Engine::default().extract_source(Path::new(file), source.as_bytes())?)
}

#[test]
fn cross_file_literals_retain_type_references_without_constructor_calls()
-> Result<(), Box<dyn Error>> {
    let model = "pub struct Item { pub value: usize }\n";
    let caller = "use crate::model::Item as Record;\nfn build() {\n    let _ = Record { value: 1 };\n    let _ = crate::model::Item { value: 2 };\n    let _ = Record { value: 3 };\n}\n";
    let directory = tempfile::tempdir()?;
    std::fs::create_dir_all(directory.path().join("src"))?;
    std::fs::write(directory.path().join("src/model.rs"), model)?;
    std::fs::write(directory.path().join("src/lib.rs"), caller)?;
    let sources = HashMap::from([
        ("src/model.rs".into(), model.into()),
        ("src/lib.rs".into(), caller.into()),
    ]);
    let inputs = vec![
        extract("src/model.rs", model)?,
        extract("src/lib.rs", caller)?,
    ];
    let evidence = inputs[1]
        .semantic_evidence
        .as_ref()
        .ok_or("missing direct evidence")?;
    let owner = evidence
        .declarations
        .iter()
        .find(|declaration| declaration.name == "build")
        .ok_or("missing build evidence")?;
    assert_eq!(
        evidence
            .candidates
            .iter()
            .filter(|candidate| candidate.source_declaration_id == owner.id
                && candidate.relation == CandidateRelation::References)
            .count(),
        3,
        "extract literal type references before cross-file resolution"
    );
    let resolved = resolve(&inputs, &sources);
    assert!(resolved.error.is_none(), "{:?}", resolved.error);
    let build =
        BuildEvidence::from_extraction(directory.path(), &resolved, "sha256:struct-literal")?;
    let graph = normalize_v1(resolved, build)?;
    let caller_node = graph
        .nodes
        .iter()
        .find(|node| node.name == "build()")
        .ok_or("missing build")?;
    let item = graph
        .nodes
        .iter()
        .find(|node| node.name == "Item")
        .ok_or("missing Item")?;
    let edges = graph
        .links
        .iter()
        .filter(|edge| edge.source == caller_node.id && edge.target == item.id)
        .collect::<Vec<_>>();
    assert_eq!(
        edges.len(),
        3,
        "struct literal type references: {:#?}",
        graph.links
    );
    let mut starts = BTreeSet::new();
    for edge in edges {
        assert_eq!(edge.kind, EdgeKind::References);
        let site = edge
            .relationship_site
            .as_ref()
            .ok_or("missing literal occurrence")?;
        assert_eq!(site.file, "src/lib.rs");
        let start = usize::try_from(site.start_byte)?;
        let end = usize::try_from(site.end_byte)?;
        assert!(matches!(
            caller.get(start..end),
            Some("Record" | "crate::model::Item")
        ));
        assert!(starts.insert(start));
        assert!(
            edge.evidence
                .iter()
                .any(|evidence| evidence.anchors.contains(site))
        );
    }
    let mut reversed = inputs;
    reversed.reverse();
    let resolved = resolve(&reversed, &sources);
    let build =
        BuildEvidence::from_extraction(directory.path(), &resolved, "sha256:struct-literal")?;
    let reversed = normalize_v1(resolved, build)?;
    assert_eq!(
        serde_json::to_value(&graph.nodes)?,
        serde_json::to_value(&reversed.nodes)?
    );
    assert_eq!(
        serde_json::to_value(&graph.links)?,
        serde_json::to_value(&reversed.links)?
    );
    Ok(())
}

fn reference_targets(source: &str, caller_name: &str) -> Result<Vec<String>, Box<dyn Error>> {
    let resolved = resolve(
        &[extract("src/lib.rs", source)?],
        &HashMap::from([("src/lib.rs".into(), source.into())]),
    );
    assert!(resolved.error.is_none(), "{:?}", resolved.error);
    let caller = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == caller_name)
        .ok_or("missing caller")?;
    let mut targets = Vec::new();
    for edge in resolved
        .edges
        .iter()
        .filter(|edge| edge.source == caller.id && edge.string("relation") == "references")
    {
        let target = resolved
            .nodes
            .iter()
            .find(|node| node.id == edge.target)
            .ok_or("missing reference target")?;
        targets.push(target.string("qualified_name"));
    }
    assert!(
        !resolved
            .edges
            .iter()
            .any(|edge| edge.source == caller.id && edge.string("relation") == "calls")
    );
    targets.sort();
    Ok(targets)
}

#[test]
fn self_generics_enum_variants_and_aliases_resolve_without_calls() -> Result<(), Box<dyn Error>> {
    for (source, caller, expected) in [
        (
            "struct Item { value: usize } impl Item { fn make() { let _ = Self { value: 1 }; } }",
            "crate::Item::make",
            "crate::Item",
        ),
        (
            "struct Item<T> { value: T } fn make() { let _ = Item::<u8> { value: 1 }; }",
            "crate::make",
            "crate::Item",
        ),
        (
            "enum Event { Ready { value: usize } } impl Event { fn make() { let _ = Self::Ready { value: 1 }; } }",
            "crate::Event::make",
            "crate::Event::Ready",
        ),
        (
            "struct Item<T> { value: T } impl<T> Item<T> { fn make() { let _ = Self { value: 1 }; } }",
            "crate::Item::make",
            "crate::Item",
        ),
        (
            "mod nested { pub struct Item { pub value: usize } } impl crate::nested::Item { fn make() { let _ = Self { value: 1 }; } }",
            "crate::nested::Item::make",
            "crate::nested::Item",
        ),
        (
            "struct Item { value: usize } type Alias = Item; fn make() { let _ = Alias { value: 1 }; }",
            "crate::make",
            "crate::Alias",
        ),
        (
            "mod nested { pub struct Item { pub value: usize } } fn make() { let _ = nested::Item { value: 1 }; }",
            "crate::make",
            "crate::nested::Item",
        ),
        (
            "struct Item { value: usize } fn make() { let Item = 0; let _ = Item { value: 1 }; }",
            "crate::make",
            "crate::Item",
        ),
    ] {
        assert_eq!(reference_targets(source, caller)?, [expected], "{source}");
    }
    Ok(())
}

#[test]
fn ambiguous_unknown_recovered_and_nonconcrete_self_literals_do_not_choose_targets()
-> Result<(), Box<dyn Error>> {
    for source in [
        "mod a { pub struct Item { pub value: usize } } mod b { pub struct Item { pub value: usize } } use a::Item; use b::Item; fn make() { let _ = Item { value: 1 }; }",
        "mod a { pub struct Item { pub value: usize } } mod b { pub struct Item { pub value: usize } } use a::*; use b::*; fn make() { let _ = Item { value: 1 }; }",
        "mod a { pub struct Item { pub value: usize } } mod b { pub struct Item { pub value: usize } } use a as model; use b as model; fn make() { let _ = model::Item { value: 1 }; }",
        "struct Item { value: usize } struct Item { value: usize } fn make() { let _ = Item { value: 1 }; }",
        "mod a { pub struct Item { pub value: usize } } fn make() { let _ = Item { value: 1 }; }",
        "fn make() { let _ = Missing { value: 1 }; }",
        "struct Item { value: usize } fn make() { let _ = Item { value: }; }",
    ] {
        assert!(
            reference_targets(source, "crate::make")?.is_empty(),
            "{source}"
        );
    }
    let source = "trait Make { fn make() { let _ = Self { value: 1 }; } }";
    assert!(reference_targets(source, "crate::Make::make")?.is_empty());
    Ok(())
}

#[test]
fn impl_self_uses_scoped_import_evidence_instead_of_a_hidden_namesake() -> Result<(), Box<dyn Error>>
{
    let model = "pub struct Item { pub value: usize }";
    let caller = "mod model; mod hidden { pub struct Item { pub value: usize } } use crate::model::Item; impl Item { fn make() { let _ = Self { value: 1 }; } }";
    let inputs = vec![
        extract("src/lib.rs", caller)?,
        extract("src/model.rs", model)?,
    ];
    let sources = HashMap::from([
        ("src/lib.rs".into(), caller.into()),
        ("src/model.rs".into(), model.into()),
    ]);
    let resolved = resolve(&inputs, &sources);
    assert!(resolved.error.is_none(), "{:?}", resolved.error);
    let make = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name").ends_with("::make"))
        .ok_or("missing make")?;
    assert_eq!(make.string("qualified_name"), "crate::model::Item::make");
    let hidden = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::hidden::Item")
        .ok_or("missing hidden Item")?;
    assert!(
        !resolved
            .edges
            .iter()
            .any(|edge| edge.source == hidden.id && edge.target == make.id)
    );
    assert!(
        !resolved
            .edges
            .iter()
            .any(|edge| edge.source == make.id && edge.target == hidden.id)
    );
    let model = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::model::Item")
        .ok_or("missing imported Item")?;
    assert!(resolved.edges.iter().any(|edge| edge.source == make.id
        && edge.target == model.id
        && edge.string("relation") == "references"));
    let directory = tempfile::tempdir()?;
    std::fs::create_dir_all(directory.path().join("src"))?;
    for (file, source) in &sources {
        std::fs::write(directory.path().join(file), source)?;
    }
    let build = BuildEvidence::from_extraction(directory.path(), &resolved, "sha256:scoped-self")?;
    let graph = normalize_v1(resolved, build)?;
    let make = graph
        .nodes
        .iter()
        .find(|node| node.qualified_name == "crate::model::Item::make")
        .ok_or("missing published make")?;
    let model = graph
        .nodes
        .iter()
        .find(|node| node.qualified_name == "crate::model::Item")
        .ok_or("missing published model")?;
    let edges = graph
        .links
        .iter()
        .filter(|edge| edge.source == make.id && edge.target == model.id)
        .collect::<Vec<_>>();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].kind, EdgeKind::References);
    let site = edges[0]
        .relationship_site
        .as_ref()
        .ok_or("missing Self site")?;
    assert_eq!(site.file, "src/lib.rs");
    assert_eq!(
        caller.get(usize::try_from(site.start_byte)?..usize::try_from(site.end_byte)?),
        Some("Self")
    );
    assert!(
        edges[0]
            .evidence
            .iter()
            .any(|evidence| evidence.anchors.contains(site))
    );
    let mut reversed = inputs;
    reversed.reverse();
    let resolved = resolve(&reversed, &sources);
    let build = BuildEvidence::from_extraction(directory.path(), &resolved, "sha256:scoped-self")?;
    let reversed = normalize_v1(resolved, build)?;
    assert_eq!(
        serde_json::to_value(&graph.nodes)?,
        serde_json::to_value(&reversed.nodes)?
    );
    assert_eq!(
        serde_json::to_value(&graph.links)?,
        serde_json::to_value(&reversed.links)?
    );
    Ok(())
}

#[test]
fn literal_type_namespace_is_independent_of_value_parameter_bindings() -> Result<(), Box<dyn Error>>
{
    let source = "struct Item { value: usize } struct Other { value: usize } fn make(Item: Other) { let _ = Item { value: 1 }; }";
    // The signature legitimately references Other; the literal references Item.
    assert_eq!(
        reference_targets(source, "crate::make")?,
        ["crate::Item", "crate::Other"]
    );
    Ok(())
}

#[test]
fn child_modules_and_sibling_callables_do_not_borrow_nominal_bindings() -> Result<(), Box<dyn Error>>
{
    let source =
        "struct Item { value: usize } mod nested { fn make() { let _ = Item { value: 1 }; } }";
    assert!(reference_targets(source, "crate::nested::make")?.is_empty());
    let source = "mod model { pub struct Item { pub value: usize } } fn first() { use crate::model::Item; let _ = Item { value: 1 }; } fn second() { let _ = Item { value: 1 }; }";
    assert_eq!(
        reference_targets(source, "crate::first")?,
        ["crate::model::Item"]
    );
    assert!(reference_targets(source, "crate::second")?.is_empty());
    Ok(())
}

#[test]
fn unknown_and_generic_impl_receivers_never_inherit_a_same_file_type() -> Result<(), Box<dyn Error>>
{
    for (source, unrelated) in [
        (
            "mod hidden { pub struct Item { pub value: usize } } impl Item { fn make() { let _ = Self { value: 1 }; } }",
            "crate::hidden::Item",
        ),
        (
            "struct T { value: usize } trait Build { fn make(); } impl<T> Build for T { fn make() { let _ = Self { value: 1 }; } }",
            "crate::T",
        ),
        (
            "struct Item { value: usize } mod nested { impl Item { fn make() { let _ = Self { value: 1 }; } } }",
            "crate::Item",
        ),
    ] {
        let resolved = resolve(
            &[extract("src/lib.rs", source)?],
            &HashMap::from([("src/lib.rs".into(), source.into())]),
        );
        assert!(resolved.error.is_none(), "{:?}", resolved.error);
        let unrelated = resolved
            .nodes
            .iter()
            .find(|node| node.string("qualified_name") == unrelated)
            .ok_or("missing unrelated type")?;
        let make = resolved
            .nodes
            .iter()
            .find(|node| {
                node.string("qualified_name").ends_with("::make")
                    && node.attributes.contains_key("implementation_hash")
            })
            .ok_or("missing impl method")?;
        assert!(
            !resolved
                .edges
                .iter()
                .any(
                    |edge| (edge.source == unrelated.id && edge.target == make.id)
                        || (edge.source == make.id && edge.target == unrelated.id)
                ),
            "{source}"
        );
    }
    Ok(())
}

#[test]
fn block_imports_preserve_inner_outer_and_sibling_literal_targets() -> Result<(), Box<dyn Error>> {
    let source = "mod a { pub struct Item { pub value: usize } } mod b { pub struct Item { pub value: usize } } use crate::b::Item; fn make() { { use crate::a::Item; let _ = Item { value: 1 }; } let _ = Item { value: 2 }; { use crate::a::Item as Choice; let _ = Choice { value: 3 }; } { use crate::b::Item as Choice; let _ = Choice { value: 4 }; } }";
    assert_eq!(
        reference_targets(source, "crate::make")?,
        [
            "crate::a::Item",
            "crate::a::Item",
            "crate::b::Item",
            "crate::b::Item"
        ]
    );
    Ok(())
}

#[test]
fn block_local_imports_and_aliases_leave_out_of_scope_occurrences_unresolved()
-> Result<(), Box<dyn Error>> {
    for source in [
        "mod a { pub struct Item { pub value: usize } } fn make() { { use crate::a::Item; let _ = Item { value: 1 }; } let _ = Item { value: 2 }; { let _ = Item { value: 3 }; } }",
        "struct Item { value: usize } fn make() { { type Local = Item; let _ = Local { value: 1 }; } let _ = Local { value: 2 }; { let _ = Local { value: 3 }; } }",
    ] {
        let extraction = extract("src/lib.rs", source)?;
        let evidence = extraction
            .semantic_evidence
            .as_ref()
            .ok_or("missing evidence")?;
        let mut occurrences = evidence
            .occurrences
            .iter()
            .filter(|occurrence| occurrence.context.as_deref() == Some("rust-struct-literal"))
            .collect::<Vec<_>>();
        occurrences.sort_by_key(|occurrence| occurrence.range.start_byte);
        assert_eq!(occurrences.len(), 3);
        for (index, occurrence) in occurrences.iter().enumerate() {
            assert_eq!(
                evidence
                    .candidates
                    .iter()
                    .filter(|candidate| candidate.occurrence_id.as_ref() == Some(&occurrence.id))
                    .count(),
                usize::from(index == 0),
                "{source}: literal {index}"
            );
            if index != 0 {
                assert!(evidence.diagnostics.iter().any(|diagnostic| diagnostic.code
                    == "unresolved-struct-literal-type"
                    && diagnostic.fact_id.as_ref() == Some(&occurrence.id)));
            }
        }
    }
    Ok(())
}

#[test]
fn block_nested_impl_receivers_follow_their_own_imports() -> Result<(), Box<dyn Error>> {
    let source = "mod a { pub struct Item { pub value: usize } } mod b { pub struct Item { pub value: usize } } fn setup() { { use crate::a::Item; impl Item { fn first() { let _ = Self { value: 1 }; } } } { use crate::b::Item; impl Item { fn second() { let _ = Self { value: 2 }; } } } }";
    assert_eq!(
        reference_targets(source, "crate::a::Item::first")?,
        ["crate::a::Item"]
    );
    assert_eq!(
        reference_targets(source, "crate::b::Item::second")?,
        ["crate::b::Item"]
    );
    Ok(())
}

#[test]
fn constant_initializer_block_imports_do_not_escape_to_the_enclosing_function()
-> Result<(), Box<dyn Error>> {
    let source = "mod a { pub struct Item { pub value: usize } } fn make() { const VALUE: usize = { use crate::a::Item; let _ = Item { value: 1 }; 1 }; let _ = Item { value: 2 }; }";
    assert_eq!(
        reference_targets(source, "crate::make::VALUE")?,
        ["crate::a::Item"]
    );
    assert!(reference_targets(source, "crate::make")?.is_empty());
    Ok(())
}

#[test]
fn block_local_modules_are_available_before_their_item_statement() -> Result<(), Box<dyn Error>> {
    let source = "fn make() { use model::Item; let _ = Item { value: 1 }; mod model { pub struct Item { pub value: usize } } }";
    assert_eq!(
        reference_targets(source, "crate::make")?,
        ["crate::make::model::Item"]
    );
    Ok(())
}

#[test]
fn module_local_type_is_not_suppressed_by_parent_import_ambiguity() -> Result<(), Box<dyn Error>> {
    let source = "use crate::a::Item; use crate::b::Item; mod nested { struct Item { value: usize } fn make() { let _ = Item { value: 1 }; } }";
    assert_eq!(
        reference_targets(source, "crate::nested::make")?,
        ["crate::nested::Item"]
    );
    Ok(())
}

#[test]
fn same_named_block_aliases_keep_distinct_exact_declarations() -> Result<(), Box<dyn Error>> {
    let source = "struct Item { value: usize } fn make() { { let _ = Local { value: 1 }; type Local = Item; } { type Local = Item; let _ = Local { value: 2 }; } { type Local = Item; let _ = Local { value: 3 }; } }";
    let extraction = extract("src/lib.rs", source)?;
    let evidence = extraction
        .semantic_evidence
        .as_ref()
        .ok_or("missing evidence")?;
    let mut aliases = evidence
        .declarations
        .iter()
        .filter(|declaration| declaration.name == "Local")
        .collect::<Vec<_>>();
    aliases.sort_by_key(|declaration| declaration.range.start_byte);
    assert_eq!(aliases.len(), 3);
    assert_eq!(
        aliases
            .iter()
            .map(|alias| &alias.id)
            .collect::<BTreeSet<_>>()
            .len(),
        3
    );
    assert_eq!(
        aliases
            .iter()
            .map(|alias| &alias.graph_node_id)
            .collect::<BTreeSet<_>>()
            .len(),
        3
    );
    let mut occurrences = evidence
        .occurrences
        .iter()
        .filter(|occurrence| occurrence.context.as_deref() == Some("rust-struct-literal"))
        .collect::<Vec<_>>();
    occurrences.sort_by_key(|occurrence| occurrence.range.start_byte);
    assert_eq!(occurrences.len(), 3);
    for (occurrence, alias) in occurrences.iter().zip(&aliases) {
        let candidates = evidence
            .candidates
            .iter()
            .filter(|candidate| candidate.occurrence_id.as_ref() == Some(&occurrence.id))
            .collect::<Vec<_>>();
        assert_eq!(candidates.len(), 1);
        assert_eq!(
            candidates[0]
                .constraints
                .exact_target_declaration_id
                .as_deref(),
            Some(alias.id.as_str())
        );
    }
    let resolved = resolve(
        &[extraction],
        &HashMap::from([("src/lib.rs".into(), source.into())]),
    );
    assert!(resolved.error.is_none(), "{:?}", resolved.error);
    let make = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "crate::make")
        .ok_or("missing make")?;
    let targets = resolved
        .edges
        .iter()
        .filter(|edge| edge.source == make.id && edge.string("relation") == "references")
        .map(|edge| edge.target.clone())
        .collect::<BTreeSet<_>>();
    assert_eq!(targets.len(), 3);
    Ok(())
}

#[test]
fn repeated_block_declarations_publish_distinct_nodes_and_literal_occurrences()
-> Result<(), Box<dyn Error>> {
    for declaration in ["type Local = Item;", "struct Local { value: usize }"] {
        for separator in [" ", "\n"] {
            let source = format!(
                "struct Item {{ value: usize }} fn make() {{ {{ {{ {declaration} let _ = Local {{ value: 1 }}; }} }}{separator}{{ {declaration} let _ = Local {{ value: 2 }}; }}{separator}{{ {declaration} let _ = Local {{ value: 3 }}; }} }}"
            );
            let extraction = extract("src/lib.rs", &source)?;
            let evidence = extraction
                .semantic_evidence
                .as_ref()
                .ok_or("missing evidence")?;
            let locals = evidence
                .declarations
                .iter()
                .filter(|declaration| declaration.name == "Local")
                .collect::<Vec<_>>();
            assert_eq!(locals.len(), 3);
            assert_eq!(
                locals
                    .iter()
                    .map(|declaration| &declaration.id)
                    .collect::<BTreeSet<_>>()
                    .len(),
                3
            );
            assert_eq!(
                locals
                    .iter()
                    .map(|declaration| &declaration.graph_node_id)
                    .collect::<BTreeSet<_>>()
                    .len(),
                3
            );
            let directory = tempfile::tempdir()?;
            std::fs::create_dir_all(directory.path().join("src"))?;
            std::fs::write(directory.path().join("src/lib.rs"), &source)?;
            std::fs::write(
                directory.path().join("src/other.rs"),
                "pub fn unrelated() {}",
            )?;
            let sources = HashMap::from([
                ("src/lib.rs".into(), source),
                ("src/other.rs".into(), "pub fn unrelated() {}".into()),
            ]);
            let inputs = vec![
                extraction,
                extract("src/other.rs", "pub fn unrelated() {}")?,
            ];
            let normalize = |inputs: &[Extraction]| -> Result<_, Box<dyn Error>> {
                let resolved = resolve(inputs, &sources);
                assert!(resolved.error.is_none(), "{:?}", resolved.error);
                let build = BuildEvidence::from_extraction(
                    directory.path(),
                    &resolved,
                    "sha256:repeated-locals",
                )?;
                Ok(normalize_v1(resolved, build)?)
            };
            let graph = normalize(&inputs)?;
            let locals = graph
                .nodes
                .iter()
                .filter(|node| node.name == "Local")
                .collect::<Vec<_>>();
            assert_eq!(locals.len(), 3);
            assert_eq!(
                locals
                    .iter()
                    .map(|node| &node.id)
                    .collect::<BTreeSet<_>>()
                    .len(),
                3
            );
            let anchors = locals
                .iter()
                .map(|node| {
                    node.source
                        .as_ref()
                        .map(|source| (source.file.as_str(), source.start_byte))
                })
                .collect::<BTreeSet<_>>();
            assert_eq!(anchors.len(), 3);
            assert!(!anchors.contains(&None));
            let make = graph
                .nodes
                .iter()
                .find(|node| node.qualified_name == "crate::make")
                .ok_or("missing make")?;
            let edges = graph
                .links
                .iter()
                .filter(|edge| edge.source == make.id && edge.kind == EdgeKind::References)
                .collect::<Vec<_>>();
            assert_eq!(edges.len(), 3);
            assert_eq!(
                edges
                    .iter()
                    .map(|edge| &edge.target)
                    .collect::<BTreeSet<_>>()
                    .len(),
                3
            );
            for edge in edges {
                let site = edge
                    .relationship_site
                    .as_ref()
                    .ok_or("missing literal site")?;
                assert!(
                    edge.evidence
                        .iter()
                        .any(|evidence| evidence.anchors.contains(site))
                );
            }
            let mut reverse = inputs;
            reverse.reverse();
            let reverse = normalize(&reverse)?;
            assert_eq!(
                serde_json::to_value(&graph.nodes)?,
                serde_json::to_value(&reverse.nodes)?
            );
            assert_eq!(
                serde_json::to_value(&graph.links)?,
                serde_json::to_value(&reverse.links)?
            );
        }
    }
    Ok(())
}

#[test]
fn block_targets_publish_in_source_order_with_provenance_and_input_order_independence()
-> Result<(), Box<dyn Error>> {
    let model = "pub struct Item { pub value: usize }";
    let caller = "mod a; mod b; use crate::b::Item; fn make() { { use crate::a::Item; let _ = Item { value: 1 }; } let _ = Item { value: 2 }; { use crate::a::Item; let _ = Item { value: 3 }; } }";
    let inputs = vec![
        extract("src/lib.rs", caller)?,
        extract("src/a.rs", model)?,
        extract("src/b.rs", model)?,
    ];
    let sources = HashMap::from([
        ("src/lib.rs".into(), caller.into()),
        ("src/a.rs".into(), model.into()),
        ("src/b.rs".into(), model.into()),
    ]);
    let directory = tempfile::tempdir()?;
    std::fs::create_dir_all(directory.path().join("src"))?;
    for (file, source) in &sources {
        std::fs::write(directory.path().join(file), source)?;
    }
    let normalize = |inputs: &[Extraction]| -> Result<_, Box<dyn Error>> {
        let resolved = resolve(inputs, &sources);
        assert!(resolved.error.is_none(), "{:?}", resolved.error);
        let build =
            BuildEvidence::from_extraction(directory.path(), &resolved, "sha256:block-scopes")?;
        Ok(normalize_v1(resolved, build)?)
    };
    let graph = normalize(&inputs)?;
    let make = graph
        .nodes
        .iter()
        .find(|node| node.qualified_name == "crate::make")
        .ok_or("missing make")?;
    let mut edges = graph
        .links
        .iter()
        .filter(|edge| edge.source == make.id && edge.kind == EdgeKind::References)
        .collect::<Vec<_>>();
    edges.sort_by_key(|edge| edge.relationship_site.as_ref().map(|site| site.start_byte));
    assert_eq!(edges.len(), 3);
    let mut sites = BTreeSet::new();
    for (edge, expected) in edges
        .iter()
        .zip(["crate::a::Item", "crate::b::Item", "crate::a::Item"])
    {
        let target = graph
            .nodes
            .iter()
            .find(|node| node.id == edge.target)
            .ok_or("missing target")?;
        assert_eq!(target.qualified_name, expected);
        let site = edge.relationship_site.as_ref().ok_or("missing site")?;
        assert_eq!(site.file, "src/lib.rs");
        assert_eq!(
            caller.get(usize::try_from(site.start_byte)?..usize::try_from(site.end_byte)?),
            Some("Item")
        );
        assert!(sites.insert(site.start_byte));
        assert!(
            edge.evidence
                .iter()
                .any(|evidence| evidence.anchors.contains(site))
        );
    }
    let mut reversed = inputs;
    reversed.reverse();
    let reverse = normalize(&reversed)?;
    assert_eq!(
        serde_json::to_value(&graph.nodes)?,
        serde_json::to_value(&reverse.nodes)?
    );
    assert_eq!(
        serde_json::to_value(&graph.links)?,
        serde_json::to_value(&reverse.links)?
    );
    Ok(())
}

fn assert_generic_call_targets(source: &str, expected: &[&str]) -> Result<(), Box<dyn Error>> {
    let extraction = extract("src/lib.rs", source)?;
    let evidence = extraction
        .semantic_evidence
        .as_ref()
        .ok_or("missing evidence")?;
    let make = evidence
        .declarations
        .iter()
        .find(|declaration| declaration.name == "make")
        .ok_or("missing make")?;
    let mut calls = evidence
        .candidates
        .iter()
        .filter(|candidate| {
            candidate.source_declaration_id == make.id
                && candidate.relation == CandidateRelation::Calls
        })
        .map(|candidate| {
            let occurrence = evidence
                .occurrences
                .iter()
                .find(|occurrence| candidate.occurrence_id.as_ref() == Some(&occurrence.id))
                .ok_or("missing call occurrence")?;
            Ok((occurrence.range.start_byte, candidate))
        })
        .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
    calls.sort_by_key(|(start, _)| *start);
    assert_eq!(calls.len(), expected.len(), "{source}");
    for ((_, candidate), target) in calls.iter().zip(expected) {
        if let Some(qualified) = candidate.constraints.qualified_name.as_deref() {
            assert_eq!(qualified, *target, "{source}");
        } else {
            assert!(
                matches!(
                    candidate.constraints.hierarchy,
                    Some(HierarchyConstraint::RustGenericBounds {
                        bounds_complete: true,
                        ..
                    })
                ),
                "generic calls must defer the complete bound set: {source}"
            );
        }
    }
    let owner_id = make.graph_node_id.clone();
    let resolved = resolve(
        &[extraction],
        &HashMap::from([("src/lib.rs".into(), source.into())]),
    );
    assert!(resolved.error.is_none(), "{:?}", resolved.error);
    let mut targets = resolved
        .edges
        .iter()
        .filter(|edge| edge.source == owner_id && edge.string("relation") == "calls")
        .map(|edge| {
            resolved
                .nodes
                .iter()
                .find(|node| node.id == edge.target)
                .map(|node| node.string("qualified_name"))
                .ok_or("missing call target")
        })
        .collect::<Result<Vec<_>, _>>()?;
    targets.sort();
    let mut sorted_expected = expected.to_vec();
    sorted_expected.sort();
    assert_eq!(targets, sorted_expected, "{source}");
    Ok(())
}

#[test]
fn generic_receiver_lookup_stops_at_a_local_nominal_shadow() -> Result<(), Box<dyn Error>> {
    assert_generic_call_targets(
        "trait Build { fn get(&self); } struct Other; impl Other { fn get(&self) {} } fn make<T: Build>(original: &T) { type T = Other; let local: &T = &Other; local.get(); original.get(); }",
        &["crate::Other::get", "crate::Build::get"],
    )
}

#[test]
fn generic_bound_names_keep_their_declaration_context() -> Result<(), Box<dyn Error>> {
    assert_generic_call_targets(
        "trait Build { fn get(&self); } mod shadow { pub trait Build { fn get(&self); } } fn make<T: Build>(original: &T) { use crate::shadow::Build; let local: &T = original; local.get(); original.get(); }",
        &["crate::Build::get", "crate::Build::get"],
    )
}

#[test]
fn generic_shadowing_preserves_where_impl_nested_and_imported_origins() -> Result<(), Box<dyn Error>>
{
    for (source, expected) in [
        (
            "trait Build { fn get(&self); } struct Other; impl Other { fn get(&self) {} } fn make<T: Build>(original: &T, other: &Other) { { type T = Other; let local: &T = other; local.get(); original.get(); } { original.get(); } }",
            vec![
                "crate::Other::get",
                "crate::Build::get",
                "crate::Build::get",
            ],
        ),
        (
            "trait Build { fn get(&self); } mod model { pub struct Other; impl Other { pub fn get(&self) {} } } fn make<T: Build>(original: &T, other: &crate::model::Other) { use crate::model::Other as T; let local: &T = other; local.get(); original.get(); }",
            vec!["crate::model::Other::get", "crate::Build::get"],
        ),
        (
            "trait Build { fn get(&self); } mod shadow { pub trait Build { fn get(&self); } } fn make<T>(original: &T) where T: Build { use crate::shadow::Build; let local: &T = original; local.get(); original.get(); }",
            vec!["crate::Build::get", "crate::Build::get"],
        ),
        (
            "trait Build { fn get(&self); } struct Other; impl Other { fn get(&self) {} } struct Container<T>(T); impl<T: Build> Container<T> { fn make(original: &T, other: &Other) { type T = Other; let local: &T = other; local.get(); original.get(); } }",
            vec!["crate::Other::get", "crate::Build::get"],
        ),
        (
            "trait Build { fn get(&self); } mod shadow { pub trait Build { fn get(&self); } } struct Container<T>(T); impl<T> Container<T> where T: Build { fn make(original: &T) { use crate::shadow::Build; let local: &T = original; local.get(); original.get(); } }",
            vec!["crate::Build::get", "crate::Build::get"],
        ),
        (
            "trait Build { fn get(&self); } mod shadow { pub trait Build { fn get(&self); } } fn make<T: Build>(items: &[T]) { use crate::shadow::Build; items[0].get(); }",
            vec!["crate::Build::get"],
        ),
    ] {
        assert_generic_call_targets(source, &expected)?;
    }
    Ok(())
}

#[test]
fn generic_field_dispatch_uses_positional_identity_through_nested_and_indexed_fields()
-> Result<(), Box<dyn Error>> {
    for (source, expected) in [
        (
            "trait Left { fn get(&self); } trait Right { fn get(&self); } struct Pair<A, B> { first: A, second: B } impl<X: Left, Y: Right> Pair<Y, X> { fn make(&self) { self.first.get(); self.second.get(); } }",
            vec!["crate::Right::get", "crate::Left::get"],
        ),
        (
            "trait Build { fn get(&self); } struct Inner<A> { value: A } struct Outer<B> { inner: Inner<B> } impl<X: Build> Outer<X> { fn make(&self) { self.inner.value.get(); } }",
            vec!["crate::Build::get"],
        ),
        (
            "trait Build { fn get(&self); } struct Entries<A> { values: Vec<A> } impl<X: Build> Entries<X> { fn make(&self) { self.values[0].get(); } }",
            vec!["crate::Build::get"],
        ),
    ] {
        assert_generic_call_targets(source, &expected)?;
    }
    Ok(())
}

#[test]
fn ambiguous_unsupported_and_cyclic_generic_receiver_types_do_not_invent_calls()
-> Result<(), Box<dyn Error>> {
    for source in [
        "mod a { pub trait Build { fn get(&self); } } mod b { pub trait Build { fn get(&self); } } use crate::a::Build; use crate::b::Build; fn make<T: Build>(value: &T) { value.get(); }",
        "trait Build { fn get(&self); } trait Other { fn get(&self); } fn make<T: Build + Other>(value: &T) { value.get(); }",
        "struct Build; impl Build { fn get(&self) {} } fn make<T: Build>(value: &T) { value.get(); }",
        "trait Build { fn get(&self); } fn make<T: Build>() { type A = B; type B = A; let local: &A = unknown(); local.get(); }",
        "trait Build { fn get(&self); } fn make<T: Build>() { type T = fn(); let local: &T = unknown(); local.get(); }",
    ] {
        let extraction = extract("src/lib.rs", source)?;
        let evidence = extraction
            .semantic_evidence
            .as_ref()
            .ok_or("missing evidence")?;
        let make = evidence
            .declarations
            .iter()
            .find(|declaration| declaration.name == "make")
            .ok_or("missing make")?;
        let owner = make.graph_node_id.clone();
        let declared_get_methods = evidence
            .declarations
            .iter()
            .filter(|declaration| declaration.kind == "method" && declaration.name == "get")
            .map(|declaration| declaration.graph_node_id.clone())
            .collect::<BTreeSet<_>>();
        assert!(
            !evidence
                .candidates
                .iter()
                .any(|candidate| candidate.source_declaration_id == make.id
                    && candidate.relation == CandidateRelation::Calls
                    && candidate
                        .constraints
                        .qualified_name
                        .as_deref()
                        .is_some_and(|target| target.ends_with("Build::get")
                            || target.ends_with("Other::get"))),
            "{source}"
        );
        let resolved = resolve(
            &[extraction],
            &HashMap::from([("src/lib.rs".into(), source.into())]),
        );
        assert!(resolved.error.is_none(), "{:?}", resolved.error);
        assert!(
            !resolved.edges.iter().any(|edge| edge.source == owner
                && edge.string("relation") == "calls"
                && declared_get_methods.contains(&edge.target)),
            "{source}: {:#?}",
            resolved.edges
        );
    }
    Ok(())
}

#[test]
fn generic_bound_origins_publish_exact_calls_with_provenance_and_stable_order()
-> Result<(), Box<dyn Error>> {
    let api = "pub trait Build { fn get(&self); }";
    let caller = "mod api; use crate::api::Build; mod shadow { pub trait Build { fn get(&self); } } struct Other; impl Other { fn get(&self) {} } fn make<T: Build>(original: &T, other: &Other) { use crate::shadow::Build; let local: &T = original; local.get(); original.get(); { type T = Other; let concrete: &T = other; concrete.get(); } original.get(); }";
    let inputs = vec![extract("src/lib.rs", caller)?, extract("src/api.rs", api)?];
    let sources = HashMap::from([
        ("src/lib.rs".into(), caller.into()),
        ("src/api.rs".into(), api.into()),
    ]);
    let directory = tempfile::tempdir()?;
    std::fs::create_dir_all(directory.path().join("src"))?;
    for (file, source) in &sources {
        std::fs::write(directory.path().join(file), source)?;
    }
    let normalize = |inputs: &[Extraction]| -> Result<_, Box<dyn Error>> {
        let resolved = resolve(inputs, &sources);
        assert!(resolved.error.is_none(), "{:?}", resolved.error);
        let build = BuildEvidence::from_extraction(
            directory.path(),
            &resolved,
            "sha256:generic-bound-origins",
        )?;
        Ok(normalize_v1(resolved, build)?)
    };
    let graph = normalize(&inputs)?;
    let make = graph
        .nodes
        .iter()
        .find(|node| node.qualified_name == "crate::make")
        .ok_or("missing make")?;
    let mut calls = graph
        .links
        .iter()
        .filter(|edge| edge.source == make.id && edge.kind == EdgeKind::Calls)
        .collect::<Vec<_>>();
    calls.sort_by_key(|edge| edge.relationship_site.as_ref().map(|site| site.start_byte));
    assert_eq!(calls.len(), 4);
    let mut sites = BTreeSet::new();
    for (edge, (expected, spelling)) in calls.iter().zip([
        ("crate::api::Build::get", "local.get"),
        ("crate::api::Build::get", "original.get"),
        ("crate::Other::get", "concrete.get"),
        ("crate::api::Build::get", "original.get"),
    ]) {
        let target = graph
            .nodes
            .iter()
            .find(|node| node.id == edge.target)
            .ok_or("missing get")?;
        assert_eq!(target.qualified_name, expected);
        let site = edge.relationship_site.as_ref().ok_or("missing call site")?;
        assert_eq!(site.file, "src/lib.rs");
        assert_eq!(
            caller.get(usize::try_from(site.start_byte)?..usize::try_from(site.end_byte)?),
            Some(spelling)
        );
        assert!(sites.insert(site.start_byte));
        assert!(
            edge.evidence
                .iter()
                .any(|evidence| evidence.anchors.contains(site))
        );
    }
    let mut reverse = inputs;
    reverse.reverse();
    let reverse = normalize(&reverse)?;
    assert_eq!(
        serde_json::to_value(&graph.nodes)?,
        serde_json::to_value(&reverse.nodes)?
    );
    assert_eq!(
        serde_json::to_value(&graph.links)?,
        serde_json::to_value(&reverse.links)?
    );
    Ok(())
}

#[test]
fn mixed_local_and_imported_generic_bounds_do_not_select_the_local_method()
-> Result<(), Box<dyn Error>> {
    let caller = "mod api; use crate::api::Remote; trait Local { fn get(&self); } fn make<T: Local + Remote>(value: &T) { value.get(); }";
    let api = "pub trait Remote { fn get(&self); }";
    let inputs = vec![extract("src/lib.rs", caller)?, extract("src/api.rs", api)?];
    let evidence = inputs[0]
        .semantic_evidence
        .as_ref()
        .ok_or("missing evidence")?;
    let make = evidence
        .declarations
        .iter()
        .find(|declaration| declaration.name == "make")
        .ok_or("missing make")?;
    assert!(
        !evidence
            .candidates
            .iter()
            .any(|candidate| candidate.source_declaration_id == make.id
                && candidate.relation == CandidateRelation::Calls
                && candidate
                    .constraints
                    .qualified_name
                    .as_deref()
                    .is_some_and(
                        |target| target.ends_with("Local::get") || target.ends_with("Remote::get")
                    )),
        "a per-file inventory cannot exclude the imported bound"
    );
    let owner = make.graph_node_id.clone();
    let declared_methods = inputs
        .iter()
        .filter_map(|input| input.semantic_evidence.as_ref())
        .flat_map(|batch| &batch.declarations)
        .filter(|declaration| declaration.kind == "method" && declaration.name == "get")
        .map(|declaration| declaration.graph_node_id.clone())
        .collect::<BTreeSet<_>>();
    let sources = HashMap::from([
        ("src/lib.rs".into(), caller.into()),
        ("src/api.rs".into(), api.into()),
    ]);
    let resolved = resolve(&inputs, &sources);
    assert!(resolved.error.is_none(), "{:?}", resolved.error);
    assert!(
        !resolved.edges.iter().any(|edge| edge.source == owner
            && edge.string("relation") == "calls"
            && declared_methods.contains(&edge.target)),
        "ambiguous bounds must not publish a source method target"
    );
    Ok(())
}
