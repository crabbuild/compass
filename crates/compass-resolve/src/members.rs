use std::collections::{HashMap, HashSet, VecDeque};

use compass_languages::{Extraction, RawCall};
use compass_model::{EdgeRecord, NodeRecord};
use serde_json::{Map, Value};

/// Apply the deterministic language-specific member-call passes to merged facts.
pub fn resolve_language_calls(extractions: &[Extraction], merged: &mut Extraction) {
    resolve_language_call_facts(collect_language_call_facts(extractions), merged);
}

pub(crate) struct LanguageCallFacts {
    tables: TypeTables,
    pub(crate) calls: Vec<RawCall>,
}

pub(crate) fn collect_language_call_facts(extractions: &[Extraction]) -> LanguageCallFacts {
    LanguageCallFacts {
        tables: TypeTables::new(extractions),
        calls: extractions
            .iter()
            .flat_map(|extraction| extraction.raw_calls.iter().flatten().cloned())
            .collect(),
    }
}

pub(crate) fn collect_language_call_facts_owned(
    extractions: &mut [Extraction],
) -> LanguageCallFacts {
    let tables = TypeTables::new(extractions);
    let calls = extractions
        .iter_mut()
        .flat_map(|extraction| extraction.raw_calls.take().into_iter().flatten())
        .collect();
    LanguageCallFacts { tables, calls }
}

pub(crate) fn resolve_language_call_facts(facts: LanguageCallFacts, merged: &mut Extraction) {
    let indexes = Indexes::new(&merged.nodes, &merged.edges);
    let mut existing = merged
        .edges
        .iter()
        .map(|edge| (edge.source.clone(), edge.target.clone()))
        .collect::<HashSet<_>>();

    resolve_swift_registry_compatibility(
        &facts.calls,
        &indexes,
        &facts.tables,
        &mut existing,
        &mut merged.edges,
    );
    resolve_typed_members(
        &facts.calls,
        &indexes,
        &facts.tables,
        &mut existing,
        &mut merged.edges,
    );
    resolve_python_members(&facts.calls, &indexes, &mut existing, &mut merged.edges);
    resolve_ruby_members(&facts.calls, &indexes, &mut existing, &mut merged.edges);
    resolve_pascal_inherited(&facts.calls, &indexes, &mut existing, &mut merged.edges);
}

/// Preserve Graphify's resolver-registry ordering for strict external parity.
///
/// Once a corpus contains Swift type facts, the Python implementation's Swift
/// pass sees the collection-wide raw-call list. Consequently, an explicitly
/// capitalized receiver from another language is still resolved as a unique
/// type reference before later language passes run. This is observable graph
/// output, so Compass deliberately retains it as a compatibility rule.
fn resolve_swift_registry_compatibility(
    calls: &[RawCall],
    indexes: &Indexes,
    tables: &TypeTables,
    existing: &mut HashSet<(String, String)>,
    edges: &mut Vec<EdgeRecord>,
) {
    if tables.swift.is_empty() {
        return;
    }
    for call in calls {
        if call.is_member_call != Some(true)
            || member_family(&call.source_file, call.lang.as_deref()) == MemberFamily::Swift
        {
            continue;
        }
        let Some(receiver) = receiver(call) else {
            continue;
        };
        let Some(owner) = starts_upper(receiver)
            .then(|| indexes.unique_type(receiver))
            .flatten()
        else {
            continue;
        };
        let (target, relation_name) = indexes
            .unique_method(owner, &call.callee)
            .map_or((owner, "references"), |method| (method, "calls"));
        emit(
            call,
            target,
            relation_name,
            "call",
            ("EXTRACTED", 1.0),
            existing,
            edges,
        );
    }
}

struct Indexes {
    nodes: HashMap<String, NodeRecord>,
    types: HashMap<String, Vec<String>>,
    methods: HashMap<(String, String), Vec<String>>,
    defines: HashMap<(String, String), Vec<String>>,
    enclosing_method: HashMap<String, String>,
    enclosing_member: HashMap<String, String>,
    contains: HashMap<String, HashMap<String, Vec<String>>>,
    file_of: HashMap<String, String>,
    imports: HashMap<String, HashSet<String>>,
    bases: HashMap<String, Vec<String>>,
}

