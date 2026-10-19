use std::collections::HashMap;
use std::error::Error;

use compass_graph::{BuildEvidence, normalize_v1};
use compass_languages::Engine;
use compass_model::code_graph::{EdgeKind, GraphDocument};
use compass_model::provenance::EvidenceConfidence;

fn graph(source: &str) -> Result<GraphDocument, Box<dyn Error>> {
    graph_sources(&[("main.ts", source)])
}

fn graph_sources(inputs: &[(&str, &str)]) -> Result<GraphDocument, Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let mut sources = HashMap::new();
    let mut extractions = Vec::new();
    for (file, source) in inputs {
        let path = root.path().join(file);
        std::fs::write(&path, source)?;
        let extraction =
            Engine::default().extract_source_graph_only(&path, file, source.as_bytes())?;
        assert!(extraction.error.is_none(), "{:?}", extraction.error);
        sources.insert(path.to_string_lossy().into_owned(), (*source).to_owned());
        extractions.push(extraction);
    }
    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root.path());
    assert!(resolved.error.is_none(), "{:?}", resolved.error);
    let evidence =
        BuildEvidence::from_extraction(root.path(), &resolved, "sha256:type-origin-control")?;
    let graph = normalize_v1(resolved, evidence)?;
    compass_model::validate_code_graph(&graph)?;
    Ok(graph)
}

fn assert_lookup(source: &str, caller: &str, target: &str) -> Result<(), Box<dyn Error>> {
    let graph = graph(source)?;
    assert_lookup_graph(&graph, source, caller, target)
}

fn assert_lookup_graph(
    graph: &GraphDocument,
    source: &str,
    caller: &str,
    target: &str,
) -> Result<(), Box<dyn Error>> {
    assert_lookup_graph_at(
        graph,
        source,
        caller,
        target,
        source
            .rfind(".lookup(")
            .map(|start| start + 1)
            .ok_or("missing lookup call")?,
    )
}

fn assert_lookup_graph_at(
    graph: &GraphDocument,
    source: &str,
    caller: &str,
    target: &str,
    expected_start: usize,
) -> Result<(), Box<dyn Error>> {
    let owner = graph
        .nodes
        .iter()
        .find(|node| node.qualified_name == caller)
        .ok_or("missing caller")?;
    let target = graph
        .nodes
        .iter()
        .find(|node| node.qualified_name == target)
        .ok_or("missing source target")?;
    assert!(target.source.is_some());
    let calls = graph
        .links
        .iter()
        .filter(|edge| edge.source == owner.id && edge.kind == EdgeKind::Calls)
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 1, "outgoing calls from {caller}: {calls:#?}");
    assert_eq!(
        calls[0].target, target.id,
        "annotation or substituted argument rebound at use site"
    );
    let site = calls[0]
        .relationship_site
        .as_ref()
        .ok_or("missing call occurrence")?;
    assert_eq!(site.file, "main.ts");
    assert_eq!(usize::try_from(site.start_byte)?, expected_start);
    assert_eq!(
        usize::try_from(site.end_byte)?,
        expected_start + "lookup".len()
    );
    assert_eq!(
        source.get(usize::try_from(site.start_byte)?..usize::try_from(site.end_byte)?),
        Some("lookup")
    );
    assert!(
        calls[0]
            .evidence
            .iter()
            .any(|evidence| evidence.confidence == EvidenceConfidence::Exact
                && evidence.anchors.contains(site))
    );
    Ok(())
}

#[test]
fn field_annotation_keeps_its_declaration_scope() -> Result<(), Box<dyn Error>> {
    assert_lookup(
        r#"
class Table { lookup(value: string) { return value.length; } }
class Owner {
    private table: Table = new Table();
    read(value: string) {
        class Table { lookup(value: string) { return 99; } }
        return this.table.lookup(value);
    }
}
"#,
        "main.Owner.read",
        "main.Table.lookup",
    )
}

