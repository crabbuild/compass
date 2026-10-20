use super::*;
use crate::{Engine, Registry};

#[test]
fn rust_trait_inventory_shared_ancestor_scans_scale_linearly()
-> Result<(), Box<dyn std::error::Error>> {
    let visits = |count: usize,
                  nested: bool,
                  comments: usize|
     -> Result<usize, Box<dyn std::error::Error>> {
        let mut source = String::new();
        if nested {
            source.push_str("mod shared {\n");
        }
        for index in 0..count {
            for _ in 0..comments {
                source.push_str("// attachment prefix\n");
            }
            source.push_str(&format!("#[allow(dead_code)] trait Trait{index} {{}}\n"));
        }
        if nested {
            source.push_str("}\n");
        }
        let path = Path::new("src/lib.rs");
        let spec = Registry::resolve(path).ok_or("missing Rust spec")?;
        let pipeline =
            Registry::universal_evidence_pipeline_for_spec(spec).ok_or("missing pipeline")?;
        let tree = Engine::default().parse(path, spec, source.as_bytes())?;
        let root = tree.root_node();
        let mut state =
            DirectEvidenceState::new(path, "src/lib.rs", source.as_bytes(), root, pipeline, None);
        state.add_file(root)?;
        state.extract_rust(root)?;
        assert_eq!(
            state
                .builder
                .batch
                .declarations
                .iter()
                .filter(|declaration| declaration.kind == "trait"
                    && declaration.member_inventory_complete == Some(true))
                .count(),
            count
        );
        Ok(state.rust_trait_member_scan_visits.get())
    };
    for nested in [false, true] {
        for comments in [0, 16] {
            let small = visits(32, nested, comments)?;
            let large = visits(64, nested, comments)?;
            assert!(
                large <= small * 3,
                "shared={nested}, comments={comments}, N visits={small}, 2N visits={large}"
            );
        }
    }
    Ok(())
}

#[test]
fn rust_trait_inventory_scan_exhaustion_is_a_typed_error() -> Result<(), Box<dyn std::error::Error>>
{
    let source = b"trait Build { fn get(&self); }";
    let path = Path::new("src/lib.rs");
    let spec = Registry::resolve(path).ok_or("missing Rust spec")?;
    let pipeline =
        Registry::universal_evidence_pipeline_for_spec(spec).ok_or("missing pipeline")?;
    let tree = Engine::default().parse(path, spec, source)?;
    let root = tree.root_node();
    let mut state = DirectEvidenceState::new(path, "src/lib.rs", source, root, pipeline, None);
    state.add_file(root)?;
    state.rust_trait_member_scan_remaining = 3;
    let error = state
        .extract_rust(root)
        .err()
        .ok_or("expected scan quota error")?;
    assert_eq!(error.code, EvidenceErrorCode::ResourceLimit);
    assert_eq!(state.rust_trait_member_scan_remaining, 0);
    assert!(
        !state
            .builder
            .batch
            .declarations
            .iter()
            .any(|declaration| declaration.member_inventory_complete == Some(true))
    );
    Ok(())
}

#[test]
fn rust_trait_inventory_depth_and_attribute_attachment_boundaries_are_honest()
-> Result<(), Box<dyn std::error::Error>> {
    let proof = |source: &str| -> Result<Option<bool>, Box<dyn std::error::Error>> {
        let extraction =
            Engine::default().extract_source(Path::new("src/lib.rs"), source.as_bytes())?;
        let batch = extraction.semantic_evidence.ok_or("missing evidence")?;
        Ok(batch
            .declarations
            .iter()
            .find(|declaration| declaration.name == "Leaf")
            .ok_or("missing trait")?
            .member_inventory_complete)
    };
    for (modules, complete) in [(31, true), (32, false)] {
        let source = format!(
            "{} trait Leaf {{}} {}",
            "mod nested {".repeat(modules),
            "}".repeat(modules)
        );
        assert_eq!(proof(&source)?, Some(complete), "modules={modules}");
    }
    for (source, complete) in [
        (
            "#[transform] struct Unrelated; #[allow(dead_code)] trait Leaf {}",
            true,
        ),
        ("#![transform] trait Leaf {}", false),
        ("mod nested { #![transform] trait Leaf {} }", false),
        ("trait Leaf { #[transform] fn get(&self); }", false),
        (
            "#[doc = \"description\"] trait Leaf { fn get(&self) { helper!(); } }",
            true,
        ),
    ] {
        assert_eq!(proof(source)?, Some(complete), "{source}");
    }
    Ok(())
}