impl Indexes {
    fn new(nodes: &[NodeRecord], edges: &[EdgeRecord]) -> Self {
        let nodes_by_id = nodes
            .iter()
            .map(|node| (node.id.clone(), node.clone()))
            .collect::<HashMap<_, _>>();
        let contained = edges
            .iter()
            .filter(|edge| relation(edge) == "contains")
            .map(|edge| edge.target.as_str())
            .collect::<HashSet<_>>();
        let mut types = HashMap::<String, Vec<String>>::new();
        for node in nodes {
            if contained.contains(node.id.as_str()) && is_type_like(node) {
                push_unique(&mut types, key(node.label()), &node.id);
            }
        }
        let mut methods = HashMap::<(String, String), Vec<String>>::new();
        let mut defines = HashMap::<(String, String), Vec<String>>::new();
        let mut enclosing_method = HashMap::new();
        let mut enclosing_member = HashMap::new();
        let mut contains = HashMap::<String, HashMap<String, Vec<String>>>::new();
        let mut file_of = HashMap::new();
        let mut imports = HashMap::<String, HashSet<String>>::new();
        let mut bases = HashMap::<String, Vec<String>>::new();
        for edge in edges {
            let target_label = nodes_by_id.get(&edge.target).map(NodeRecord::label);
            match relation(edge) {
                "method" => {
                    if let Some(label) = target_label {
                        push_unique_pair(&mut methods, (&edge.source, key(label)), &edge.target);
                        enclosing_method
                            .entry(edge.target.clone())
                            .or_insert_with(|| edge.source.clone());
                        enclosing_member
                            .entry(edge.target.clone())
                            .or_insert_with(|| edge.source.clone());
                    }
                }
                "defines" => {
                    if let Some(label) = target_label {
                        push_unique_pair(&mut defines, (&edge.source, key(label)), &edge.target);
                        enclosing_member
                            .entry(edge.target.clone())
                            .or_insert_with(|| edge.source.clone());
                    }
                }
                "contains" => {
                    if let Some(label) = target_label {
                        push_unique(
                            contains.entry(edge.source.clone()).or_default(),
                            key(label),
                            &edge.target,
                        );
                        file_of.insert(edge.target.clone(), edge.source.clone());
                    }
                }
                "imports" | "imports_from" => {
                    imports
                        .entry(edge.source.clone())
                        .or_default()
                        .insert(edge.target.clone());
                }
                "inherits" => push_unique(&mut bases, edge.source.clone(), &edge.target),
                _ => {}
            }
        }
        Self {
            nodes: nodes_by_id,
            types,
            methods,
            defines,
            enclosing_method,
            enclosing_member,
            contains,
            file_of,
            imports,
            bases,
        }
    }

    fn unique_type(&self, name: &str) -> Option<&str> {
        unique(self.types.get(&key(name)))
    }

    fn unique_method(&self, owner: &str, name: &str) -> Option<&str> {
        unique(self.methods.get(&(owner.to_owned(), key(name))))
    }

    fn unique_cpp_member(&self, owner: &str, name: &str) -> Option<&str> {
        self.unique_method(owner, name)
            .or_else(|| unique(self.defines.get(&(owner.to_owned(), key(name)))))
    }
}

#[derive(Default)]
struct TypeTables {
    swift: HashMap<String, HashMap<String, String>>,
    typescript: HashMap<String, HashMap<String, String>>,
    cpp: HashMap<String, HashMap<String, String>>,
    csharp: HashMap<String, HashMap<String, String>>,
    objc: HashMap<String, HashMap<String, String>>,
}

impl TypeTables {
    fn new(extractions: &[Extraction]) -> Self {
        let mut tables = Self::default();
        for extraction in extractions {
            collect_table(extraction, "swift_type_table", &mut tables.swift);
            collect_table(extraction, "ts_type_table", &mut tables.typescript);
            collect_table(extraction, "cpp_type_table", &mut tables.cpp);
            collect_table(extraction, "csharp_type_table", &mut tables.csharp);
            collect_table(extraction, "objc_type_table", &mut tables.objc);
        }
        tables
    }
}