#[test]
fn field_annotation_alias_follows_the_alias_definition_scope() -> Result<(), Box<dyn Error>> {
    assert_lookup(
        r#"
function outer() {
    class Table { lookup(value: string) { return value.length; } }
    type Selection = Table;
    function inner() {
        class Table { lookup(value: string) { return 99; } }
        class Owner {
            private table: Selection;
            read(value: string) { return this.table.lookup(value); }
        }
    }
}
"#,
        "main.outer.inner.Owner.read",
        "main.outer.Table.lookup",
    )
}

#[test]
fn field_annotation_constraint_keeps_its_parameter_scope() -> Result<(), Box<dyn Error>> {
    assert_lookup(
        r#"
class Table { lookup(value: string) { return value.length; } }
class Owner<T extends Table> {
    private table: T;
    read(value: string) {
        class Table { lookup(value: string) { return 99; } }
        return this.table.lookup(value);
    }
}
"#,
        "main.Owner.read",
        "main.Table.lookup",
    )
}

#[test]
fn field_generic_argument_keeps_its_instantiation_scope() -> Result<(), Box<dyn Error>> {
    assert_lookup(
        r#"
class Table { lookup(value: string) { return value.length; } }
class Holder<T> { field: T; }
class Owner {
    private holder: Holder<Table>;
    read(value: string) {
        class Table { lookup(value: string) { return 99; } }
        return this.holder.field.lookup(value);
    }
}
"#,
        "main.Owner.read",
        "main.Table.lookup",
    )
}

#[test]
fn field_alias_generic_argument_keeps_its_instantiation_scope() -> Result<(), Box<dyn Error>> {
    assert_lookup(
        r#"
class Table { lookup(value: string) { return value.length; } }
class Holder<T> { field: T; }
type Selection<T> = Holder<T>;
class Owner {
    private holder: Selection<Table>;
    read(value: string) {
        class Table { lookup(value: string) { return 99; } }
        return this.holder.field.lookup(value);
    }
}
"#,
        "main.Owner.read",
        "main.Table.lookup",
    )
}

#[test]
fn inherited_field_argument_keeps_the_derived_class_scope() -> Result<(), Box<dyn Error>> {
    assert_lookup(
        r#"
class Table { lookup(value: string) { return 99; } }
class Holder<T> { field: T; }
function outer() {
    class Table { lookup(value: string) { return value.length; } }
    class Owner extends Holder<Table> {
        read(value: string) { return this.field.lookup(value); }
    }
}
"#,
        "main.outer.Owner.read",
        "main.outer.Table.lookup",
    )
}

#[test]
fn field_actual_parameter_keeps_the_callers_constraint() -> Result<(), Box<dyn Error>> {
    assert_lookup(
        r#"
class Table { lookup(value: string) { return value.length; } }
class Holder<T> { field: T; }
class Owner<T extends Table> {
    private holder: Holder<T>;
    read(value: string) { return this.holder.field.lookup(value); }
}
"#,
        "main.Owner.read",
        "main.Table.lookup",
    )
}

#[test]
fn field_alias_actual_parameter_keeps_the_callers_constraint() -> Result<(), Box<dyn Error>> {
    assert_lookup(
        r#"
class Table { lookup(value: string) { return value.length; } }
class Holder<T> { field: T; }
type Selection<T> = Holder<T>;
class Owner<T extends Table> {
    private holder: Selection<T>;
    read(value: string) { return this.holder.field.lookup(value); }
}
"#,
        "main.Owner.read",
        "main.Table.lookup",
    )
}

