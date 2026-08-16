#[test]
fn kotlin_named_default_member_and_imported_extension_calls_resolve_stably() {
    let service_source = br#"
package demo
class Service {
    fun render(prefix: String = "x", count: Int = 1): String = prefix
}
"#;
    let extension_source = br#"
package ext
fun String.decorate(prefix: String = "x"): String = this
"#;
    let caller_source = br#"
package demo
import ext.decorate
class Caller {
    fun run(service: Service, text: String) {
        service.render(count = 2)
        text.decorate()
    }
}
"#;
    let service = extract("src/Service.kt", service_source);
    let extension = extract("src/Extensions.kt", extension_source);
    let caller = extract("src/Caller.kt", caller_source);
    let sources = HashMap::from([
        (
            "src/Service.kt".to_owned(),
            String::from_utf8_lossy(service_source).into_owned(),
        ),
        (
            "src/Extensions.kt".to_owned(),
            String::from_utf8_lossy(extension_source).into_owned(),
        ),
        (
            "src/Caller.kt".to_owned(),
            String::from_utf8_lossy(caller_source).into_owned(),
        ),
    ]);
    let first = compass_resolve::resolve(
        &[service.clone(), extension.clone(), caller.clone()],
        &sources,
    );
    let reversed = compass_resolve::resolve(&[caller, extension, service], &sources);
    assert_eq!(universal_edges(&first), universal_edges(&reversed));

    for target in ["demo.Service::render", "ext::decorate"] {
        let declaration = first
            .nodes
            .iter()
            .find(|node| node.string("qualified_name") == target)
            .unwrap_or_else(|| panic!("missing {target}: {:#?}", first.nodes));
        assert!(first.edges.iter().any(|edge| {
            edge.target == declaration.id
                && edge.string("relation") == "calls"
                && edge.string("resolution_rule") == "kotlin-named-default-arguments"
        }), "missing resolved call to {target}: {:#?}", first.edges);
    }
}

#[test]
fn kotlin_never_terminal_matches_a_java_declaration_without_compiler_evidence() {
    let java = extract(
        "src/Service.java",
        b"package demo; public class Service { public void render() {} }",
    );
    let kotlin_source = br#"
package demo
class Caller {
    fun run(service: Service) { service.render() }
}
"#;
    let kotlin = extract("src/Caller.kt", kotlin_source);
    let sources = HashMap::from([
        (
            "src/Service.java".to_owned(),
            "package demo; public class Service { public void render() {} }".to_owned(),
        ),
        (
            "src/Caller.kt".to_owned(),
            String::from_utf8_lossy(kotlin_source).into_owned(),
        ),
    ]);
    let resolved = compass_resolve::resolve(&[java, kotlin], &sources);
    let java_render = resolved
        .nodes
        .iter()
        .find(|node| {
            node.string("language") == "java"
                && node.string("qualified_name") == "demo.Service::render"
        })
        .expect("Java render declaration");
    assert!(resolved.edges.iter().all(|edge| {
        !(edge.target == java_render.id
            && edge.string("language") == "kotlin"
            && edge.string("relation") == "calls")
    }));
}

#[test]
fn kotlin_objects_project_to_supported_class_nodes() {
    let source = br#"
package demo
class Owner {
    companion object Factory
}
object Singleton
"#;
    let extraction = extract("src/Objects.kt", source);
    let sources = HashMap::from([(
        "src/Objects.kt".to_owned(),
        String::from_utf8_lossy(source).into_owned(),
    )]);
    let resolved = compass_resolve::resolve(&[extraction], &sources);
    for qualified in ["demo.Owner.Factory", "demo.Singleton"] {
        let node = resolved
            .nodes
            .iter()
            .find(|node| node.string("qualified_name") == qualified)
            .unwrap_or_else(|| panic!("missing {qualified}: {:#?}", resolved.nodes));
        assert_eq!(node.string("symbol_kind"), "class");
    }
}
