use std::error::Error;
use std::path::Path;

use compass_languages::{
    CandidateRelation, Engine, EvidenceLimits, Registry, SemanticRole, UniversalEvidenceRegistry,
    validate_evidence,
};

#[test]
fn extended_languages_publish_ast_first_universal_evidence() -> Result<(), Box<dyn Error>> {
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
fn registry_fixtures_keep_extended_pipelines_valid() -> Result<(), Box<dyn Error>> {
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
fn extended_empty_and_recovered_sources_remain_bounded() -> Result<(), Box<dyn Error>> {
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
