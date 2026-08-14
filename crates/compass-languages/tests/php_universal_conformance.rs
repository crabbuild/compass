use std::error::Error;
use std::path::Path;

use compass_languages::{
    AdapterRegistry, BindingKind, CandidateRelation, Engine, EvidenceLimits, LanguageCapability,
    RawFrameworkFact, RawFrameworkOrigin, SemanticRole, UniversalAdapterProfile, validate_evidence,
};

#[test]
fn php_emits_valid_universal_evidence_for_modern_language_constructs() -> Result<(), Box<dyn Error>>
{
    let source = br#"<?php
namespace App\Contracts {
    interface Renderable { public function render(Input $input): Output; }
}

namespace App\Support {
    trait Logs { private function record(): void {} }
    function helper(): void {}
    const LIMIT = 10;
}

namespace App\Models;

use App\Contracts\{Renderable as ViewContract};
use App\Support\{Logs, function helper as assist, const LIMIT};

#[\Attribute]
class Marker {}

#[Marker]
enum State: string { case Ready = 'ready'; }

class Input {}
class Output {}
class Repository { public function save(): void {} }

class Service implements ViewContract
{
    use Logs;
    public const FLAG = 'flag';

    public function __construct(private Repository $repository) {}

    #[Marker]
    public function Render(Input $input): Output
    {
        $local = new Repository();
        self::Boot();
        $this->record();
        $local->save();
        assist();
        return new Output();
    }

    public static function Boot(): void {}
}

function top(): \Closure
{
    return function (int $value): int { return $value; };
}
"#;
    let extraction = Engine::default().extract_source_combined(
        Path::new("/checkout/app/Models/Service.php"),
        "app/Models/Service.php",
        source,
    )?;
    assert_eq!(extraction.graph.error, None, "{:#?}", extraction.graph);
    assert!(
        extraction.graph.nodes.is_empty(),
        "nodes={:#?}; evidence={:#?}",
        extraction.graph.nodes,
        extraction.graph.semantic_evidence
    );
    assert!(
        extraction.graph.edges.is_empty(),
        "edges={:#?}",
        extraction.graph.edges
    );
    assert!(extraction.graph.raw_calls.is_none());
    let evidence = extraction
        .graph
        .semantic_evidence
        .as_ref()
        .ok_or("missing PHP universal evidence")?;
    validate_evidence(evidence, EvidenceLimits::default())?;
    assert_eq!(evidence.adapter.id, "compass.php");
    assert_eq!(evidence.adapter.version, 1);
    assert_eq!(
        evidence.adapter.profile,
        UniversalAdapterProfile::UniversalCandidate
    );
    for capability in [
        LanguageCapability::Namespaces,
        LanguageCapability::Traits,
        LanguageCapability::Imports,
        LanguageCapability::Aliases,
        LanguageCapability::Calls,
        LanguageCapability::Construction,
        LanguageCapability::Decorators,
        LanguageCapability::BaseTypes,
        LanguageCapability::HierarchyDispatch,
        LanguageCapability::Members,
        LanguageCapability::Receivers,
    ] {
        assert!(evidence.adapter.capabilities.contains(&capability));
    }

    for (qualified_name, kind, source_name) in [
        ("app\\contracts\\renderable", "interface", "Renderable"),
        ("app\\support\\logs", "trait", "Logs"),
        ("app\\support\\helper", "function", "helper"),
        ("app\\models\\marker", "class", "Marker"),
        ("app\\models\\state", "enum", "State"),
        ("app\\models\\service", "class", "Service"),
        ("app\\models\\service::render", "method", "Render"),
        (
            "app\\models\\service::$repository",
            "property",
            "repository",
        ),
        ("App\\Support\\LIMIT", "constant", "LIMIT"),
        ("app\\models\\service::FLAG", "constant", "FLAG"),
    ] {
        assert!(
            evidence.declarations.iter().any(|declaration| {
                declaration.qualified_name == qualified_name
                    && declaration.kind == kind
                    && declaration.name == source_name
            }),
            "missing {kind} {qualified_name}: {:#?}",
            evidence.declarations
        );
    }
    assert!(
        evidence.declarations.iter().any(|declaration| {
            declaration.kind == "closure"
                && declaration
                    .qualified_name
                    .starts_with("app\\models\\top::closure@")
        }),
        "missing closure: {:#?}",
        evidence.declarations
    );
    let render = evidence
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name == "app\\models\\service::render")
        .ok_or("missing render declaration")?;
    assert_eq!(render.parameter_count, Some(1));
    assert_eq!(render.parameter_types, ["app\\models\\input"]);
    let models_namespace = evidence
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name == "app\\models")
        .ok_or("missing App\\Models namespace")?;
    let service = evidence
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name == "app\\models\\service")
        .ok_or("missing service declaration")?;
    assert!(evidence.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Owns
            && candidate.source_declaration_id == models_namespace.id
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(service.id.as_str())
    }));

    for (kind, spelling, target) in [
        (
            BindingKind::ImportAlias,
            "viewcontract",
            "app\\contracts\\renderable",
        ),
        (BindingKind::Import, "logs", "app\\support\\logs"),
        (BindingKind::ImportAlias, "assist", "app\\support\\helper"),
        (BindingKind::Import, "LIMIT", "App\\Support\\LIMIT"),
    ] {
        assert!(
            evidence.bindings.iter().any(|binding| {
                binding.kind == kind
                    && binding.spelling == spelling
                    && binding.qualified_target == target
            }),
            "missing binding {spelling} -> {target}: {:#?}",
            evidence.bindings
        );
    }
    for relation in [
        CandidateRelation::Calls,
        CandidateRelation::Constructs,
        CandidateRelation::Decorates,
        CandidateRelation::Implements,
        CandidateRelation::UsesTrait,
        CandidateRelation::References,
        CandidateRelation::Returns,
        CandidateRelation::Owns,
    ] {
        assert!(
            evidence
                .candidates
                .iter()
                .any(|candidate| candidate.relation == relation),
            "missing {relation:?}: {:#?}",
            evidence.candidates
        );
    }
    assert!(evidence.occurrences.iter().any(|occurrence| {
        occurrence.role == SemanticRole::Call
            && occurrence.spelling == "assist"
            && occurrence.qualifier.as_deref() == Some("app\\support\\helper")
    }));
    for occurrence in &evidence.occurrences {
        let start = usize::try_from(occurrence.range.start_byte)?;
        let end = usize::try_from(occurrence.range.end_byte)?;
        assert!(start < end && end <= source.len());
    }
    Ok(())
}

