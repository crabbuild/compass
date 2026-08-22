use std::error::Error;
use std::path::Path;

use compass_languages::{
    BindingKind, CandidateRelation, Engine, EvidenceLimits, LanguageCapability, Registry,
    SemanticRole, UniversalEvidenceRegistry, validate_evidence,
};

#[test]
fn language_wave_publish_ast_first_universal_evidence() -> Result<(), Box<dyn Error>> {
    let fixtures = [
        (
            "Sources/Greeter.swift",
            br#"import Foundation
protocol Renderable { func render() }
struct Greeter: Renderable {
    func render() { helper() }
    func helper() {}
}
"#
            .as_slice(),
            "swift",
            "Renderable",
            "helper",
        ),
        (
            "lib/greeter.dart",
            br#"import 'package:flutter/widgets.dart' as widgets;
class Greeter {
  Widget build() { return helper(); }
  Widget helper() => const Widget();
}
"#
            .as_slice(),
            "dart",
            "Greeter",
            "helper",
        ),
        (
            "src/Greeter.scala",
            br#"package sample
trait Renderable { def render(): Unit }
class Greeter extends Renderable {
  def render(): Unit = helper()
  def helper(): Unit = ()
}
"#
            .as_slice(),
            "scala",
            "Greeter",
            "helper",
        ),
        (
            "src/Greeter.groovy",
            br#"package sample
import java.util.List
class Greeter {
  void render() { helper() }
  void helper() {}
}
"#
            .as_slice(),
            "groovy",
            "Greeter",
            "helper",
        ),
    ];

    for (path, source, language, type_name, call_name) in fixtures {
        let mut engine = Engine::default();
        let evidence = engine.extract_source_universal_evidence(Path::new(path), path, source)?;
        validate_evidence(&evidence, EvidenceLimits::default())?;
        assert_eq!(evidence.pipeline.language, language);
        let pipeline = UniversalEvidenceRegistry::pipeline(language)
            .ok_or_else(|| format!("missing universal pipeline for {language}"))?;
        assert_eq!(evidence.pipeline.qualification, pipeline.qualification);
        assert!(
            evidence
                .declarations
                .iter()
                .any(|decl| decl.name == type_name),
            "{language}: {:#?}",
            evidence.declarations
        );
        assert!(
            evidence
                .declarations
                .iter()
                .any(|decl| decl.name == call_name),
            "{language}: {:#?}",
            evidence.declarations
        );
        assert!(
            evidence
                .occurrences
                .iter()
                .any(|occurrence| occurrence.role == SemanticRole::Call
                    && occurrence.spelling == call_name),
            "{language}: {:#?}",
            evidence.occurrences
        );
        assert!(
            evidence
                .candidates
                .iter()
                .any(|candidate| candidate.relation == CandidateRelation::Calls
                    && candidate.target_spelling == call_name),
            "{language}: {:#?}",
            evidence.candidates
        );
        assert!(
            evidence.candidates.iter().all(|candidate| candidate
                .constraints
                .exact_language
                .as_deref()
                == Some(language)),
            "{language}: cross-language candidate"
        );
    }
    Ok(())
}

#[test]
fn registry_fixtures_keep_language_wave_pipelines_valid() -> Result<(), Box<dyn Error>> {
    for case in Registry::cases()
        .iter()
        .filter(|case| matches!(case.spec.name, "swift" | "dart" | "scala" | "groovy"))
    {
        let mut engine = Engine::default();
        let evidence = engine.extract_source_universal_evidence(
            Path::new(case.fixture_path),
            case.fixture_path,
            case.fixture_source.as_bytes(),
        )?;
        validate_evidence(&evidence, EvidenceLimits::default())?;
        assert_eq!(evidence.pipeline.language, case.spec.name);
        assert!(!evidence.declarations.is_empty(), "{}", case.id);
    }
    Ok(())
}

#[test]
fn language_wave_empty_and_recovered_sources_remain_bounded() -> Result<(), Box<dyn Error>> {
    for (path, source) in [
        ("empty.swift", b"".as_slice()),
        ("empty.dart", b"\n".as_slice()),
        ("empty.scala", b"/* unterminated".as_slice()),
        ("empty.groovy", b"class Broken {".as_slice()),
    ] {
        let mut engine = Engine::default();
        let evidence = engine.extract_source_universal_evidence(Path::new(path), path, source)?;
        validate_evidence(&evidence, EvidenceLimits::default())?;
        assert_eq!(
            evidence.pipeline.language,
            Path::new(path)
                .extension()
                .and_then(|ext| ext.to_str())
                .unwrap_or_default()
        );
    }
    for (path, invalid_source) in [
        ("invalid.swift", b"struct Broken {\n  \xff\n}".as_slice()),
        ("invalid.dart", b"class Broken {\n  \xff\n}".as_slice()),
        ("invalid.scala", b"class Broken {\n  \xff\n}".as_slice()),
        ("invalid.groovy", b"class Broken {\n  \xff\n}".as_slice()),
    ] {
        let mut engine = Engine::default();
        let evidence =
            engine.extract_source_universal_evidence(Path::new(path), path, invalid_source)?;
        validate_evidence(&evidence, EvidenceLimits::default())?;
        assert!(
            evidence
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "invalid_utf8"),
            "{path}: {:#?}",
            evidence.diagnostics
        );
        assert!(
            evidence
                .declarations
                .iter()
                .all(|declaration| declaration.range.end_byte <= invalid_source.len() as u64),
            "{path}: {:#?}",
            evidence.declarations
        );
    }
    Ok(())
}