fn literal(node: Node<'_>) -> Option<Node<'_>> {
    if node.kind() == "struct_expression" {
        return Some(node);
    }
    let mut cursor = node.walk();
    node.children(&mut cursor).find_map(literal)
}

#[test]
fn rust_literal_local_recovery_does_not_consume_disjoint_ranges()
-> Result<(), Box<dyn std::error::Error>> {
    let source = b"struct Item { value: usize } fn make() { let _ = Item { value: 1 }; later(); }";
    let path = Path::new("src/lib.rs");
    let spec = Registry::resolve(path).ok_or("missing Rust spec")?;
    let pipeline =
        Registry::universal_evidence_pipeline_for_spec(spec).ok_or("missing Rust pipeline")?;
    let tree = Engine::default().parse(path, spec, source)?;
    let root = tree.root_node();
    let literal = literal(root).ok_or("missing literal AST")?;
    assert!(!literal.has_error());
    let start = literal.start_byte();
    let end = literal.end_byte();
    for (range, emitted) in [
        ((end + 2, end + 4), true),
        ((end + 2, end + 2), true),
        ((end, end), true),
        ((start - 2, start), true),
        ((start - 2, start - 2), true),
        ((start, start), false),
        ((start + 2, start + 4), false),
    ] {
        let mut state = DirectEvidenceState::new(path, "src/lib.rs", source, root, pipeline, None);
        state.add_file(root)?;
        let file = state.file.clone().ok_or("missing file context")?;
        state.collect_rust_containers(root, &file)?;
        state.collect_rust_declarations(root, &file, None)?;
        let owner = state
            .declarations
            .values()
            .find(|owner| owner.name == "make")
            .cloned()
            .ok_or("missing make context")?;
        state.parser_error_ranges = vec![range];
        // Exercise the actual producer with a retained recovery entry whose
        // exact position is known, rather than relying on one parser shape.
        assert_eq!(state.parser_error_ranges, [range]);
        state.add_rust_struct_literal(literal, &owner)?;
        assert_eq!(
            state
                .builder
                .batch
                .occurrences
                .iter()
                .any(|occurrence| occurrence.context.as_deref() == Some("rust-struct-literal")),
            emitted,
            "literal={start}..{end}, recovery={range:?}"
        );
    }
    Ok(())
}

#[test]
fn rust_block_scope_exhaustion_is_a_resource_error() -> Result<(), Box<dyn std::error::Error>> {
    let source = b"struct Item { value: usize } fn make() { { { let _ = Item { value: 1 }; } } }";
    let path = Path::new("src/lib.rs");
    let spec = Registry::resolve(path).ok_or("missing Rust spec")?;
    let pipeline =
        Registry::universal_evidence_pipeline_for_spec(spec).ok_or("missing pipeline")?;
    let tree = Engine::default().parse(path, spec, source)?;
    let root = tree.root_node();
    let mut state = DirectEvidenceState::new(path, "src/lib.rs", source, root, pipeline, None);
    state.builder.limits.scopes = 4;
    state.add_file(root)?;
    let error = state
        .extract_rust(root)
        .err()
        .ok_or("scope limit must fail")?;
    assert_eq!(error.code, EvidenceErrorCode::ResourceLimit);
    Ok(())
}

#[test]
fn rust_receiver_argument_mapping_is_balanced_and_bounded() {
    assert_eq!(
        rust_receiver_type_arguments("Pair<Other<One, Two>, [u8; 2]>"),
        Some(vec!["Other<One, Two>", "[u8; 2]"])
    );
    assert_eq!(
        rust_receiver_type_arguments("Pair<(One, Two), Three,>"),
        Some(vec!["(One, Two)", "Three"])
    );
    for unsupported in [
        "Pair<One,, Two>",
        "Pair<One, Two",
        "Pair<One)>",
        "Pair<'a, One>",
        "Pair<{ 1 < 2 }>",
        "Pair<\"a,b\">",
        "Pair<One>::Other<Two>",
    ] {
        assert!(
            rust_receiver_type_arguments(unsupported).is_none(),
            "{unsupported}"
        );
    }
    let at_limit = format!("Wrapper<{}>", vec!["Item"; 64].join(","));
    assert_eq!(
        rust_receiver_type_arguments(&at_limit).map(|arguments| arguments.len()),
        Some(64)
    );
    let over_limit = format!("Wrapper<{}>", vec!["Item"; 65].join(","));
    assert!(rust_receiver_type_arguments(&over_limit).is_none());
}

#[test]
fn rust_actual_substitution_chains_and_merged_maps_obey_limits()
-> Result<(), Box<dyn std::error::Error>> {
    let parameters = (0..65).map(|index| format!("P{index}")).collect::<Vec<_>>();
    let source = format!(
        "struct Holder<T> {{ value: T }} fn make<{}>() {{}}",
        parameters.join(",")
    );
    let path = Path::new("src/lib.rs");
    let spec = Registry::resolve(path).ok_or("missing Rust spec")?;
    let pipeline =
        Registry::universal_evidence_pipeline_for_spec(spec).ok_or("missing pipeline")?;
    let tree = Engine::default().parse(path, spec, source.as_bytes())?;
    let root = tree.root_node();
    let mut state =
        DirectEvidenceState::new(path, "src/lib.rs", source.as_bytes(), root, pipeline, None);
    state.add_file(root)?;
    state.extract_rust(root)?;
    let owner = state
        .declarations
        .values()
        .find(|declaration| declaration.name == "make")
        .ok_or("missing make")?
        .clone();
    let parameter_ids = parameters
        .iter()
        .map(|name| {
            state
                .rust_generic_parameter_at_origin(&owner, name)
                .map(|parameter| parameter.fact_id.clone())
                .ok_or("missing exact parameter")
        })
        .collect::<Result<Vec<_>, _>>()?;
    let plain = RustSourceType {
        raw: "usize".to_owned(),
        owner: owner.clone(),
        source_start: 0,
        substitutions: BTreeMap::new(),
    };
    for (steps, accepted) in [(16, true), (17, false)] {
        let mut receiver = plain.clone();
        for index in (0..steps).rev() {
            receiver = RustSourceType {
                raw: parameters[index].clone(),
                owner: owner.clone(),
                source_start: 0,
                substitutions: BTreeMap::from([(parameter_ids[index].clone(), Arc::new(receiver))]),
            };
        }
        let actual = state.rust_instantiated_receiver(&receiver, "P0");
        assert_eq!(actual.is_some(), accepted, "steps={steps}");
        if let Some((_, nominal)) = actual {
            assert_eq!(nominal, "usize");
        }
    }
    let field = state
        .rust_field_types
        .get("crate::Holder")
        .and_then(|fields| fields.get("value"))
        .ok_or("missing field")?;
    let receiver = RustSourceType {
        raw: "Holder<usize>".to_owned(),
        ..plain.clone()
    };
    for (existing, accepted) in [(63, true), (64, false)] {
        let mut field = field.clone();
        field.substitutions = parameter_ids
            .iter()
            .take(existing)
            .map(|id| (id.clone(), Arc::new(plain.clone())))
            .collect();
        let actual = state.rust_instantiated_field_type(&receiver, &field);
        assert_eq!(actual.is_some(), accepted, "existing={existing}");
        if let Some(actual) = actual {
            assert_eq!(actual.substitutions.len(), 64);
            assert_eq!(
                state
                    .rust_instantiated_receiver(&actual, "T")
                    .map(|(_, nominal)| nominal),
                Some("usize".to_owned())
            );
        }
    }
    Ok(())
}