#[test]
fn php_profile_is_registered_in_sorted_universal_registry() -> Result<(), Box<dyn Error>> {
    AdapterRegistry::validate()?;
    let profile = AdapterRegistry::universal_profile("php").ok_or("missing PHP profile")?;
    assert_eq!(profile.profile, UniversalAdapterProfile::UniversalCandidate);
    Ok(())
}

#[test]
fn php_evidence_is_checkout_independent_deterministic_and_recovery_bounded()
-> Result<(), Box<dyn Error>> {
    let source = br#"<?php
namespace MixedCase;
class Widget { public function Run(): void {} }
class WIDGET { public function run(): void {} }
function invoke(Widget $widget): void { $widget->RUN(); }
"#;
    let first = Engine::default().extract_source_combined(
        Path::new("/first/checkout/src/Widget.php"),
        "src/Widget.php",
        source,
    )?;
    let second = Engine::default().extract_source_combined(
        Path::new("/different/checkout/src/Widget.php"),
        "src/Widget.php",
        source,
    )?;
    assert_eq!(
        first.graph.semantic_evidence,
        second.graph.semantic_evidence
    );
    let evidence = first
        .graph
        .semantic_evidence
        .as_ref()
        .ok_or("missing deterministic PHP evidence")?;
    assert_eq!(
        evidence
            .declarations
            .iter()
            .filter(|declaration| declaration.qualified_name == "mixedcase\\widget")
            .count(),
        2
    );

    let malformed = Engine::default().extract_source_combined(
        Path::new("/checkout/src/Broken.php"),
        "src/Broken.php",
        b"<?php class Broken { public function unfinished(",
    )?;
    let malformed_evidence = malformed
        .graph
        .semantic_evidence
        .as_ref()
        .ok_or("missing recovered PHP evidence")?;
    validate_evidence(malformed_evidence, EvidenceLimits::default())?;
    assert!(
        malformed_evidence
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "partial_parser_recovery")
    );

    let empty = Engine::default().extract_source_combined(
        Path::new("/checkout/src/Empty.php"),
        "src/Empty.php",
        b"",
    )?;
    let empty_evidence = empty
        .graph
        .semantic_evidence
        .as_ref()
        .ok_or("missing empty PHP evidence batch")?;
    validate_evidence(empty_evidence, EvidenceLimits::default())?;
    assert_eq!(empty_evidence.declarations.len(), 1);
    assert_eq!(empty_evidence.declarations[0].kind, "file");
    assert_eq!(empty_evidence.scopes.len(), 1);
    assert_eq!(empty_evidence.scopes[0].kind, "module");
    Ok(())
}