#[test]
fn unknown_actual_arguments_and_ambiguous_or_cyclic_aliases_never_borrow_a_target()
-> Result<(), Box<dyn Error>> {
    for source in [
        "class Wrong { lookup() {} } class Holder<T extends Wrong> { field: T; } class Owner<T> { holder: Holder<T>; read() { this.holder.field.lookup(); } }",
        "class Good { lookup() {} } class Other { lookup() {} } type Selected = Good; type Selected = Other; class Owner { field: Selected; read() { this.field.lookup(); } }",
        "type First = Second; type Second = First; class Owner { field: First; read() { this.field.lookup(); } }",
    ] {
        let graph = graph(source)?;
        let owner = graph
            .nodes
            .iter()
            .find(|node| node.qualified_name == "main.Owner.read")
            .ok_or("missing caller")?;
        assert!(
            !graph.links.iter().any(|edge| edge.source == owner.id
                && edge.kind == EdgeKind::Calls
                && graph
                    .nodes
                    .iter()
                    .any(|node| node.id == edge.target && node.source.is_some())),
            "unsupported source origin acquired a source target: {source}"
        );
    }
    Ok(())
}

#[test]
fn generic_array_elements_keep_the_actual_parameters_origin() -> Result<(), Box<dyn Error>> {
    assert_lookup(
        r#"
class Table { lookup(value: string) { return value.length; } }
class Holder<T> { fields: T[]; }
class Owner<T extends Table> {
    holder: Holder<T>;
    read(value: string) {
        class Table { lookup(value: string) { return 99; } }
        return this.holder.fields[0].lookup(value);
    }
}
"#,
        "main.Owner.read",
        "main.Table.lookup",
    )
}

#[test]
fn generic_projection_literal_keys_keep_their_actual_argument() -> Result<(), Box<dyn Error>> {
    assert_lookup(
        r#"
class Table { lookup(value: string) { return value.length; } }
type Selection<T, K extends keyof T> = Pick<T, K>;
class Owner {
    field: Selection<Table, "lookup">;
    read(value: string) { return this.field.lookup(value); }
}
"#,
        "main.Owner.read",
        "main.Table.lookup",
    )
}

#[test]
fn inline_index_annotations_reject_body_local_namesakes() -> Result<(), Box<dyn Error>> {
    assert_lookup(
        r#"
class Good { lookup() {} }
function use(dict: { [key: string]: Good }, key: string) {
    class Good { lookup() {} }
    return dict[key].lookup();
}
"#,
        "main.use",
        "main.Good.lookup",
    )
}

#[test]
fn nominal_index_arguments_do_not_bind_the_callers_namesake_parameter() -> Result<(), Box<dyn Error>>
{
    for source in [
        "class Good { lookup() {} } class Wrong { lookup() {} } interface Dict<E> { [key: string]: E } class Owner<E extends Wrong> { dict: Dict<Good>; read(key: string) { return this.dict[key].lookup(); } }",
        "class Owner<E extends Wrong> { dict: Dict<Good>; read(key: string) { return this.dict[key].lookup(); } } interface Dict<E> { [key: string]: E } class Wrong { lookup() {} } class Good { lookup() {} }",
    ] {
        assert_lookup(source, "main.Owner.read", "main.Good.lookup")?;
    }
    Ok(())
}

#[test]
fn cross_file_unbounded_arguments_never_borrow_provider_namesakes() -> Result<(), Box<dyn Error>> {
    let provider = "export class T { lookup() {} } export interface Holder<T> { field: T }";
    let consumer = "import type { Holder } from './provider'; class Owner<T> { holder: Holder<T>; read() { return this.holder.field.lookup(); } }";
    for inputs in [
        [("provider.ts", provider), ("main.ts", consumer)],
        [("main.ts", consumer), ("provider.ts", provider)],
    ] {
        let graph = graph_sources(&inputs)?;
        let owner = graph
            .nodes
            .iter()
            .find(|node| node.qualified_name == "main.Owner.read")
            .ok_or("missing caller")?;
        assert!(
            !graph.links.iter().any(|edge| edge.source == owner.id
                && edge.kind == EdgeKind::Calls
                && graph
                    .nodes
                    .iter()
                    .any(|node| node.id == edge.target && node.source.is_some())),
            "unbounded caller argument borrowed a provider target"
        );
    }
    Ok(())
}

