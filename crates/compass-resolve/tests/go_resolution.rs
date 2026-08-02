use std::collections::HashMap;
use std::error::Error;
use std::path::Path;

use compass_languages::{Engine, RawNodeRecord};
use serde_json::{Map, Value};

#[test]
fn go_nested_calls_follow_cross_file_method_return_types() -> Result<(), Box<dyn Error>> {
    let provider_path = Path::new("storage/elements.go");
    let provider_source = br#"package storage
type element struct{}
func (*element) flags() uint32 { return 0 }
type unrelated struct{}
func (*unrelated) flags() uint32 { return 0 }
type page struct{}
func (*page) element() *element { return nil }
func (*page) ambiguous() (*element, *unrelated) { return nil, nil }
"#;
    let caller_path = Path::new("storage/read.go");
    let caller_source = br#"package storage
type item struct{}
func (*item) setFlags(uint32) {}
func read(p *page, item *item) {
    element := p.element()
    item.setFlags(element.flags())
    p.element().flags()
    first, _ := p.ambiguous()
    first.flags()
}
"#;
    let mut engine = Engine::default();
    let extractions = [
        engine.extract_source(provider_path, provider_source)?,
        engine.extract_source(caller_path, caller_source)?,
    ];
    let resolved = compass_resolve::resolve_with_root(
        &extractions,
        &HashMap::from([
            (
                provider_path.to_string_lossy().into_owned(),
                String::from_utf8(provider_source.to_vec())?,
            ),
            (
                caller_path.to_string_lossy().into_owned(),
                String::from_utf8(caller_source.to_vec())?,
            ),
        ]),
        Path::new("."),
    );
    let read = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "storage.read")
        .ok_or("missing read")?;
    for (qualified_name, expected_sites) in [
        (
            "storage.item::setFlags",
            std::collections::BTreeSet::from(["L6".to_owned()]),
        ),
        (
            "storage.element::flags",
            std::collections::BTreeSet::from(["L6".to_owned(), "L7".to_owned(), "L9".to_owned()]),
        ),
    ] {
        let target = resolved
            .nodes
            .iter()
            .find(|node| node.string("qualified_name") == qualified_name)
            .ok_or_else(|| format!("missing {qualified_name}"))?;
        let sites = resolved
            .edges
            .iter()
            .filter(|edge| {
                edge.source == read.id
                    && edge.target == target.id
                    && edge.string("relation") == "calls"
                    && edge.string("source_file") == caller_path.to_string_lossy()
            })
            .map(|edge| edge.string("source_location"))
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(sites, expected_sites, "target={qualified_name}");
    }
    let unrelated = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "storage.unrelated::flags")
        .ok_or("missing unrelated flags")?;
    assert!(resolved.edges.iter().all(|edge| {
        edge.source != read.id || edge.target != unrelated.id || edge.string("relation") != "calls"
    }));
    Ok(())
}

#[test]
fn go_nested_fields_do_not_reuse_a_same_named_outer_field_type() -> Result<(), Box<dyn Error>> {
    let path = Path::new("storage/nested_fields.go");
    let source = br#"package storage
type item struct{}
func (*item) touch() {}
type other struct{}
func (*other) touch() {}
type branch struct { leaf *other }
type root struct { leaf *item; branch *branch }
func (r *root) run() {
    r.leaf.touch()
    r.branch.leaf.touch()
}
"#;
    let extracted = Engine::default().extract_source(path, source)?;
    let resolved = compass_resolve::resolve_with_root(
        &[extracted],
        &HashMap::from([(
            path.to_string_lossy().into_owned(),
            String::from_utf8(source.to_vec())?,
        )]),
        Path::new("."),
    );
    for (qualified_name, expected_sites) in [
        (
            "storage.item::touch",
            std::collections::BTreeSet::from(["L9".to_owned()]),
        ),
        (
            "storage.other::touch",
            std::collections::BTreeSet::from(["L10".to_owned()]),
        ),
    ] {
        let target = resolved
            .nodes
            .iter()
            .find(|node| node.string("qualified_name") == qualified_name)
            .ok_or_else(|| format!("missing {qualified_name}"))?;
        let call_sites = resolved
            .edges
            .iter()
            .filter(|edge| edge.target == target.id && edge.string("relation") == "calls")
            .map(|edge| edge.string("source_location"))
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(call_sites, expected_sites);
    }
    Ok(())
}