#[test]
fn laravel_and_drupal_source_packs_consume_php_universal_evidence() -> Result<(), Box<dyn Error>> {
    let laravel_source = br#"<?php
use Illuminate\Support\Facades\Route;
use App\Http\Controllers\UserController;
class LocalController { public function Update(): void {} }

Route::get('/users', [UserController::class, 'index']);
Route::put('/local', 'LocalController@Update');
Route::prefix('api')->group(function (): void {
    Route::post('/users', [UserController::class, 'store']);
});
Other::get('/not-a-route', [UserController::class, 'ignored']);
"#;
    let laravel = Engine::default().extract_source_combined(
        Path::new("/checkout/routes/web.php"),
        "routes/web.php",
        laravel_source,
    )?;
    assert!(laravel.graph.semantic_evidence.is_some());
    let routes = laravel
        .graph
        .framework_facts
        .iter()
        .filter_map(|fact| match fact {
            RawFrameworkFact::Route(route) if route.framework == "laravel" => Some(route),
            RawFrameworkFact::Route(_)
            | RawFrameworkFact::Domain(_)
            | RawFrameworkFact::Annotation(_) => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        routes.len(),
        3,
        "facts={:#?}",
        laravel.graph.framework_facts
    );
    assert!(routes.iter().any(|route| {
        route.operation == "GET"
            && route.normalized_path == "/users"
            && route.handler_reference == "app.http.controllers.usercontroller.index"
            && route.origin == RawFrameworkOrigin::Ast
    }));
    assert!(routes.iter().any(|route| {
        route.operation == "POST"
            && route.normalized_path == "/api/users"
            && route.handler_reference == "app.http.controllers.usercontroller.store"
    }));
    assert!(routes.iter().any(|route| {
        route.operation == "PUT"
            && route.normalized_path == "/local"
            && route.handler_reference == "localcontroller.update"
    }));

    let drupal_source = br#"<?php
/** Implements hook_form_alter(). */
function demo_form_alter(array &$form): void {}
function hook_help(): void {}
"#;
    let drupal = Engine::default().extract_source_combined(
        Path::new("/checkout/modules/demo/demo.module"),
        "modules/demo/demo.module",
        drupal_source,
    )?;
    assert!(drupal.graph.semantic_evidence.is_some());
    let hooks = drupal
        .graph
        .framework_facts
        .iter()
        .filter_map(|fact| match fact {
            RawFrameworkFact::Route(route) if route.framework == "drupal" => Some(route),
            RawFrameworkFact::Route(_)
            | RawFrameworkFact::Domain(_)
            | RawFrameworkFact::Annotation(_) => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(hooks.len(), 2, "facts={:#?}", drupal.graph.framework_facts);
    assert!(hooks.iter().all(|hook| {
        hook.origin == RawFrameworkOrigin::Ast
            && hook
                .detail
                .get("declarationId")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|id| id.starts_with("declaration:"))
    }));
    Ok(())
}

#[test]
fn drupal_config_and_blade_template_extractors_remain_available() -> Result<(), Box<dyn Error>> {
    let config = Engine::default().extract_source(
        Path::new("demo.routing.yml"),
        br#"demo.page:
  path: '/demo'
  defaults:
    _controller: '\\Drupal\\demo\\Controller\\DemoController::page'
  requirements:
    _method: 'GET|POST'
"#,
    )?;
    assert_eq!(
        config
            .framework_facts
            .iter()
            .filter(|fact| matches!(fact, RawFrameworkFact::Route(route) if route.framework == "drupal" && route.origin == RawFrameworkOrigin::Config))
            .count(),
        2
    );

    let directory = tempfile::tempdir()?;
    let blade = directory.path().join("welcome.blade.php");
    std::fs::write(&blade, "<h1>{{ $title }}</h1>")?;
    let extraction = Engine::default().extract(&blade)?;
    assert_eq!(extraction.error, None);
    assert!(!extraction.nodes.is_empty());
    Ok(())
}