#[test]
fn cross_file_unknown_arguments_keep_direct_member_identity_without_return_inference()
-> Result<(), Box<dyn Error>> {
    let provider = "export class T { lookup() {} } export interface Holder<T> { lookup(): T }";
    let consumer = "import type { Holder } from './provider'; class Owner<T> { holder: Holder<T>; read() { return this.holder.lookup().lookup(); } }";
    let direct_start = consumer
        .find("lookup().lookup")
        .ok_or("missing direct lookup")?;
    let dependent_start = consumer.rfind("lookup").ok_or("missing dependent lookup")?;
    assert_ne!(direct_start, dependent_start);
    for inputs in [
        [("provider.ts", provider), ("main.ts", consumer)],
        [("main.ts", consumer), ("provider.ts", provider)],
    ] {
        let graph = graph_sources(&inputs)?;
        assert_lookup_graph_at(
            &graph,
            consumer,
            "main.Owner.read",
            "provider.Holder.lookup",
            direct_start,
        )?;
        let owner = graph
            .nodes
            .iter()
            .find(|node| node.qualified_name == "main.Owner.read")
            .ok_or("missing caller")?;
        assert!(
            !graph.links.iter().any(|edge| edge.source == owner.id
                && edge.kind == EdgeKind::Calls
                && edge
                    .relationship_site
                    .as_ref()
                    .is_some_and(|site| site.start_byte
                        == u64::try_from(dependent_start).unwrap_or(u64::MAX))
                && graph
                    .nodes
                    .iter()
                    .any(|node| node.id == edge.target && node.source.is_some())),
            "unknown return argument supplied a source-backed dependent call"
        );
    }
    Ok(())
}

#[test]
fn cross_file_canonical_arguments_keep_the_callers_source_type() -> Result<(), Box<dyn Error>> {
    let provider = "export class T { lookup() {} } export interface Holder<T> { field: T }";
    let consumer = "import type { Holder } from './provider'; class Good { lookup() {} } class Owner { holder: Holder<Good>; read() { return this.holder.field.lookup(); } }";
    for inputs in [
        [("provider.ts", provider), ("main.ts", consumer)],
        [("main.ts", consumer), ("provider.ts", provider)],
    ] {
        let graph = graph_sources(&inputs)?;
        assert_lookup_graph(&graph, consumer, "main.Owner.read", "main.Good.lookup")?;
    }
    Ok(())
}

#[test]
fn missing_imported_arguments_do_not_hide_independent_direct_members() -> Result<(), Box<dyn Error>>
{
    let provider = "export interface Holder<T> { lookup(): T }";
    let consumer = "import type { Holder } from './provider'; import type { Remote } from './missing'; class Owner { holder: Holder<Remote>; read() { return this.holder.lookup(); } }";
    for inputs in [
        [("provider.ts", provider), ("main.ts", consumer)],
        [("main.ts", consumer), ("provider.ts", provider)],
    ] {
        let graph = graph_sources(&inputs)?;
        assert_lookup_graph(
            &graph,
            consumer,
            "main.Owner.read",
            "provider.Holder.lookup",
        )?;
    }
    Ok(())
}

#[test]
fn unknown_arguments_do_not_hide_independent_provider_field_types() -> Result<(), Box<dyn Error>> {
    let provider = "class Good { lookup() {} } export interface Holder<T> { fixed: Good }";
    let consumer = "import type { Holder } from './provider'; import type { Remote } from './missing'; class Owner { holder: Holder<Remote>; read() { return this.holder.fixed.lookup(); } }";
    for inputs in [
        [("provider.ts", provider), ("main.ts", consumer)],
        [("main.ts", consumer), ("provider.ts", provider)],
    ] {
        let graph = graph_sources(&inputs)?;
        assert_lookup_graph(&graph, consumer, "main.Owner.read", "provider.Good.lookup")?;
    }
    Ok(())
}