#[test]
fn groovy_spock_quoted_features_are_source_bounded_methods() -> Result<(), Box<dyn Error>> {
    let source = br#"package routes
class UserSpec extends spock.lang.Specification {
    def "loads users"() {
        helper()
    }
}
"#;
    let path = Path::new("src/UserSpec.groovy");
    let mut engine = Engine::default();
    let evidence = engine.extract_source_universal_evidence(path, "src/UserSpec.groovy", source)?;
    validate_evidence(&evidence, EvidenceLimits::default())?;

    let feature = evidence
        .declarations
        .iter()
        .find(|declaration| declaration.name == "loads users")
        .ok_or("missing quoted Spock feature")?;
    assert_eq!(feature.kind, "method");
    assert_eq!(feature.qualified_name, "routes.UserSpec.loads users");
    let range = &feature.range;
    let text = std::str::from_utf8(
        source
            .get(range.start_byte as usize..range.end_byte as usize)
            .ok_or("feature range outside source")?,
    )?;
    assert!(text.contains("def \"loads users\"()"));
    assert!(
        !evidence
            .pipeline
            .capabilities
            .contains(&LanguageCapability::Tests)
    );
    Ok(())
}

#[test]
fn dart_library_parts_and_import_filters_remain_explicit() -> Result<(), Box<dyn Error>> {
    let source = br#"library foo.bar;
part 'src/generated.dart';
import 'package:widgets/widgets.dart' deferred as widgets show Widget, Api hide Internal;
export 'src/api.dart' show Api hide Internal;
class Screen {
    void render() { widgets.Widget(); }
}
"#;
    let path = Path::new("lib/foo.dart");
    let mut engine = Engine::default();
    let evidence = engine.extract_source_universal_evidence(path, "lib/foo.dart", source)?;
    validate_evidence(&evidence, EvidenceLimits::default())?;

    assert!(evidence.declarations.iter().any(|declaration| {
        declaration.kind == "namespace" && declaration.qualified_name == "foo.bar"
    }));
    assert!(evidence.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Embeds
            && candidate.target_spelling == "src/generated.dart"
    }));
    assert!(evidence.bindings.iter().any(|binding| {
        binding.kind == BindingKind::ImportAlias
            && binding.spelling == "widgets.Widget"
            && binding.qualified_target == "package:widgets/widgets.dart.Widget"
    }));
    assert!(evidence.bindings.iter().any(|binding| {
        binding.kind == BindingKind::Reexport
            && binding.spelling == "Api"
            && binding.qualified_target == "src/api.dart.Api"
    }));
    assert!(
        !evidence
            .bindings
            .iter()
            .any(|binding| binding.spelling.contains("Internal"))
    );

    let part_source = br#"part of foo.bar;
class Generated {}
"#;
    let part_path = Path::new("lib/src/generated.dart");
    let part = engine.extract_source_universal_evidence(
        part_path,
        "lib/src/generated.dart",
        part_source,
    )?;
    validate_evidence(&part, EvidenceLimits::default())?;
    assert!(part.declarations.iter().any(|declaration| {
        declaration.name == "Generated" && declaration.qualified_name == "foo.bar.Generated"
    }));
    Ok(())
}

#[test]
fn swift_nominal_and_extension_identities_remain_distinct() -> Result<(), Box<dyn Error>> {
    let source = br#"import Foundation
protocol Renderable {}
class Box: Renderable {}
struct Widget {}
enum State { case ready, done }
extension Box { func render() {} }
typealias Alias = Box
"#;
    let path = Path::new("Sources/Models.swift");
    let mut engine = Engine::default();
    let evidence =
        engine.extract_source_universal_evidence(path, "Sources/Models.swift", source)?;
    validate_evidence(&evidence, EvidenceLimits::default())?;
    for (name, kind) in [
        ("Renderable", "protocol"),
        ("Box", "class"),
        ("Widget", "struct"),
        ("State", "enum"),
        ("Box", "extension"),
        ("Alias", "type_alias"),
    ] {
        assert!(
            evidence
                .declarations
                .iter()
                .any(|declaration| declaration.name == name && declaration.kind == kind)
        );
    }
    assert!(evidence.declarations.iter().any(|declaration| {
        declaration.name == "render"
            && declaration.kind == "method"
            && declaration.qualified_name == "Box.render"
    }));
    Ok(())
}

#[test]
fn scala_companion_and_selector_identities_do_not_collapse() -> Result<(), Box<dyn Error>> {
    let source = br#"package sample
import foo.{Bar => Baz, Hidden => _, _}
class Box {}
object Box {}
"#;
    let path = Path::new("src/Models.scala");
    let mut engine = Engine::default();
    let evidence = engine.extract_source_universal_evidence(path, "src/Models.scala", source)?;
    validate_evidence(&evidence, EvidenceLimits::default())?;

    let boxes = evidence
        .declarations
        .iter()
        .filter(|declaration| declaration.name == "Box")
        .collect::<Vec<_>>();
    assert!(boxes.iter().any(|declaration| declaration.kind == "class"));
    assert!(boxes.iter().any(|declaration| declaration.kind == "module"));
    assert_ne!(boxes[0].id, boxes[1].id);
    assert!(evidence.bindings.iter().any(|binding| {
        binding.kind == BindingKind::ImportAlias
            && binding.spelling == "Baz"
            && binding.qualified_target == "foo.Bar"
    }));
    assert!(
        evidence
            .bindings
            .iter()
            .any(|binding| { binding.spelling == "foo.*" && binding.qualified_target == "foo" })
    );
    assert!(
        !evidence
            .bindings
            .iter()
            .any(|binding| binding.spelling == "Hidden")
    );
    Ok(())
}