#[test]
fn go_explicit_embedded_field_selectors_resolve_the_embedded_receiver_method()
-> Result<(), Box<dyn Error>> {
    let path = Path::new("command/options.go");
    let source = br#"package command
type baseOptions struct{}
func (*baseOptions) AddFlags() {}
type clearOptions struct { baseOptions }
func (o *clearOptions) AddFlags() {
    o.baseOptions.AddFlags()
}
"#;
    let extracted = Engine::default().extract_source(path, source)?;
    let resolved = compass_resolve::resolve_with_root(
        &[extracted],
        &HashMap::from([(
            path.to_string_lossy().into_owned(),
            String::from_utf8(source.to_vec())?,
        )]),
        Path::new("."),
    );
    let caller = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "command.clearOptions::AddFlags")
        .ok_or("missing outer AddFlags")?;
    let target = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "command.baseOptions::AddFlags")
        .ok_or("missing embedded AddFlags")?;

    assert!(resolved.edges.iter().any(|edge| {
        edge.source == caller.id
            && edge.target == target.id
            && edge.string("relation") == "calls"
            && edge.string("source_location") == "L6"
    }));
    Ok(())
}

#[test]
fn go_external_test_package_calls_do_not_collide_with_production_helpers()
-> Result<(), Box<dyn Error>> {
    let production_path = Path::new("cmd/command/surgery.go");
    let production_source = br#"package command
func readMetaPage() {}
func rebuild() { readMetaPage() }
"#;
    let test_path = Path::new("cmd/command/utils_test.go");
    let test_source = br#"package command_test
func readMetaPage() {}
func testPage() { readMetaPage() }
"#;
    let mut engine = Engine::default();
    let extractions = [
        engine.extract_source(production_path, production_source)?,
        engine.extract_source(test_path, test_source)?,
    ];
    let resolved = compass_resolve::resolve_with_root(
        &extractions,
        &HashMap::from([
            (
                production_path.to_string_lossy().into_owned(),
                String::from_utf8(production_source.to_vec())?,
            ),
            (
                test_path.to_string_lossy().into_owned(),
                String::from_utf8(test_source.to_vec())?,
            ),
        ]),
        Path::new("."),
    );
    for (caller_name, target_name) in [
        ("cmd/command.rebuild", "cmd/command.readMetaPage"),
        ("cmd/command_test.testPage", "cmd/command_test.readMetaPage"),
    ] {
        let caller = resolved
            .nodes
            .iter()
            .find(|node| node.string("qualified_name") == caller_name)
            .ok_or_else(|| format!("missing {caller_name}"))?;
        let target = resolved
            .nodes
            .iter()
            .find(|node| node.string("qualified_name") == target_name)
            .ok_or_else(|| format!("missing {target_name}"))?;
        assert!(resolved.edges.iter().any(|edge| {
            edge.source == caller.id
                && edge.target == target.id
                && edge.string("relation") == "calls"
        }));
    }
    Ok(())
}

#[test]
fn go_nested_selectors_follow_exact_owner_qualified_field_types() -> Result<(), Box<dyn Error>> {
    let path = Path::new("storage/fields.go");
    let source = br#"package storage
type item struct{}
func (*item) touch() {}
type other struct{}
func (*other) touch() {}
type holder struct { item *item; other *other; next *holder }
func (h *holder) run() {
    h.item.touch()
    h.other.touch()
    h.next.item.touch()
    item := &other{}
    item.touch()
}
type holders []*holder
func (*holder) split() holders { return nil }
func (h *holder) spill() {
    var values = h.split()
    for _, value := range values {
        value.next.item.touch()
    }
}
"#;
    let extracted = Engine::default().extract_source(path, source)?;
    let resolved = compass_resolve::resolve_with_root(
        &[extracted],
        &HashMap::from([(
            path.to_string_lossy().into_owned(),
            String::from_utf8(source.to_vec())?,
        )]),
        Path::new("."),
    );
    for (qualified_name, expected_sites) in [
        (
            "storage.item::touch",
            std::collections::BTreeSet::from(["L8".to_owned(), "L10".to_owned(), "L19".to_owned()]),
        ),
        (
            "storage.other::touch",
            std::collections::BTreeSet::from(["L9".to_owned(), "L12".to_owned()]),
        ),
    ] {
        let target = resolved
            .nodes
            .iter()
            .find(|node| node.string("qualified_name") == qualified_name)
            .ok_or_else(|| format!("missing {qualified_name}"))?;
        let call_sites = resolved
            .edges
            .iter()
            .filter(|edge| edge.target == target.id && edge.string("relation") == "calls")
            .map(|edge| edge.string("source_location"))
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(call_sites, expected_sites);
    }
    Ok(())
}

