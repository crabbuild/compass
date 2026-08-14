use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use compass_languages::{
    Engine, Extraction, ExtractorKind, FrameworkLimits, RawFrameworkFact, Registry,
};
use compass_model::provenance::ResolutionState;
use compass_resolve::{frameworks::resolve_and_publish_framework_routes, resolve};

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

fn extract_and_resolve(relatives: &[&str]) -> Result<Extraction, Box<dyn Error>> {
    let mut extractions = Vec::with_capacity(relatives.len());
    let mut sources = HashMap::new();
    for relative in relatives {
        let path = fixture(relative);
        let source = fs::read_to_string(&path)?;
        let extraction = extract(relative)?;
        sources.insert((*relative).to_owned(), source.clone());
        sources.insert(path.to_string_lossy().into_owned(), source.clone());
        for source_file in extraction
            .nodes
            .iter()
            .map(|node| node.string("source_file"))
            .filter(|source_file| !source_file.is_empty())
            .chain(
                extraction
                    .semantic_evidence
                    .iter()
                    .flat_map(|batch| batch.declarations.iter())
                    .map(|declaration| declaration.range.source_file.clone()),
            )
        {
            sources.insert(source_file, source.clone());
        }
        extractions.push(extraction);
    }
    Ok(resolve(&extractions, &sources))
}