fn resolve_typed_members(
    calls: &[RawCall],
    indexes: &Indexes,
    tables: &TypeTables,
    existing: &mut HashSet<(String, String)>,
    edges: &mut Vec<EdgeRecord>,
) {
    for call in calls {
        if call.is_member_call != Some(true) {
            continue;
        }
        let Some(receiver) = receiver(call) else {
            continue;
        };
        let source = call.source_file.as_str();
        let language = call.lang.as_deref();
        let family = member_family(source, language);
        let owner = match family {
            MemberFamily::Swift => typed_owner(receiver, source, &tables.swift, indexes, None),
            MemberFamily::Typescript => {
                let result = typed_owner(receiver, source, &tables.typescript, indexes, None);
                if result
                    .as_ref()
                    .is_some_and(|(owner, _)| is_builtin_type(indexes, owner))
                {
                    None
                } else {
                    result
                }
            }
            MemberFamily::Cpp => {
                if language != Some("cpp") {
                    None
                } else if receiver == "this" {
                    indexes
                        .enclosing_member
                        .get(&call.caller_nid)
                        .cloned()
                        .map(|owner| (owner, true))
                } else {
                    typed_owner(receiver, source, &tables.cpp, indexes, None)
                }
            }
            MemberFamily::Csharp => {
                if language != Some("csharp") {
                    None
                } else if receiver == "this" {
                    indexes
                        .enclosing_method
                        .get(&call.caller_nid)
                        .cloned()
                        .map(|owner| (owner, true))
                } else {
                    typed_owner(receiver, source, &tables.csharp, indexes, None)
                }
            }
            MemberFamily::Java => {
                if language != Some("java") {
                    None
                } else if receiver == "this" {
                    indexes
                        .enclosing_method
                        .get(&call.caller_nid)
                        .cloned()
                        .map(|owner| (owner, true))
                } else {
                    let explicit = call
                        .receiver_type
                        .as_ref()
                        .and_then(|value| value.as_deref());
                    let name = explicit.or_else(|| starts_upper(receiver).then_some(receiver));
                    name.and_then(|name| {
                        indexes
                            .unique_type(name)
                            .map(|owner| (owner.to_owned(), explicit.is_none()))
                    })
                }
            }
            MemberFamily::Objc => {
                if language != Some("objc") {
                    None
                } else if matches!(receiver, "self" | "super") {
                    indexes
                        .enclosing_method
                        .get(&call.caller_nid)
                        .cloned()
                        .map(|owner| (owner, true))
                } else {
                    typed_owner(receiver, source, &tables.objc, indexes, None)
                }
            }
            MemberFamily::Other => None,
        };
        let Some((owner, exact)) = owner else {
            continue;
        };
        let method = if family == MemberFamily::Cpp {
            indexes.unique_cpp_member(&owner, &call.callee)
        } else {
            indexes.unique_method(&owner, &call.callee)
        };
        let strict_method = matches!(family, MemberFamily::Csharp | MemberFamily::Java);
        if strict_method && method.is_none() {
            continue;
        }
        let (target, relation_name) =
            method.map_or((owner.as_str(), "references"), |method| (method, "calls"));
        let confidence = if family == MemberFamily::Typescript || exact {
            ("EXTRACTED", 1.0)
        } else {
            ("INFERRED", 0.8)
        };
        emit(
            call,
            target,
            relation_name,
            "call",
            confidence,
            existing,
            edges,
        );
    }
}

fn resolve_python_members(
    calls: &[RawCall],
    indexes: &Indexes,
    existing: &mut HashSet<(String, String)>,
    edges: &mut Vec<EdgeRecord>,
) {
    for call in calls {
        if call.is_member_call != Some(true) || extension(&call.source_file) != "py" {
            continue;
        }
        let Some(receiver) = receiver(call) else {
            continue;
        };
        let target = if starts_upper(receiver) {
            indexes
                .unique_type(receiver)
                .and_then(|owner| indexes.unique_method(owner, &call.callee))
        } else {
            let Some(caller_file) = indexes.file_of.get(&call.caller_nid) else {
                continue;
            };
            let imported = indexes.imports.get(caller_file);
            let modules = indexes
                .contains
                .keys()
                .filter(|module| imported.is_some_and(|set| set.contains(*module)))
                .filter(|module| module_stem(indexes.nodes.get(*module)) == key(receiver))
                .collect::<Vec<_>>();
            if modules.len() != 1 {
                continue;
            }
            unique(indexes.contains[modules[0]].get(&key(&call.callee)))
        };
        if let Some(target) = target {
            emit(
                call,
                target,
                "calls",
                "call",
                ("EXTRACTED", 1.0),
                existing,
                edges,
            );
        }
    }
}