#[test]
fn go_indexed_receivers_use_the_exact_collection_element_owner() -> Result<(), Box<dyn Error>> {
    let path = Path::new("storage/index.go");
    let source = br#"package storage
type item struct{}
func (*item) size() {}
type other struct{}
func (*other) size() {}
type items []*item
type bucket struct { items []*item; byName map[string]*item }
type otherBucket struct { items []*other }
func (*bucket) values() []*item { return nil }
func (b *bucket) run(o *otherBucket) {
    b.items[0].size()
    b.byName["first"].size()
    b.values()[0].size()
    o.items[0].size()
}
func named(values items) {
    values[0].size()
    local := make(items, 1)
    local[0].size()
}
func shadow() {
    type items []*other
    local := make(items, 1)
    local[0].size()
}
"#;
    let extracted = Engine::default().extract_source(path, source)?;
    let resolved = compass_resolve::resolve_with_root(
        &[extracted],
        &HashMap::from([(
            path.to_string_lossy().into_owned(),
            String::from_utf8(source.to_vec())?,
        )]),
        Path::new("."),
    );
    for (qualified_name, expected_sites) in [
        (
            "storage.item::size",
            std::collections::BTreeSet::from([
                "L11".to_owned(),
                "L12".to_owned(),
                "L13".to_owned(),
                "L17".to_owned(),
                "L19".to_owned(),
            ]),
        ),
        (
            "storage.other::size",
            std::collections::BTreeSet::from(["L14".to_owned()]),
        ),
    ] {
        let target = resolved
            .nodes
            .iter()
            .find(|node| node.string("qualified_name") == qualified_name)
            .ok_or_else(|| format!("missing {qualified_name}"))?;
        let call_sites = resolved
            .edges
            .iter()
            .filter(|edge| edge.target == target.id && edge.string("relation") == "calls")
            .map(|edge| edge.string("source_location"))
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(call_sites, expected_sites);
        assert!(!call_sites.contains("L24"));
    }
    Ok(())
}

#[test]
fn go_range_values_use_exact_collection_element_types_without_typing_indexes()
-> Result<(), Box<dyn Error>> {
    let path = Path::new("storage/bucket.go");
    let source = br#"package storage
type item struct{}
func (*item) size() {}
type bucket struct { items map[string]*item }
func (*bucket) values() []*item { return nil }
func (b *bucket) run() {
    values := b.values()
    for _, item := range values {
        for i := 0; i < 1; i++ {
            item.size()
        }
        item.size()
    }
    for _, item := range b.items {
        item.size()
    }
    for item := range values {
        item.size()
    }
}
"#;
    let extracted = Engine::default().extract_source(path, source)?;
    let resolved = compass_resolve::resolve_with_root(
        &[extracted],
        &HashMap::from([(
            path.to_string_lossy().into_owned(),
            String::from_utf8(source.to_vec())?,
        )]),
        Path::new("."),
    );
    let size = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "storage.item::size")
        .ok_or("missing item.size")?;
    let call_sites = resolved
        .edges
        .iter()
        .filter(|edge| edge.target == size.id && edge.string("relation") == "calls")
        .map(|edge| edge.string("source_location"))
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        call_sites,
        std::collections::BTreeSet::from(["L10".to_owned(), "L12".to_owned(), "L15".to_owned(),])
    );
    assert!(!call_sites.contains("L18"));
    Ok(())
}

#[test]
fn go_named_result_receivers_retain_their_declared_type() -> Result<(), Box<dyn Error>> {
    let path = Path::new("storage/db.go");
    let source = br#"package storage
type DB struct{}
func (*DB) Begin() {}
func (*DB) Mmap() {}
func Open() (db *DB) {
    db = &DB{}
    db.Begin()
    db.Mmap()
    return db
}
"#;
    let extracted = Engine::default().extract_source(path, source)?;
    let resolved = compass_resolve::resolve_with_root(
        &[extracted],
        &HashMap::from([(
            path.to_string_lossy().into_owned(),
            String::from_utf8(source.to_vec())?,
        )]),
        Path::new("."),
    );
    let open = resolved
        .nodes
        .iter()
        .find(|node| node.label() == "Open()")
        .ok_or("missing Open")?;
    for (qualified_name, line) in [("storage.DB::Begin", "L7"), ("storage.DB::Mmap", "L8")] {
        let target = resolved
            .nodes
            .iter()
            .find(|node| node.string("qualified_name") == qualified_name)
            .ok_or_else(|| format!("missing {qualified_name}"))?;
        assert!(resolved.edges.iter().any(|edge| {
            edge.source == open.id
                && edge.target == target.id
                && edge.string("relation") == "calls"
                && edge.string("source_location") == line
        }));
    }
    Ok(())
}