#[test]
fn provider_type_aliases_can_shadow_intrinsic_utility_names() -> Result<(), Box<dyn Error>> {
    let provider = "interface Readonly<T> { lookup(): void } export interface Holder<T> { field: Readonly<T> }";
    let consumer = "import type { Holder } from './provider'; class Good { lookup() {} } class Owner { holder: Holder<Good>; read() { return this.holder.field.lookup(); } }";
    for inputs in [
        [("provider.ts", provider), ("main.ts", consumer)],
        [("main.ts", consumer), ("provider.ts", provider)],
    ] {
        let graph = graph_sources(&inputs)?;
        assert_lookup_graph(
            &graph,
            consumer,
            "main.Owner.read",
            "provider.Readonly.lookup",
        )?;
    }
    Ok(())
}

#[test]
fn method_type_parameters_shadow_class_parameters_by_identity() -> Result<(), Box<dyn Error>> {
    let provider = "export class T { lookup() {} } export interface Holder<T> { lookup<T>(): T }";
    let consumer = "import type { Holder } from './provider'; class Good { lookup() {} } class Owner { holder: Holder<Good>; read() { return this.holder.lookup().lookup(); } }";
    let direct = consumer
        .find("lookup().lookup")
        .ok_or("missing direct call")?;
    for inputs in [
        [("provider.ts", provider), ("main.ts", consumer)],
        [("main.ts", consumer), ("provider.ts", provider)],
    ] {
        let graph = graph_sources(&inputs)?;
        assert_lookup_graph_at(
            &graph,
            consumer,
            "main.Owner.read",
            "provider.Holder.lookup",
            direct,
        )?;
    }
    Ok(())
}

#[test]
fn imported_actual_arguments_follow_exact_reexports() -> Result<(), Box<dyn Error>> {
    let provider = "export interface Holder<T> { field: T }";
    let types = "export class Good { lookup() {} }";
    let barrel = "export { Good } from './types';";
    let consumer = "import type { Holder } from './provider'; import type { Good } from './barrel'; class Owner { holder: Holder<Good>; read() { this.holder.field.lookup(); } }";
    for inputs in [
        [
            ("provider.ts", provider),
            ("types.ts", types),
            ("barrel.ts", barrel),
            ("main.ts", consumer),
        ],
        [
            ("main.ts", consumer),
            ("barrel.ts", barrel),
            ("types.ts", types),
            ("provider.ts", provider),
        ],
    ] {
        let graph = graph_sources(&inputs)?;
        assert_lookup_graph(&graph, consumer, "main.Owner.read", "types.Good.lookup")?;
    }
    Ok(())
}

#[test]
fn actual_export_proofs_preserve_independent_fields_and_publication_anchors()
-> Result<(), Box<dyn Error>> {
    let provider =
        "class Stable { lookup() {} } export interface Holder<T> { field: T; fixed: Stable }";
    let types = "export class Remote { lookup() {} }";
    for (barrel, complete) in [
        ("export { Remote } from './types';", true),
        (
            "export * from './types'; export * from './unavailable';",
            false,
        ),
        (
            "export { Remote } from './unavailable'; export * from './types';",
            false,
        ),
    ] {
        for member in ["field", "fixed"] {
            let consumer = format!(
                "import type {{ Holder }} from './provider'; import type {{ Remote }} from './barrel'; class Owner {{ holder: Holder<Remote>; read() {{ this.holder.{member}.lookup(); }} }}"
            );
            let inputs = [
                ("provider.ts", provider),
                ("types.ts", types),
                ("barrel.ts", barrel),
                ("main.ts", consumer.as_str()),
            ];
            for inputs in [inputs.to_vec(), inputs.into_iter().rev().collect()] {
                let graph = graph_sources(&inputs)?;
                if member == "fixed" {
                    assert_lookup_graph(
                        &graph,
                        &consumer,
                        "main.Owner.read",
                        "provider.Stable.lookup",
                    )?;
                } else if complete {
                    assert_lookup_graph(
                        &graph,
                        &consumer,
                        "main.Owner.read",
                        "types.Remote.lookup",
                    )?;
                } else {
                    assert_no_source_lookup_at(&graph, &consumer, "main.Owner.read")?;
                }
            }
        }
    }
    Ok(())
}