fn resolve_ruby_members(
    calls: &[RawCall],
    indexes: &Indexes,
    existing: &mut HashSet<(String, String)>,
    edges: &mut Vec<EdgeRecord>,
) {
    let mut ruby_types = HashMap::new();
    for node in indexes.nodes.values() {
        if matches!(
            extension(&node.string("source_file")).as_str(),
            "rb" | "rake"
        ) && is_bare_constant(node.label())
        {
            push_unique(&mut ruby_types, key(node.label()), &node.id);
        }
    }
    for call in calls {
        if !matches!(extension(&call.source_file).as_str(), "rb" | "rake") {
            continue;
        }
        if call.extensions.get("is_mixin").and_then(Value::as_bool) == Some(true) {
            if let Some(target) = unique(ruby_types.get(&key(&call.callee))) {
                emit(
                    call,
                    target,
                    "mixes_in",
                    "mixin",
                    ("EXTRACTED", 1.0),
                    existing,
                    edges,
                );
            }
            continue;
        }
        if call.is_member_call != Some(true) {
            continue;
        }
        let Some(receiver) = receiver(call) else {
            continue;
        };
        let type_name = if starts_upper(receiver) {
            Some(receiver)
        } else {
            call.receiver_type
                .as_ref()
                .and_then(|value| value.as_deref())
        };
        let Some(owner) = type_name.and_then(|name| unique(ruby_types.get(&key(name)))) else {
            continue;
        };
        let target = if starts_upper(receiver) && call.callee == "new" {
            owner
        } else {
            indexes.unique_method(owner, &call.callee).unwrap_or(owner)
        };
        emit(
            call,
            target,
            "calls",
            "call",
            ("EXTRACTED", 1.0),
            existing,
            edges,
        );
    }
}

fn resolve_pascal_inherited(
    calls: &[RawCall],
    indexes: &Indexes,
    existing: &mut HashSet<(String, String)>,
    edges: &mut Vec<EdgeRecord>,
) {
    for call in calls {
        if !matches!(
            extension(&call.source_file).as_str(),
            "pas" | "pp" | "dpr" | "dpk" | "inc"
        ) {
            continue;
        }
        let Some(owner) = indexes.enclosing_method.get(&call.caller_nid) else {
            continue;
        };
        let mut queue = indexes
            .bases
            .get(owner)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .collect::<VecDeque<_>>();
        let mut seen = HashSet::new();
        let mut target = None;
        while let Some(base) = queue.pop_front() {
            if !seen.insert(base.clone()) {
                continue;
            }
            let candidates = indexes.methods.get(&(base.clone(), key(&call.callee)));
            if candidates.is_some() {
                target = unique(candidates);
                break;
            }
            queue.extend(indexes.bases.get(&base).into_iter().flatten().cloned());
        }
        if let Some(target) = target {
            emit(
                call,
                target,
                "calls",
                "call",
                ("EXTRACTED", 1.0),
                existing,
                edges,
            );
        }
    }
}

fn typed_owner(
    receiver: &str,
    source: &str,
    tables: &HashMap<String, HashMap<String, String>>,
    indexes: &Indexes,
    explicit_type: Option<&str>,
) -> Option<(String, bool)> {
    let (name, exact) = if let Some(name) = explicit_type {
        (name, false)
    } else if starts_upper(receiver) {
        (receiver, true)
    } else {
        (tables.get(source)?.get(receiver)?.as_str(), false)
    };
    indexes
        .unique_type(name)
        .map(|owner| (owner.to_owned(), exact))
}