#[test]
fn go_chained_call_receivers_follow_exact_return_types() -> Result<(), Box<dyn Error>> {
    let path = Path::new("storage/page.go");
    let source = br#"package storage

type Page struct{}
type Meta struct{}
func (*Page) Meta() *Meta { return &Meta{} }
func (*Meta) Validate() {}
func page() *Page { return &Page{} }
func run() {
    page().Meta().Validate()
}
"#;
    let extracted = Engine::default().extract_source(path, source)?;
    let resolved = compass_resolve::resolve_with_root(
        &[extracted],
        &HashMap::from([(
            path.to_string_lossy().into_owned(),
            String::from_utf8(source.to_vec())?,
        )]),
        Path::new("."),
    );
    let run = resolved
        .nodes
        .iter()
        .find(|node| node.label() == "run()")
        .ok_or("missing run")?;
    for qualified_name in ["storage.Page::Meta", "storage.Meta::Validate"] {
        let target = resolved
            .nodes
            .iter()
            .find(|node| node.string("qualified_name") == qualified_name)
            .ok_or_else(|| format!("missing {qualified_name}"))?;
        assert!(resolved.edges.iter().any(|edge| {
            edge.source == run.id
                && edge.target == target.id
                && edge.string("relation") == "calls"
                && edge.string("source_location") == "L9"
        }));
    }
    Ok(())
}

#[test]
fn go_type_conversions_are_references_while_functions_remain_calls() -> Result<(), Box<dyn Error>> {
    let provider_path = Path::new("internal/common/types.go");
    let provider_source = br#"package common

type Pgid uint64
type Element struct{}
func (Element) Pgid() Pgid { return 0 }
func PgidFunc(value uint64) uint64 { return value }
"#;
    let caller_path = Path::new("internal/freelist/array.go");
    let caller_source = br#"package freelist
import "example.com/project/internal/common"

func convert(value uint64) common.Pgid {
    return common.Pgid(value)
}
func invoke(value uint64) uint64 {
    return common.PgidFunc(value)
}
"#;
    let mut engine = Engine::default();
    let extractions = [
        engine.extract_source(provider_path, provider_source)?,
        engine.extract_source(caller_path, caller_source)?,
    ];
    let sources = HashMap::from([
        (
            provider_path.to_string_lossy().into_owned(),
            String::from_utf8(provider_source.to_vec())?,
        ),
        (
            caller_path.to_string_lossy().into_owned(),
            String::from_utf8(caller_source.to_vec())?,
        ),
    ]);
    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, Path::new("."));
    let convert = resolved
        .nodes
        .iter()
        .find(|node| {
            node.label() == "convert()"
                && node.string("source_file") == caller_path.to_string_lossy()
        })
        .ok_or("missing convert")?;
    let invoke = resolved
        .nodes
        .iter()
        .find(|node| {
            node.label() == "invoke()"
                && node.string("source_file") == caller_path.to_string_lossy()
        })
        .ok_or("missing invoke")?;
    let pgid = resolved
        .nodes
        .iter()
        .find(|node| {
            node.label() == "Pgid" && node.string("source_file") == provider_path.to_string_lossy()
        })
        .ok_or("missing Pgid")?;
    let pgid_func = resolved
        .nodes
        .iter()
        .find(|node| {
            node.label() == "PgidFunc()"
                && node.string("source_file") == provider_path.to_string_lossy()
        })
        .ok_or("missing PgidFunc")?;

    assert!(resolved.edges.iter().any(|edge| {
        edge.source == convert.id
            && edge.target == pgid.id
            && edge.string("relation") == "references"
            && edge.string("source_location") == "L5"
    }));
    assert!(!resolved.edges.iter().any(|edge| {
        edge.source == convert.id && edge.target == pgid.id && edge.string("relation") == "calls"
    }));
    assert!(resolved.edges.iter().any(|edge| {
        edge.source == invoke.id
            && edge.target == pgid_func.id
            && edge.string("relation") == "calls"
            && edge.string("source_location") == "L8"
    }));
    Ok(())
}

#[test]
fn go_interface_methods_keep_exact_interface_ownership_and_source_sites()
-> Result<(), Box<dyn Error>> {
    let path = Path::new("storage/store.go");
    let source = br#"package storage

type Store interface {
    Read(key []byte) error
    Close() error
}

type store struct{}
func (s *store) Read(key []byte) error { return nil }
func (s *store) Close() error { return nil }
"#;
    let extracted = Engine::default().extract_source(path, source)?;
    let resolved = compass_resolve::resolve_with_root(
        &[extracted],
        &HashMap::from([(
            path.to_string_lossy().into_owned(),
            String::from_utf8(source.to_vec())?,
        )]),
        Path::new("."),
    );
    let interface = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "storage.Store")
        .ok_or("missing Store interface")?;
    let concrete = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "storage.store")
        .ok_or("missing store struct")?;

    for (name, line) in [("Read", "L4"), ("Close", "L5")] {
        let interface_method = resolved
            .nodes
            .iter()
            .find(|node| {
                node.string("qualified_name") == format!("storage.Store::{name}")
                    && node.string("source_location") == line
            })
            .ok_or_else(|| format!("missing interface method {name} at {line}"))?;
        let concrete_method = resolved
            .nodes
            .iter()
            .find(|node| node.string("qualified_name") == format!("storage.store::{name}"))
            .ok_or_else(|| format!("missing concrete method {name}"))?;
        assert_ne!(interface_method.id, concrete_method.id);
        assert!(resolved.edges.iter().any(|edge| {
            edge.source == interface.id
                && edge.target == interface_method.id
                && edge.string("relation") == "contains"
        }));
        assert!(resolved.edges.iter().any(|edge| {
            edge.source == concrete.id
                && edge.target == concrete_method.id
                && edge.string("relation") == "method"
        }));
    }
    Ok(())
}