fn assert_no_source_lookup_at(
    graph: &GraphDocument,
    source: &str,
    caller: &str,
) -> Result<(), Box<dyn Error>> {
    let owner = graph
        .nodes
        .iter()
        .find(|node| node.qualified_name == caller)
        .ok_or("missing caller")?;
    let start = source.rfind(".lookup(").ok_or("missing lookup call")? + 1;
    let start = u64::try_from(start)?;
    assert!(
        !graph.links.iter().any(|edge| edge.source == owner.id
            && edge.kind == EdgeKind::Calls
            && edge.relationship_site.as_ref().is_some_and(|site| {
                site.file == "main.ts" && site.start_byte == start && site.end_byte == start + 6
            })
            && graph
                .nodes
                .iter()
                .any(|node| node.id == edge.target && node.source.is_some())),
        "unsupported type or export proof invented a source lookup at {start}"
    );
    Ok(())
}

#[test]
fn nested_awaited_does_not_erase_provider_defined_promise_wrappers() -> Result<(), Box<dyn Error>> {
    let consumer = "import type { Holder } from './provider'; class Good { lookup() {} } class Owner { holder: Holder<Good>; read() { return this.holder.field.lookup(); } }";
    for (shadow, intrinsic) in [("PromiseLike", "Promise"), ("Promise", "PromiseLike")] {
        let provider = format!(
            "interface {shadow}<T> {{ different: T }} export interface Holder<T> {{ field: Awaited<{intrinsic}<{shadow}<T>>> }}"
        );
        for inputs in [
            [("provider.ts", provider.as_str()), ("main.ts", consumer)],
            [("main.ts", consumer), ("provider.ts", provider.as_str())],
        ] {
            let graph = graph_sources(&inputs)?;
            assert_no_source_lookup_at(&graph, consumer, "main.Owner.read")?;
        }
    }
    Ok(())
}

#[test]
fn nested_awaited_does_not_erase_imported_promise_wrappers() -> Result<(), Box<dyn Error>> {
    let consumer = "import type { Holder } from './provider'; class Good { lookup() {} } class Owner { holder: Holder<Good>; read() { return this.holder.field.lookup(); } }";
    for (shadow, intrinsic) in [("PromiseLike", "Promise"), ("Promise", "PromiseLike")] {
        let types = "export interface Custom<T> { different: T }";
        let provider = format!(
            "import type {{ Custom as {shadow} }} from './types'; export interface Holder<T> {{ field: Awaited<{intrinsic}<{shadow}<T>>> }}"
        );
        for inputs in [
            [
                ("provider.ts", provider.as_str()),
                ("types.ts", types),
                ("main.ts", consumer),
            ],
            [
                ("main.ts", consumer),
                ("types.ts", types),
                ("provider.ts", provider.as_str()),
            ],
        ] {
            let graph = graph_sources(&inputs)?;
            assert_no_source_lookup_at(&graph, consumer, "main.Owner.read")?;
        }
    }
    Ok(())
}

#[test]
fn nested_awaited_intrinsics_keep_the_actual_source_target() -> Result<(), Box<dyn Error>> {
    let consumer = "import type { Holder } from './provider'; class Good { lookup() {} } class Owner { holder: Holder<Good>; read() { return this.holder.field.lookup(); } }";
    for expression in [
        "Awaited<T>",
        "Awaited<Promise<PromiseLike<T>>>",
        "Awaited<PromiseLike<Promise<T>>>",
    ] {
        let provider = format!("export interface Holder<T> {{ field: {expression} }}");
        for inputs in [
            [("provider.ts", provider.as_str()), ("main.ts", consumer)],
            [("main.ts", consumer), ("provider.ts", provider.as_str())],
        ] {
            let graph = graph_sources(&inputs)?;
            assert_lookup_graph(&graph, consumer, "main.Owner.read", "main.Good.lookup")?;
        }
    }
    Ok(())
}