fn emit(
    call: &RawCall,
    target: &str,
    relation: &str,
    context: &str,
    confidence: (&str, f64),
    existing: &mut HashSet<(String, String)>,
    edges: &mut Vec<EdgeRecord>,
) {
    if target == call.caller_nid || !existing.insert((call.caller_nid.clone(), target.to_owned())) {
        return;
    }
    let mut attributes = Map::new();
    attributes.insert("relation".into(), Value::String(relation.to_owned()));
    attributes.insert("context".into(), Value::String(context.to_owned()));
    attributes.insert("confidence".into(), Value::String(confidence.0.to_owned()));
    attributes.insert("confidence_score".into(), Value::from(confidence.1));
    attributes.insert(
        "source_file".into(),
        Value::String(call.source_file.clone()),
    );
    attributes.insert(
        "source_location".into(),
        Value::String(call.source_location.clone()),
    );
    attributes.insert("weight".into(), Value::from(1.0));
    edges.push(EdgeRecord {
        source: call.caller_nid.clone(),
        target: target.to_owned(),
        attributes,
    });
}

fn collect_table(
    extraction: &Extraction,
    name: &str,
    output: &mut HashMap<String, HashMap<String, String>>,
) {
    let Some(object) = extraction.extensions.get(name).and_then(Value::as_object) else {
        return;
    };
    let Some(path) = object.get("path").and_then(Value::as_str) else {
        return;
    };
    let Some(table) = object.get("table").and_then(Value::as_object) else {
        return;
    };
    output.insert(
        path.to_owned(),
        table
            .iter()
            .filter_map(|(key, value)| value.as_str().map(|value| (key.clone(), value.to_owned())))
            .collect(),
    );
}

fn relation(edge: &EdgeRecord) -> &str {
    edge.attributes
        .get("relation")
        .and_then(Value::as_str)
        .unwrap_or_default()
}

fn is_type_like(node: &NodeRecord) -> bool {
    if node.string("type") == "namespace" || node.string("file_type") != "code" {
        return false;
    }
    let label = node.label().trim();
    !label.is_empty() && !label.ends_with(')') && !label.starts_with('.') && !label.contains('.')
}

fn is_builtin_type(indexes: &Indexes, owner: &str) -> bool {
    indexes
        .nodes
        .get(owner)
        .is_some_and(|node| is_builtin(node.label()))
}

fn is_builtin(name: &str) -> bool {
    matches!(
        name,
        "String"
            | "Number"
            | "Boolean"
            | "Object"
            | "Array"
            | "Symbol"
            | "BigInt"
            | "Date"
            | "RegExp"
            | "Error"
            | "TypeError"
            | "RangeError"
            | "SyntaxError"
            | "ReferenceError"
            | "EvalError"
            | "URIError"
            | "Promise"
            | "Map"
            | "Set"
            | "WeakMap"
            | "WeakSet"
            | "JSON"
            | "Math"
            | "Reflect"
            | "Proxy"
            | "Intl"
            | "URL"
            | "URLSearchParams"
            | "FormData"
            | "Blob"
            | "File"
            | "Headers"
            | "Request"
            | "Response"
            | "AbortController"
            | "AbortSignal"
            | "TextEncoder"
            | "TextDecoder"
            | "console"
    )
}

fn key(label: &str) -> String {
    label
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .flat_map(char::to_lowercase)
        .collect()
}

fn receiver(call: &RawCall) -> Option<&str> {
    call.receiver.as_ref().and_then(|value| value.as_deref())
}

fn starts_upper(value: &str) -> bool {
    value.chars().next().is_some_and(char::is_uppercase)
}

fn unique(values: Option<&Vec<String>>) -> Option<&str> {
    values
        .filter(|values| values.len() == 1)
        .and_then(|values| values.first())
        .map(String::as_str)
}

fn push_unique(map: &mut HashMap<String, Vec<String>>, key: String, value: &str) {
    let values = map.entry(key).or_default();
    if !values.iter().any(|item| item == value) {
        values.push(value.to_owned());
    }
}

fn push_unique_pair(
    map: &mut HashMap<(String, String), Vec<String>>,
    key: (&str, String),
    value: &str,
) {
    let values = map.entry((key.0.to_owned(), key.1)).or_default();
    if !values.iter().any(|item| item == value) {
        values.push(value.to_owned());
    }
}