#[test]
fn cross_file_go_receiver_methods_use_the_declared_type_owner() -> Result<(), Box<dyn Error>> {
    let declaration_path = Path::new("cmd/agent/vogon.go");
    let declaration_source = b"package agent\n\ntype Agent struct{}\n";
    let methods_path = Path::new("cmd/agent/hooks.go");
    let methods_source = br#"package agent

func (a *Agent) Prepare() {}
func (a *Agent) Finish() {}
"#;

    let mut engine = Engine::default();
    let declaration = engine.extract_source(declaration_path, declaration_source)?;
    let methods = engine.extract_source(methods_path, methods_source)?;
    let sources = HashMap::from([
        (
            declaration_path.to_string_lossy().into_owned(),
            String::from_utf8(declaration_source.to_vec())?,
        ),
        (
            methods_path.to_string_lossy().into_owned(),
            String::from_utf8(methods_source.to_vec())?,
        ),
    ]);

    let resolved =
        compass_resolve::resolve_with_root(&[declaration, methods], &sources, Path::new("."));
    let owners = resolved
        .nodes
        .iter()
        .filter(|node| node.label() == "Agent")
        .collect::<Vec<_>>();

    assert_eq!(
        owners.len(),
        1,
        "receiver placeholders must be rebound before source-ID disambiguation: {:?}",
        resolved.nodes
    );
    let owner_id = &owners[0].id;
    let method_edges = resolved
        .edges
        .iter()
        .filter(|edge| edge.string("relation") == "method")
        .collect::<Vec<_>>();
    assert_eq!(method_edges.len(), 2, "edges={:?}", resolved.edges);
    assert!(
        method_edges.iter().all(|edge| &edge.source == owner_id),
        "every receiver method must be owned by the declared type: {method_edges:?}"
    );
    assert_eq!(
        owners[0].string("source_file"),
        declaration_path.to_string_lossy()
    );
    Ok(())
}

#[test]
fn imported_go_local_types_resolve_members_to_repository_sources() -> Result<(), Box<dyn Error>> {
    let provider_path = Path::new("bucket/stats.go");
    let provider_source = br#"package bucket
type Stats struct{}
func (s *Stats) Add() {}
"#;
    let caller_path = Path::new("cmd/report/main.go");
    let caller_source = br#"package report
import "example.com/project/bucket"
func run() {
    var stats bucket.Stats
    stats.Add()
}
"#;
    let mut engine = Engine::default();
    let extractions = [
        engine.extract_source(provider_path, provider_source)?,
        engine.extract_source(caller_path, caller_source)?,
    ];
    let sources = HashMap::from([
        (
            provider_path.to_string_lossy().into_owned(),
            String::from_utf8(provider_source.to_vec())?,
        ),
        (
            caller_path.to_string_lossy().into_owned(),
            String::from_utf8(caller_source.to_vec())?,
        ),
    ]);
    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, Path::new("."));
    let run = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "cmd/report.run")
        .ok_or("missing run")?;
    let add = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "bucket.Stats::Add")
        .ok_or("missing Add")?;
    assert!(
        resolved.edges.iter().any(|edge| {
            edge.source == run.id
                && edge.target == add.id
                && edge.string("relation") == "calls"
                && edge.string("resolution_rule") == "member-binding"
        }),
        "nodes={:#?} edges={:#?}",
        resolved.nodes,
        resolved.edges
    );
    assert!(!resolved.nodes.iter().any(|node| {
        node.string("qualified_name") == "example.com/project/bucket.Stats::Add"
            && node
                .attributes
                .get("external")
                .and_then(serde_json::Value::as_bool)
                == Some(true)
    }));
    Ok(())
}