#[test]
fn direct_shadowed_promise_wrappers_keep_their_own_members() -> Result<(), Box<dyn Error>> {
    let consumer = "import type { Holder } from './provider'; class Good { lookup() {} } class Owner { holder: Holder<Good>; read() { return this.holder.field.lookup(); } }";
    for shadow in ["Promise", "PromiseLike"] {
        let provider = format!(
            "interface {shadow}<T> {{ lookup(): void }} export interface Holder<T> {{ field: {shadow}<T> }}"
        );
        for inputs in [
            [("provider.ts", provider.as_str()), ("main.ts", consumer)],
            [("main.ts", consumer), ("provider.ts", provider.as_str())],
        ] {
            let graph = graph_sources(&inputs)?;
            assert_lookup_graph(
                &graph,
                consumer,
                "main.Owner.read",
                &format!("provider.{shadow}.lookup"),
            )?;
        }
    }
    Ok(())
}

#[test]
fn awaited_actual_thenables_do_not_keep_the_wrappers_lookup() -> Result<(), Box<dyn Error>> {
    let consumer = "import type { Holder } from './provider'; class Leaf { lookup() {} } class Good { lookup() {} then(onfulfilled: (value: Leaf) => unknown): unknown { return onfulfilled(new Leaf()); } } class Owner { holder: Holder<Good>; read() { return this.holder.field.lookup(); } }";
    let expected_start = u64::try_from(consumer.rfind(".lookup(").ok_or("missing call")? + 1)?;
    for expression in [
        "Awaited<T>",
        "Awaited<Promise<T>>",
        "Awaited<Promise<PromiseLike<T>>>",
    ] {
        let provider = format!("export interface Holder<T> {{ field: {expression} }}");
        for inputs in [
            [("provider.ts", provider.as_str()), ("main.ts", consumer)],
            [("main.ts", consumer), ("provider.ts", provider.as_str())],
        ] {
            let graph = graph_sources(&inputs)?;
            let owner = graph
                .nodes
                .iter()
                .find(|node| node.qualified_name == "main.Owner.read")
                .ok_or("missing owner")?;
            let wrong = graph
                .nodes
                .iter()
                .find(|node| node.qualified_name == "main.Good.lookup")
                .ok_or("missing wrapper method")?;
            assert!(
                !graph.links.iter().any(|edge| edge.kind == EdgeKind::Calls
                    && edge.source == owner.id
                    && edge.target == wrong.id
                    && edge
                        .relationship_site
                        .as_ref()
                        .is_some_and(|site| site.file == "main.ts"
                            && site.start_byte == expected_start
                            && site.end_byte == expected_start + 6)),
                "Awaited selected a custom thenable's own lookup for {expression}"
            );
        }
    }
    Ok(())
}

#[test]
fn awaited_alias_actuals_keep_a_certified_private_class() -> Result<(), Box<dyn Error>> {
    let provider = "export interface Holder<T> { field: Awaited<Promise<T>> }";
    let consumer = "import type { Holder } from './provider'; class Good { lookup() {} } type Actual = Good; class Owner { holder: Holder<Actual>; read() { this.holder.field.lookup(); } }";
    for inputs in [
        [("provider.ts", provider), ("main.ts", consumer)],
        [("main.ts", consumer), ("provider.ts", provider)],
    ] {
        let graph = graph_sources(&inputs)?;
        assert_lookup_graph(&graph, consumer, "main.Owner.read", "main.Good.lookup")?;
    }
    Ok(())
}