#[test]
fn laravel_routes_expand_resources_prefixes_and_handler_syntaxes() -> Result<(), Box<dyn Error>> {
    let mut extraction = extract("php/laravel.php")?;
    let routes = extraction
        .framework_facts
        .iter()
        .filter_map(|fact| match fact {
            RawFrameworkFact::Route(route) => Some(route),
            RawFrameworkFact::Domain(_) | RawFrameworkFact::Annotation(_) => None,
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
            && route.handler_reference == "usercontroller.store"
            && route.normalized_path == "/users"
    }));

    let resolved =
        resolve_and_publish_framework_routes(&mut extraction, FrameworkLimits::default())?;
    assert!(resolved.iter().any(|route| {
        route.route.normalized_path == "/users"
            && route.route.operation == "GET"
            && route.state == ResolutionState::Exact
    }));

    let constrained = Engine::default().extract_source(
        Path::new("routes/constrained.php"),
        br#"<?php
use Illuminate\Support\Facades\Route;
Route::resource('/categories', CategoryController::class)->only(['index', 'show']);
"#,
    )?;
    let constrained_routes = constrained
        .framework_facts
        .iter()
        .filter_map(|fact| match fact {
            RawFrameworkFact::Route(route) => Some(route),
            RawFrameworkFact::Domain(_) | RawFrameworkFact::Annotation(_) => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(constrained_routes.len(), 2);
    assert!(constrained_routes.iter().any(|route| {
        route.operation == "GET" && route.normalized_path == "/categories/{category}"
    }));
    assert!(constrained_routes.iter().all(|route| {
        !matches!(
            route
                .detail
                .get("resourceAction")
                .and_then(|value| value.as_str()),
            Some("create" | "edit")
        )
    }));
    assert!(extract("php/near_matches.php")?.framework_facts.is_empty());
    Ok(())
}

#[test]
fn laravel_routes_require_the_exact_facade_receiver_and_static_handler()
-> Result<(), Box<dyn Error>> {
    let mut engine = Engine::default();
    let wrong_import = engine.extract_source(
        Path::new("routes/wrong.php"),
        br#"<?php
use Acme\Routing\Route;
Route::get('/wrong', [WrongController::class, 'show']);
"#,
    )?;
    assert!(wrong_import.framework_facts.is_empty());

    let missing_import = engine.extract_source(
        Path::new("routes/missing.php"),
        br#"<?php
Route::get('/missing', [MissingController::class, 'show']);
"#,
    )?;
    assert!(missing_import.framework_facts.is_empty());

    let routes = engine.extract_source(
        Path::new("app/routes.php"),
        br#"<?php
use Illuminate\Support\Facades\Route as Router;
use Illuminate\Support\Facades\{Route as GroupedRouter};
use Acme\Routing\Route;

Router::get('/alias', [AliasController::class, 'show']);
GroupedRouter::put('/grouped-alias', [AliasController::class, 'update']);
\Illuminate\Support\Facades\Route::post('/qualified', 'QualifiedController@store');
Router::get('/dynamic', $handler);
Router::$method('/variable-method', [AliasController::class, 'show']);

Route::prefix('/wrong')->group(function () {
    Router::get('/unprefixed', [AliasController::class, 'show']);
});
"#,
    )?;
    let routes = routes
        .framework_facts
        .iter()
        .filter_map(|fact| match fact {
            RawFrameworkFact::Route(route) => Some(route),
            RawFrameworkFact::Domain(_) | RawFrameworkFact::Annotation(_) => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(routes.len(), 4, "routes={routes:#?}");
    assert!(routes.iter().any(|route| {
        route.normalized_path == "/alias" && route.handler_reference == "aliascontroller.show"
    }));
    assert!(routes.iter().any(|route| {
        route.normalized_path == "/grouped-alias"
            && route.handler_reference == "aliascontroller.update"
    }));
    assert!(routes.iter().any(|route| {
        route.normalized_path == "/qualified"
            && route.handler_reference == "qualifiedcontroller.store"
    }));
    assert!(routes.iter().any(|route| {
        route.normalized_path == "/unprefixed" && route.handler_reference == "aliascontroller.show"
    }));
    assert!(
        routes
            .iter()
            .all(|route| !route.normalized_path.contains("/wrong"))
    );
    Ok(())
}

#[test]
fn drupal_yaml_and_hook_files_publish_auditable_routes() -> Result<(), Box<dyn Error>> {
    let mut extraction = extract_and_resolve(&["php/drupal.routing.yml", "php/drupal.module"])?;
    let resolved =
        resolve_and_publish_framework_routes(&mut extraction, FrameworkLimits::default())?;

    let views = resolved
        .iter()
        .filter(|route| route.route.normalized_path == "/examples/{example}")
        .collect::<Vec<_>>();
    assert_eq!(views.len(), 2, "routes={resolved:#?}");
    assert_eq!(
        views
            .iter()
            .map(|route| route.route.operation.as_str())
            .collect::<std::collections::BTreeSet<_>>(),
        std::collections::BTreeSet::from(["GET", "POST"])
    );
    assert!(
        views.iter().all(|route| {
            route.route.origin.as_str() == "config" && route.state == ResolutionState::Exact
        }),
        "routes={resolved:#?}"
    );
    assert!(resolved.iter().any(|route| {
        route.route.operation == "HOOK"
            && route.route.normalized_path == "/__hook/hook_entity_type_build"
            && route.route.handler_reference == "drupal_entity_type_build"
            && route.state == ResolutionState::Exact
    }));
    assert_eq!(
        resolved
            .iter()
            .filter(|route| route.route.operation == "HOOK")
            .count(),
        1,
        "same-prefix helper functions must not become Drupal hooks"
    );
    assert!(resolved.iter().any(|route| {
        route.route.normalized_path == "/examples/{example}/edit"
            && route.route.handler_reference == "example.edit"
            && route.state == ResolutionState::Unresolved
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
            && route.route.handler_reference == "Admin.DashboardController.index"
    }));
    assert!(extract("ruby/near_matches.rb")?.framework_facts.is_empty());
    Ok(())
}

#[test]
fn spring_composes_class_and_method_mappings_without_custom_annotation_matches()
-> Result<(), Box<dyn Error>> {
    let mut extraction = extract_and_resolve(&["jvm/SpringController.java"])?;
    let resolved =
        resolve_and_publish_framework_routes(&mut extraction, FrameworkLimits::default())?;
    let show = resolved
        .iter()
        .find(|route| {
            route.route.operation == "GET"
                && route.route.normalized_path == "/api/users/{id}"
                && route.state == ResolutionState::Exact
        })
        .ok_or("missing exact Spring GET route")?;
    let [candidate] = show.candidates.as_slice() else {
        return Err("Spring GET route did not resolve uniquely".into());
    };
    assert_eq!(
        extraction
            .nodes
            .iter()
            .find(|node| node.id == candidate.node_id)
            .map(|node| node.string("qualified_name")),
        Some("example.SpringController::show".to_owned())
    );
    assert_eq!(
        resolved
            .iter()
            .filter(|route| route.route.normalized_path == "/api/search")
            .count(),
        2
    );
    assert!(extract("jvm/NearMatches.java")?.framework_facts.is_empty());

    let mut helper = Engine::default().extract_source(
        Path::new("src/Helper.java"),
        br#"import org.springframework.web.bind.annotation.GetMapping;
import org.springframework.web.bind.annotation.RequestMapping;
class Helper {
    @GetMapping("/not-an-endpoint")
    public String helper() { return "ok"; }
}
"#,
    )?;
    let helper_routes =
        resolve_and_publish_framework_routes(&mut helper, FrameworkLimits::default())?;
    assert!(helper_routes.is_empty(), "routes={helper_routes:#?}");
    Ok(())
}

#[test]
fn spring_uses_package_and_signature_targets_for_overloaded_controllers()
-> Result<(), Box<dyn Error>> {
    let first_source = br#"package first.api;
import org.springframework.web.bind.annotation.GetMapping;
import org.springframework.web.bind.annotation.RestController;
@RestController
class UserController {
    @GetMapping("/name")
    public String show(String id) { return id; }
    @GetMapping("/count")
    public String show(int count) { return String.valueOf(count); }
}
"#;
    let second_source = br#"package second.api;
import org.springframework.web.bind.annotation.GetMapping;
import org.springframework.web.bind.annotation.RestController;
@RestController
class UserController {
    @GetMapping("/other")
    public String show(String id) { return id; }
}
"#;
    let first_path = Path::new("src/main/java/first/api/UserController.java");
    let second_path = Path::new("src/main/java/second/api/UserController.java");
    let first = Engine::default().extract_source(first_path, first_source)?;
    let second = Engine::default().extract_source(second_path, second_source)?;
    let sources = HashMap::from([
        (
            first_path.to_string_lossy().into_owned(),
            String::from_utf8(first_source.to_vec())?,
        ),
        (
            second_path.to_string_lossy().into_owned(),
            String::from_utf8(second_source.to_vec())?,
        ),
    ]);
    let mut extraction = resolve(&[first, second], &sources);
    let resolved =
        resolve_and_publish_framework_routes(&mut extraction, FrameworkLimits::default())?;
    assert_eq!(resolved.len(), 3, "routes={resolved:#?}");
    assert!(
        resolved
            .iter()
            .all(|route| route.state == ResolutionState::Exact),
        "routes={resolved:#?}"
    );
    for (path, qualified, signature) in [
        ("/name", "first.api.UserController::show", "show(String)"),
        ("/count", "first.api.UserController::show", "show(int)"),
        ("/other", "second.api.UserController::show", "show(String)"),
    ] {
        let route = resolved
            .iter()
            .find(|route| route.route.normalized_path == path)
            .ok_or_else(|| format!("missing route {path}"))?;
        let [candidate] = route.candidates.as_slice() else {
            return Err(format!("route {path} is not exact: {route:#?}").into());
        };
        let target = extraction
            .nodes
            .iter()
            .find(|node| node.id == candidate.node_id)
            .ok_or_else(|| format!("missing target for {path}"))?;
        assert_eq!(target.string("qualified_name"), qualified);
        assert_eq!(target.string("signature"), signature);
    }
    Ok(())
}

#[test]
fn spring_kotlin_controllers_publish_exact_routes() -> Result<(), Box<dyn Error>> {
    let source = br#"
package example

import org.springframework.web.bind.annotation.GetMapping
import org.springframework.web.bind.annotation.RequestMapping
import org.springframework.web.bind.annotation.RestController

@RestController
@RequestMapping("/api")
class KotlinController {
    @GetMapping("/users/{id}")
    fun show(id: Long): String = id.toString()
}
"#;
    let mut engine = Engine::default();
    let mut extraction = engine.extract_source(Path::new("src/KotlinController.kt"), source)?;
    let resolved =
        resolve_and_publish_framework_routes(&mut extraction, FrameworkLimits::default())?;

    assert!(
        resolved.iter().any(|route| {
            route.route.operation == "GET"
                && route.route.normalized_path == "/api/users/{id}"
                && route.route.handler_reference == "KotlinController.show"
                && route.state == ResolutionState::Exact
        }),
        "routes={resolved:#?}"
    );
    Ok(())
}

#[test]
fn play_config_routes_resolve_java_scala_and_injected_controllers() -> Result<(), Box<dyn Error>> {
    let route_path = fixture("jvm/play/conf/routes");
    let spec = Registry::resolve(&route_path).ok_or("Play routes are not registered")?;
    assert_eq!(spec.kind, ExtractorKind::FrameworkConfig);

    let mut extraction = extract_and_resolve(&[
        "jvm/play/conf/routes",
        "jvm/PlayController.java",
        "jvm/PlayController.scala",
    ])?;
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