#[test]
fn imported_go_field_types_resolve_members_to_repository_sources() -> Result<(), Box<dyn Error>> {
    let provider_path = Path::new("internal/common/meta.go");
    let provider_source = br#"package common
type Meta struct{}
func (*Meta) Pgid() uint64 { return 0 }
type Item struct{}
func (*Item) Touch() {}
func (*Meta) Item() *Item { return nil }
"#;
    let unrelated_path = Path::new("vendor/common/meta.go");
    let unrelated_source = br#"package common
type Meta struct{}
func (*Meta) Pgid() uint64 { return 0 }
type Item struct{}
func (*Item) Touch() {}
func (*Meta) Item() *Item { return nil }
"#;
    let caller_path = Path::new("tx.go");
    let caller_source = br#"package storage
import "example.com/project/internal/common"
type Tx struct { meta *common.Meta }
func (tx *Tx) page() uint64 { return tx.meta.Pgid() }
func (tx *Tx) touch() {
    item := tx.meta.Item()
    item.Touch()
}
"#;
    let mut engine = Engine::default();
    let extractions = [
        engine.extract_source(provider_path, provider_source)?,
        engine.extract_source(unrelated_path, unrelated_source)?,
        engine.extract_source(caller_path, caller_source)?,
    ];
    let caller_evidence = extractions[2]
        .semantic_evidence
        .as_ref()
        .ok_or("missing caller evidence")?;
    let call_result = caller_evidence
        .bindings
        .iter()
        .find(|binding| {
            binding.kind == compass_languages::BindingKind::CallResult && binding.spelling == "item"
        })
        .ok_or("missing item call-result binding")?;
    assert_eq!(
        call_result.qualified_target,
        "example.com/project/internal/common.Meta::Item"
    );
    assert!(caller_evidence.candidates.iter().any(|candidate| {
        candidate.target_spelling == "Touch"
            && candidate.binding_id.as_deref() == Some(call_result.id.as_str())
    }));
    let sources = HashMap::from([
        (
            provider_path.to_string_lossy().into_owned(),
            String::from_utf8(provider_source.to_vec())?,
        ),
        (
            unrelated_path.to_string_lossy().into_owned(),
            String::from_utf8(unrelated_source.to_vec())?,
        ),
        (
            caller_path.to_string_lossy().into_owned(),
            String::from_utf8(caller_source.to_vec())?,
        ),
    ]);
    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, Path::new("."));
    let page = resolved
        .nodes
        .iter()
        .find(|node| {
            node.label() == ".page()" && node.string("source_file") == caller_path.to_string_lossy()
        })
        .ok_or("missing page")?;
    let pgid = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "internal/common.Meta::Pgid")
        .ok_or("missing Pgid")?;
    assert!(resolved.edges.iter().any(|edge| {
        edge.source == page.id
            && edge.target == pgid.id
            && edge.string("relation") == "calls"
            && edge.string("source_location") == "L4"
            && edge.string("resolution_rule") == "member-binding"
    }));
    assert!(!resolved.nodes.iter().any(|node| {
        node.string("qualified_name") == "example.com/project/internal/common.Meta::Pgid"
            && node
                .attributes
                .get("external")
                .and_then(serde_json::Value::as_bool)
                == Some(true)
    }));
    let touch = resolved
        .nodes
        .iter()
        .find(|node| {
            node.label() == ".touch()"
                && node.string("source_file") == caller_path.to_string_lossy()
        })
        .ok_or("missing touch")?;
    let item_touch = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "internal/common.Item::Touch")
        .ok_or("missing Item.Touch")?;
    assert!(
        resolved.edges.iter().any(|edge| {
            edge.source == touch.id
                && edge.target == item_touch.id
                && edge.string("relation") == "calls"
                && edge.string("source_location") == "L7"
                && edge.string("resolution_rule") == "member-binding"
        }),
        "touch edges={:#?}; nodes={:#?}",
        resolved
            .edges
            .iter()
            .filter(|edge| edge.source == touch.id)
            .collect::<Vec<_>>(),
        resolved
            .nodes
            .iter()
            .filter(|node| node.label().contains("Touch"))
            .collect::<Vec<_>>()
    );
    Ok(())
}