fn module_stem(node: Option<&NodeRecord>) -> String {
    let Some(node) = node else {
        return String::new();
    };
    let source = node.string("source_file");
    let stem = std::path::Path::new(&source)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or_else(|| node.label());
    key(stem)
}

fn is_bare_constant(label: &str) -> bool {
    let mut chars = label.chars();
    chars.next().is_some_and(|first| first.is_ascii_uppercase())
        && chars.all(|character| character.is_ascii_alphanumeric() || character == '_')
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MemberFamily {
    Swift,
    Typescript,
    Cpp,
    Csharp,
    Java,
    Objc,
    Other,
}

fn member_family(source: &str, language: Option<&str>) -> MemberFamily {
    match language {
        Some("cpp") => MemberFamily::Cpp,
        Some("csharp") => MemberFamily::Csharp,
        Some("java") => MemberFamily::Java,
        Some("objc") => MemberFamily::Objc,
        _ => match extension(source).as_str() {
            "swift" => MemberFamily::Swift,
            "ts" | "tsx" | "mts" | "cts" | "js" | "jsx" => MemberFamily::Typescript,
            _ => MemberFamily::Other,
        },
    }
}

fn extension(source: &str) -> String {
    std::path::Path::new(source)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(value: Value) -> NodeRecord {
        serde_json::from_value(value).unwrap_or_else(|_| NodeRecord {
            id: String::new(),
            attributes: Map::new(),
        })
    }

    fn call(receiver_value: Option<Option<&str>>) -> RawCall {
        RawCall {
            caller_nid: "caller".to_owned(),
            callee: "run".to_owned(),
            is_member_call: Some(true),
            source_file: "source.ts".to_owned(),
            source_location: "L2".to_owned(),
            receiver: receiver_value.map(|value| value.map(str::to_owned)),
            receiver_type: None,
            lang: Some("typescript".to_owned()),
            extensions: Map::new(),
        }
    }

    #[test]
    fn member_classification_and_collection_helpers_cover_boundary_shapes() {
        assert!(is_type_like(&node(serde_json::json!({
            "id":"type","label":"Service","file_type":"code"
        }))));
        for value in [
            serde_json::json!({"id":"n","label":"Space","type":"namespace","file_type":"code"}),
            serde_json::json!({"id":"n","label":"run()","file_type":"code"}),
            serde_json::json!({"id":"n","label":".hidden","file_type":"code"}),
            serde_json::json!({"id":"n","label":"A.B","file_type":"code"}),
            serde_json::json!({"id":"n","label":"","file_type":"code"}),
            serde_json::json!({"id":"n","label":"External","file_type":"document"}),
        ] {
            assert!(!is_type_like(&node(value)));
        }
        assert!(is_builtin("Promise"));
        assert!(!is_builtin("Custom"));
        assert_eq!(key("HTTP_Client.run()"), "httpclientrun");
        assert_eq!(receiver(&call(Some(Some("service")))), Some("service"));
        assert_eq!(receiver(&call(Some(None))), None);
        assert_eq!(receiver(&call(None)), None);
        assert!(starts_upper("Service"));
        assert!(!starts_upper("service"));
        assert_eq!(unique(Some(&vec!["one".to_owned()])), Some("one"));
        assert_eq!(
            unique(Some(&vec!["one".to_owned(), "two".to_owned()])),
            None
        );
        assert_eq!(unique(None), None);

        let mut values = HashMap::new();
        push_unique(&mut values, "key".to_owned(), "value");
        push_unique(&mut values, "key".to_owned(), "value");
        assert_eq!(values["key"], ["value"]);
        let mut pairs = HashMap::new();
        push_unique_pair(&mut pairs, ("owner", "member".to_owned()), "target");
        push_unique_pair(&mut pairs, ("owner", "member".to_owned()), "target");
        assert_eq!(pairs.len(), 1);
    }

    #[test]
    fn module_family_and_table_helpers_preserve_language_specific_contracts() {
        assert_eq!(module_stem(None), "");
        assert_eq!(
            module_stem(Some(&node(serde_json::json!({
                "id":"m","label":"Fallback","source_file":"src/MyModule.ts"
            })))),
            "mymodule"
        );
        assert_eq!(
            module_stem(Some(&node(serde_json::json!({
                "id":"m","label":"Fallback","source_file":""
            })))),
            "fallback"
        );
        assert!(is_bare_constant("HTTP_2"));
        assert!(!is_bare_constant("notConstant"));
        assert_eq!(member_family("x.any", Some("cpp")), MemberFamily::Cpp);
        assert_eq!(member_family("x.any", Some("csharp")), MemberFamily::Csharp);
        assert_eq!(member_family("x.any", Some("java")), MemberFamily::Java);
        assert_eq!(member_family("x.any", Some("objc")), MemberFamily::Objc);
        assert_eq!(member_family("x.swift", None), MemberFamily::Swift);
        assert_eq!(member_family("x.TSX", None), MemberFamily::Typescript);
        assert_eq!(member_family("x.unknown", None), MemberFamily::Other);

        let mut extraction = Extraction::default();
        extraction.extensions.insert(
            "types".to_owned(),
            serde_json::json!({"path":"source.ts","table":{"value":"Service","bad":7}}),
        );
        let mut tables = HashMap::new();
        collect_table(&extraction, "missing", &mut tables);
        collect_table(&extraction, "types", &mut tables);
        assert_eq!(
            tables["source.ts"].get("value").map(String::as_str),
            Some("Service")
        );
        assert!(!tables["source.ts"].contains_key("bad"));
    }

    #[test]
    fn edge_emission_rejects_self_and_duplicates_and_stamps_metadata() {
        let call = call(Some(Some("service")));
        let mut existing = HashSet::new();
        let mut edges = Vec::new();
        emit(
            &call,
            "caller",
            "calls",
            "call",
            ("INFERRED", 0.8),
            &mut existing,
            &mut edges,
        );
        emit(
            &call,
            "target",
            "calls",
            "call",
            ("INFERRED", 0.8),
            &mut existing,
            &mut edges,
        );
        emit(
            &call,
            "target",
            "calls",
            "call",
            ("INFERRED", 0.8),
            &mut existing,
            &mut edges,
        );
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].string("relation"), "calls");
        assert_eq!(edges[0].attributes["confidence_score"], 0.8);
        assert_eq!(edges[0].attributes["source_location"], "L2");
    }

    #[test]
    fn swift_registry_compatibility_precedes_other_language_member_passes() {
        let mut swift = Extraction::default();
        swift.extensions.insert(
            "swift_type_table".to_owned(),
            serde_json::json!({"path":"source.swift","table":{"value":"Service"}}),
        );
        let python = Extraction {
            raw_calls: Some(vec![RawCall {
                caller_nid: "caller".to_owned(),
                callee: "glob".to_owned(),
                is_member_call: Some(true),
                source_file: "tests/test_extract.py".to_owned(),
                source_location: "L61".to_owned(),
                receiver: Some(Some("Fixtures".to_owned())),
                receiver_type: None,
                lang: None,
                extensions: Map::new(),
            }]),
            ..Extraction::default()
        };
        let mut merged = Extraction {
            nodes: vec![
                node(serde_json::json!({
                    "id":"file","label":"coverage_paths.rs","file_type":"code",
                    "source_file":"coverage_paths.rs"
                })),
                node(serde_json::json!({
                    "id":"fixtures","label":"Fixtures","file_type":"code",
                    "source_file":"coverage_paths.rs"
                })),
                node(serde_json::json!({
                    "id":"caller","label":"test_extract()","file_type":"code",
                    "source_file":"tests/test_extract.py"
                })),
            ],
            ..Extraction::default()
        };
        merged.edges.push(EdgeRecord {
            source: "file".to_owned(),
            target: "fixtures".to_owned(),
            attributes: Map::from_iter([(
                "relation".to_owned(),
                Value::String("contains".to_owned()),
            )]),
        });

        resolve_language_calls(&[swift, python], &mut merged);

        let edge = merged
            .edges
            .iter()
            .find(|edge| edge.source == "caller" && edge.target == "fixtures")
            .unwrap_or_else(|| std::process::abort());
        assert_eq!(relation(edge), "references");
        assert_eq!(edge.string("context"), "call");
        assert_eq!(edge.string("confidence"), "EXTRACTED");
    }
}
