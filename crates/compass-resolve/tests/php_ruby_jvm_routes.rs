use std::error::Error;
use std::path::{Path, PathBuf};

use compass_languages::{
    Engine, Extraction, ExtractorKind, FrameworkLimits, RawFrameworkFact, Registry,
};
use compass_model::provenance::ResolutionState;
use compass_resolve::frameworks::resolve_and_publish_framework_routes;

fn fixture(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/code-graph/routes")
        .join(relative)
}

fn extract(relative: &str) -> Result<Extraction, Box<dyn Error>> {
    let path = fixture(relative);
    let extraction = Engine::default().extract(&path)?;
    if let Some(error) = &extraction.error {
        return Err(error.clone().into());
    }
    Ok(extraction)
}

fn merge(target: &mut Extraction, mut source: Extraction) {
    target.nodes.append(&mut source.nodes);
    target.edges.append(&mut source.edges);
    target.framework_facts.append(&mut source.framework_facts);
}

#[test]
fn laravel_routes_expand_resources_prefixes_and_handler_syntaxes() -> Result<(), Box<dyn Error>> {
    let mut extraction = extract("php/laravel.php")?;
    let routes = extraction
        .framework_facts
        .iter()
        .filter_map(|fact| match fact {
            RawFrameworkFact::Route(route) => Some(route),
            RawFrameworkFact::Domain(_) => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        routes
            .iter()
            .filter(|route| route.rule.as_deref() == Some("laravel-resource-expansion"))
            .count(),
        8
    );
    assert!(routes.iter().any(|route| {
        route.operation == "DELETE" && route.normalized_path == "/admin/users/{id}"
    }));
    assert!(routes.iter().any(|route| {
        route.operation == "POST"
            && route.handler_reference == "UserController.store"
            && route.normalized_path == "/users"
    }));

    let resolved =
        resolve_and_publish_framework_routes(&mut extraction, FrameworkLimits::default())?;
    assert!(resolved.iter().any(|route| {
        route.route.normalized_path == "/users"
            && route.route.operation == "GET"
            && route.state == ResolutionState::Exact
    }));
    assert!(extract("php/near_matches.php")?.framework_facts.is_empty());
    Ok(())
}

#[test]
fn drupal_yaml_and_hook_files_publish_auditable_routes() -> Result<(), Box<dyn Error>> {
    let mut extraction = extract("php/drupal.routing.yml")?;
    merge(&mut extraction, extract("php/drupal.module")?);
    let resolved =
        resolve_and_publish_framework_routes(&mut extraction, FrameworkLimits::default())?;

    let view = resolved
        .iter()
        .find(|route| route.route.normalized_path == "/examples/{example}")
        .ok_or("missing Drupal controller route")?;
    assert_eq!(view.route.operation, "GET");
    assert_eq!(view.route.origin.as_str(), "config");
    assert_eq!(view.state, ResolutionState::Exact);
    assert!(resolved.iter().any(|route| {
        route.route.operation == "HOOK"
            && route.route.handler_reference == "hook_entity_type_build"
            && route.state == ResolutionState::Exact
    }));
    assert!(extraction.edges.iter().any(|edge| {
        edge.string("relation") == "routes_to"
            && edge.string("_origin") == "config"
            && edge.string("source_file").ends_with("drupal.routing.yml")
    }));
    Ok(())
}

#[test]
fn rails_routes_resolve_to_controller_actions_and_compose_namespaces() -> Result<(), Box<dyn Error>>
{
    let mut extraction = extract("ruby/rails.rb")?;
    let resolved =
        resolve_and_publish_framework_routes(&mut extraction, FrameworkLimits::default())?;
    assert!(resolved.iter().any(|route| {
        route.route.operation == "GET"
            && route.route.normalized_path == "/users/:id"
            && route.route.handler_reference == "UsersController.show"
            && route.state == ResolutionState::Exact
    }));
    assert!(resolved.iter().any(|route| {
        route.route.normalized_path == "/admin/dashboard"
            && route.route.handler_reference == "DashboardController.index"
    }));
    assert!(extract("ruby/near_matches.rb")?.framework_facts.is_empty());
    Ok(())
}

#[test]
fn spring_composes_class_and_method_mappings_without_custom_annotation_matches()
-> Result<(), Box<dyn Error>> {
    let mut extraction = extract("jvm/SpringController.java")?;
    let resolved =
        resolve_and_publish_framework_routes(&mut extraction, FrameworkLimits::default())?;
    assert!(resolved.iter().any(|route| {
        route.route.operation == "GET"
            && route.route.normalized_path == "/api/users/{id}"
            && route.route.handler_reference == "SpringController.show"
            && route.state == ResolutionState::Exact
    }));
    assert_eq!(
        resolved
            .iter()
            .filter(|route| route.route.normalized_path == "/api/search")
            .count(),
        2
    );
    assert!(extract("jvm/NearMatches.java")?.framework_facts.is_empty());
    Ok(())
}

#[test]
fn play_config_routes_resolve_java_scala_and_injected_controllers() -> Result<(), Box<dyn Error>> {
    let route_path = fixture("jvm/play/conf/routes");
    let spec = Registry::resolve(&route_path).ok_or("Play routes are not registered")?;
    assert_eq!(spec.kind, ExtractorKind::FrameworkConfig);

    let mut extraction = extract("jvm/play/conf/routes")?;
    merge(&mut extraction, extract("jvm/PlayController.java")?);
    merge(&mut extraction, extract("jvm/PlayController.scala")?);
    let resolved =
        resolve_and_publish_framework_routes(&mut extraction, FrameworkLimits::default())?;
    assert_eq!(resolved.len(), 3);
    assert!(
        resolved
            .iter()
            .all(|route| route.route.origin.as_str() == "config")
    );
    assert!(
        resolved
            .iter()
            .all(|route| route.state == ResolutionState::Exact)
    );
    Ok(())
}