#[test]
fn root_package_call_results_resolve_members_to_repository_sources() -> Result<(), Box<dyn Error>> {
    let temporary = tempfile::tempdir()?;
    let project_root = temporary.path().join("bbolt");
    std::fs::create_dir(&project_root)?;
    std::fs::write(project_root.join("go.mod"), "module go.etcd.io/bbolt\n")?;
    let provider_path = project_root.join("bucket.go");
    let provider_source = br#"package bbolt
type Bucket struct{}
func (*Bucket) Put() {}
func OpenBucket() *Bucket { return nil }
"#;
    let caller_path = project_root.join("external_test.go");
    let caller_source = br#"package bbolt_test
import bolt "go.etcd.io/bbolt"
func use() {
    bucket := bolt.OpenBucket()
    bucket.Put()
}
"#;
    let foreign_path = project_root.join("foreign_test.go");
    let foreign_source = br#"package bbolt_test
import foreign "other.example/bbolt"
func useForeign() {
    bucket := foreign.OpenBucket()
    bucket.Put()
}
"#;
    let mut engine = Engine::default();
    let extractions = [
        engine
            .extract_source_combined(&provider_path, "bucket.go", provider_source)?
            .graph,
        engine
            .extract_source_combined(&caller_path, "external_test.go", caller_source)?
            .graph,
        engine
            .extract_source_combined(&foreign_path, "foreign_test.go", foreign_source)?
            .graph,
    ];
    let resolved = compass_resolve::resolve_with_root(
        &extractions,
        &HashMap::from([
            (
                "bucket.go".to_owned(),
                String::from_utf8(provider_source.to_vec())?,
            ),
            (
                "external_test.go".to_owned(),
                String::from_utf8(caller_source.to_vec())?,
            ),
            (
                "foreign_test.go".to_owned(),
                String::from_utf8(foreign_source.to_vec())?,
            ),
        ]),
        &project_root,
    );
    let use_function = resolved
        .nodes
        .iter()
        .find(|node| node.label() == "use()")
        .ok_or("missing use")?;
    let put = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "bbolt.Bucket::Put")
        .ok_or("missing Bucket.Put")?;
    assert!(resolved.edges.iter().any(|edge| {
        edge.source == use_function.id
            && edge.target == put.id
            && edge.string("relation") == "calls"
            && edge.string("source_location") == "L5"
            && edge.string("resolution_rule") == "member-binding"
    }));
    let foreign_use = resolved
        .nodes
        .iter()
        .find(|node| node.label() == "useForeign()")
        .ok_or("missing useForeign")?;
    assert!(
        resolved.edges.iter().all(|edge| {
            edge.source != foreign_use.id
                || edge.target != put.id
                || edge.string("relation") != "calls"
        }),
        "foreign edges={:#?}",
        resolved
            .edges
            .iter()
            .filter(|edge| edge.source == foreign_use.id)
            .collect::<Vec<_>>()
    );
    assert!(!resolved.nodes.iter().any(|node| {
        node.string("qualified_name") == "go.etcd.io/bbolt.Bucket::Put"
            && node
                .attributes
                .get("external")
                .and_then(serde_json::Value::as_bool)
                == Some(true)
    }));
    Ok(())
}

#[test]
fn qualified_go_embeddings_bind_packages_without_cross_module_name_joins()
-> Result<(), Box<dyn Error>> {
    let agent_path = Path::new("cmd/agent/agent.go");
    let agent_source = b"package agent\n\ntype Agent interface { Run() }\n";
    let wrapper_path = Path::new("cmd/client/wrapper.go");
    let wrapper_source = br#"package client

import "example.com/project/cmd/agent"

type Wrapper interface {
    agent.Agent
}
"#;
    let context_path = Path::new("internal/contexts/context.go");
    let context_source = b"package contexts\n\ntype Context struct{}\n";
    let caller_path = Path::new("cmd/client/run.go");
    let caller_source = br#"package client

import "context"

func Run(ctx context.Context) {}
"#;
    let external_path = Path::new("cmd/external/wrapper.go");
    let external_source = br#"package external

import "other.example/agent"

type External interface {
    agent.Agent
}
"#;

    let mut engine = Engine::default();
    let extractions = [
        engine.extract_source(agent_path, agent_source)?,
        engine.extract_source(wrapper_path, wrapper_source)?,
        engine.extract_source(context_path, context_source)?,
        engine.extract_source(caller_path, caller_source)?,
        engine.extract_source(external_path, external_source)?,
    ];
    let sources = HashMap::from([
        (
            agent_path.to_string_lossy().into_owned(),
            String::from_utf8(agent_source.to_vec())?,
        ),
        (
            wrapper_path.to_string_lossy().into_owned(),
            String::from_utf8(wrapper_source.to_vec())?,
        ),
        (
            context_path.to_string_lossy().into_owned(),
            String::from_utf8(context_source.to_vec())?,
        ),
        (
            caller_path.to_string_lossy().into_owned(),
            String::from_utf8(caller_source.to_vec())?,
        ),
        (
            external_path.to_string_lossy().into_owned(),
            String::from_utf8(external_source.to_vec())?,
        ),
    ]);
    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, Path::new("."));

    let agent = resolved
        .nodes
        .iter()
        .find(|node| {
            node.label() == "Agent" && node.string("source_file") == agent_path.to_string_lossy()
        })
        .ok_or("missing Agent definition")?;
    let embedding = resolved
        .edges
        .iter()
        .find(|edge| {
            edge.string("relation") == "embeds"
                && edge.string("source_file") == wrapper_path.to_string_lossy()
        })
        .ok_or("missing embedding")?;
    assert_eq!(
        embedding.target, agent.id,
        "nodes={:#?} embedding={embedding:#?}",
        resolved.nodes
    );

    let local_context = resolved
        .nodes
        .iter()
        .find(|node| {
            node.label() == "Context"
                && node.string("source_file") == context_path.to_string_lossy()
        })
        .ok_or("missing repository Context")?;
    let external_context = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "context.Context")
        .ok_or("missing qualified standard-library Context")?;
    assert_eq!(
        external_context
            .attributes
            .get("external")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert!(external_context.string("source_file").is_empty());
    assert_ne!(external_context.id, local_context.id);
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "references" && edge.target == external_context.id
    }));
    assert!(resolved.edges.iter().all(|edge| {
        edge.string("source_file") != caller_path.to_string_lossy()
            || edge.string("relation") != "references"
            || edge.target != local_context.id
    }));
    let external_agent = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "other.example/agent.Agent")
        .ok_or("missing path-qualified external Agent")?;
    assert_ne!(external_agent.id, agent.id);
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("source_file") == external_path.to_string_lossy()
            && edge.string("relation") == "embeds"
            && edge.target == external_agent.id
    }));
    Ok(())
}