#[test]
fn awaited_incomplete_or_merged_actuals_cannot_prove_terminal_identity()
-> Result<(), Box<dyn Error>> {
    let provider = "export interface Holder<T> { field: Awaited<Promise<T>> }";
    for actual in [
        "class Base { then(done: (value: string) => unknown) {} } class Good extends Base { lookup() {} }",
        "class Good { lookup() {} } interface Good { then(done: (value: string) => unknown): void; }",
        "class Good { ['then'](done: (value: string) => unknown) {} lookup() {} }",
        "class Good { constructor(public then: (done: () => void) => void) {} lookup() {} }",
    ] {
        let consumer = format!(
            "import type {{ Holder }} from './provider'; {actual} class Owner {{ holder: Holder<Good>; read() {{ this.holder.field.lookup(); }} }}"
        );
        for inputs in [
            [("provider.ts", provider), ("main.ts", consumer.as_str())],
            [("main.ts", consumer.as_str()), ("provider.ts", provider)],
        ] {
            let graph = graph_sources(&inputs)?;
            assert_no_source_lookup_at(&graph, &consumer, "main.Owner.read")?;
        }
    }
    Ok(())
}

#[test]
fn awaited_custom_type_aliases_and_imports_cannot_supply_a_nominal_target()
-> Result<(), Box<dyn Error>> {
    let custom = "class Actual<T> { then(done: (value: T) => unknown): void {} lookup() {} }";
    let types = "export class Actual<T> { then(done: (value: T) => unknown): void {} lookup() {} }";
    let consumer = "import type { Holder } from './provider'; class Good { lookup() {} } class Owner { holder: Holder<Good>; read() { this.holder.field.lookup(); } }";
    for provider in [
        format!("{custom} export interface Holder<T> {{ field: Awaited<Actual<T>> }}"),
        format!("{custom} type Alias<T> = Actual<T>; export interface Holder<T> {{ field: Awaited<Promise<Alias<T>>> }}"),
        "import type { Actual } from './types'; export interface Holder<T> { field: Awaited<Promise<Actual<T>>> }".to_owned(),
        "import type { Missing } from './missing'; export interface Holder<T> { field: Awaited<Missing<T>> }".to_owned(),
    ] {
        for inputs in [
            [("provider.ts", provider.as_str()), ("types.ts", types), ("main.ts", consumer)],
            [("main.ts", consumer), ("types.ts", types), ("provider.ts", provider.as_str())],
        ] {
            let graph = graph_sources(&inputs)?;
            assert_no_source_lookup_at(&graph, consumer, "main.Owner.read")?;
        }
    }
    Ok(())
}

#[test]
fn awaited_exported_nominals_require_complete_project_merge_inventory() -> Result<(), Box<dyn Error>>
{
    let provider = "export interface Holder<T> { field: Awaited<Promise<T>> }";
    let consumer = "import type { Holder } from './provider'; import type { Good } from './types'; class Owner { holder: Holder<Good>; read() { this.holder.field.lookup(); } }";
    for types in [
        "export class Good { lookup() {} }",
        "export interface Good { lookup(): void }",
    ] {
        for inputs in [
            [
                ("provider.ts", provider),
                ("types.ts", types),
                ("main.ts", consumer),
            ],
            [
                ("main.ts", consumer),
                ("types.ts", types),
                ("provider.ts", provider),
            ],
        ] {
            let graph = graph_sources(&inputs)?;
            assert_lookup_graph(&graph, consumer, "main.Owner.read", "types.Good.lookup")?;
        }
        let augmentation = "import './types'; declare module './types' { interface Good { then(done: (value: string) => unknown): void } }";
        for inputs in [
            [
                ("provider.ts", provider),
                ("types.ts", types),
                ("augment.ts", augmentation),
                ("main.ts", consumer),
            ],
            [
                ("main.ts", consumer),
                ("augment.ts", augmentation),
                ("types.ts", types),
                ("provider.ts", provider),
            ],
        ] {
            let graph = graph_sources(&inputs)?;
            assert_no_source_lookup_at(&graph, consumer, "main.Owner.read")?;
        }
    }
    Ok(())
}