#[test]
fn go_local_callback_is_not_exported_for_cross_file_resolution() -> Result<(), Box<dyn Error>> {
    let path = Path::new("internal/pushqueue/pushqueue.go");
    let source = br#"package pushqueue

func acquire() (func(), error) { return func() {}, nil }

func enqueue() {
    release, err := acquire()
    _ = err
    defer release()
}
"#;

    let extracted = Engine::default().extract_source(path, source)?;
    let raw_calls = extracted.raw_calls.as_deref().unwrap_or_default();
    assert!(
        raw_calls.iter().all(|call| call.callee != "release"),
        "lexically bound callbacks must stay within their callable scope: {raw_calls:?}"
    );
    Ok(())
}

#[test]
fn go_local_binding_only_shadows_calls_after_its_declaration() -> Result<(), Box<dyn Error>> {
    let path = Path::new("internal/pushqueue/shadow.go");
    let source = br#"package pushqueue

func release() {}
func enqueue() {
    release()
    release := func() {}
    release()
}
"#;

    let extracted = Engine::default().extract_source(path, source)?;
    let sources = HashMap::from([(
        path.to_string_lossy().into_owned(),
        String::from_utf8(source.to_vec())?,
    )]);
    let resolved = compass_resolve::resolve(&[extracted], &sources);
    let release_calls = resolved
        .edges
        .iter()
        .filter(|edge| {
            edge.string("relation") == "calls"
                && resolved.nodes.iter().any(|node| {
                    node.id == edge.target
                        && node.label() == "release()"
                        && node.string("source_file") == path.to_string_lossy()
                })
        })
        .collect::<Vec<_>>();

    assert_eq!(
        release_calls.len(),
        1,
        "only the call before the short declaration targets the package function: {:?}",
        resolved.edges
    );
    assert_eq!(release_calls[0].string("source_location"), "L5");
    Ok(())
}

#[test]
fn generic_call_resolution_never_targets_file_nodes() -> Result<(), Box<dyn Error>> {
    let caller_path = Path::new("internal/pushqueue/pushqueue.go");
    let caller_source = b"package pushqueue\n\nfunc enqueue() { release() }\n";
    let mut extracted = Engine::default().extract_source(caller_path, caller_source)?;
    extracted.nodes.push(RawNodeRecord {
        id: "mise_tasks_release_file".to_owned(),
        attributes: Map::from_iter([
            ("label".to_owned(), Value::String("release".to_owned())),
            (
                "source_file".to_owned(),
                Value::String("mise-tasks/release".to_owned()),
            ),
            ("file_type".to_owned(), Value::String("code".to_owned())),
            ("symbol_kind".to_owned(), Value::String("file".to_owned())),
        ]),
    });
    let sources = HashMap::from([(
        caller_path.to_string_lossy().into_owned(),
        String::from_utf8(caller_source.to_vec())?,
    )]);

    let resolved = compass_resolve::resolve_with_root(&[extracted], &sources, Path::new("."));
    assert!(
        resolved.edges.iter().all(|edge| {
            edge.string("relation") != "calls" || edge.target != "mise_tasks_release_file"
        }),
        "file nodes must never be selected as callable targets: {:?}",
        resolved.edges
    );
    Ok(())
}
