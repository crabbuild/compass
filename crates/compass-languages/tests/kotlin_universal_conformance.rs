use std::path::Path;

use compass_languages::{CandidateRelation, Engine, SemanticRole, UniversalAdapterProfile};

const SOURCE: &[u8] = br#"
package demo.api

import other.Service as OtherService
import other.helpers.runTask
import org.springframework.web.bind.annotation.GetMapping

annotation class Audit(val value: String)
private interface Marker
open class Base<T>

@Audit("controller")
class Controller<T : Any>(private val service: OtherService, count: Int = 1) : Base<T>(), Marker {
    companion object Factory {
        const val NAME: String = "demo"
        fun create(): Controller<String> = Controller(OtherService())
    }

    @GetMapping(path = ["/x"])
    fun String.render(prefix: String = "x", vararg ids: Long?): String? {
        service.run(prefix = prefix, count = ids.size)
        runTask(this)
        return this
    }
}

object Singleton
typealias Alias = Controller<String>
"#;

#[test]
fn kotlin_emits_modern_universal_evidence_with_exact_anchors()
-> Result<(), Box<dyn std::error::Error>> {
    let extraction = Engine::default().extract_source_graph_only(
        Path::new("src/main/kotlin/demo/api/Controller.kt"),
        "src/main/kotlin/demo/api/Controller.kt",
        SOURCE,
    )?;
    assert!(extraction.nodes.is_empty());
    assert!(extraction.edges.is_empty());
    assert!(extraction.raw_calls.is_none());
    let evidence = extraction
        .semantic_evidence
        .as_ref()
        .ok_or_else(|| format!("missing Kotlin evidence: {:?}", extraction.error))?;
    let qualification = Engine::default().extract_source_universal_candidate_evidence(
        Path::new("src/main/kotlin/demo/api/Controller.kt"),
        "src/main/kotlin/demo/api/Controller.kt",
        SOURCE,
    )?;
    assert_eq!(evidence, &qualification);
    assert_eq!(evidence.adapter.language, "kotlin");
    assert_eq!(evidence.adapter.version, 1);
    assert_eq!(
        evidence.adapter.profile,
        UniversalAdapterProfile::UniversalCandidate
    );
    for (kind, qualified) in [
        ("annotation_type", "demo.api.Audit"),
        ("interface", "demo.api.Marker"),
        ("class", "demo.api.Base"),
        ("class", "demo.api.Controller"),
        ("companion_object", "demo.api.Controller.Factory"),
        ("constant", "demo.api.Controller.Factory::NAME"),
        ("method", "demo.api.Controller::render"),
        ("object", "demo.api.Singleton"),
        ("type_alias", "demo.api.Alias"),
    ] {
        assert!(
            evidence
                .declarations
                .iter()
                .any(|declaration| declaration.kind == kind
                    && declaration.qualified_name == qualified),
            "missing {kind} {qualified}; declarations={:#?}",
            evidence.declarations
        );
    }
    assert!(evidence.bindings.iter().any(|binding| {
        binding.spelling == "OtherService" && binding.qualified_target == "other.Service"
    }));
    assert!(evidence.occurrences.iter().any(|occurrence| {
        occurrence.role == SemanticRole::Annotation
            && occurrence.spelling == "GetMapping"
            && slice(&occurrence.range, SOURCE) == "GetMapping"
    }));
    assert!(evidence.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Extends
            && candidate.constraints.qualified_name.as_deref() == Some("demo.api.Base")
    }));
    assert!(evidence.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Implements
            && candidate.constraints.qualified_name.as_deref() == Some("demo.api.Marker")
    }));
    let named_call = evidence
        .occurrences
        .iter()
        .find(|occurrence| occurrence.role == SemanticRole::Call && occurrence.spelling == "run")
        .ok_or("missing named member call")?;
    assert_eq!(
        named_call.context.as_deref(),
        Some("kotlin_args:prefix,count")
    );
    assert_eq!(slice(&named_call.range, SOURCE), "run");
    assert!(
        evidence
            .candidates
            .iter()
            .all(|candidate| { candidate.constraints.exact_language.as_deref() == Some("kotlin") })
    );
    assert!(
        evidence
            .declarations
            .iter()
            .all(|declaration| declaration.language == "kotlin")
    );
    assert!(
        evidence
            .candidates
            .iter()
            .filter(|candidate| {
                matches!(
                    candidate.relation,
                    CandidateRelation::Calls | CandidateRelation::Constructs
                )
            })
            .all(|candidate| {
                evidence.declarations.iter().any(|declaration| {
                    declaration.id == candidate.source_declaration_id
                        && matches!(
                            declaration.kind.as_str(),
                            "constructor" | "function" | "method"
                        )
                })
            })
    );
    Ok(())
}

#[test]
fn kotlin_evidence_is_deterministic_and_malformed_input_is_bounded()
-> Result<(), Box<dyn std::error::Error>> {
    let path = Path::new("src/Unicode.kt");
    let source =
        "package δοκιμή\nclass Café { fun привет(value: String?) = value?.length }\n".as_bytes();
    let first = Engine::default().extract_source_graph_only(path, "src/Unicode.kt", source)?;
    let second = Engine::default().extract_source_graph_only(path, "src/Unicode.kt", source)?;
    assert_eq!(first.semantic_evidence, second.semantic_evidence);

    let malformed = b"package demo\nclass Broken( { fun call( = target(\n";
    let extraction = Engine::default().extract_source_graph_only(
        Path::new("src/Broken.kt"),
        "src/Broken.kt",
        malformed,
    )?;
    let evidence = extraction
        .semantic_evidence
        .ok_or("missing malformed evidence")?;
    assert!(
        evidence
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "partial_parser_recovery")
    );
    assert!(evidence.occurrences.iter().all(|occurrence| {
        occurrence.range.end_byte >= occurrence.range.start_byte
            && usize::try_from(occurrence.range.end_byte).is_ok_and(|end| end <= malformed.len())
    }));
    Ok(())
}

#[test]
fn kotlin_traversal_limit_is_reported_without_unbounded_walk()
-> Result<(), Box<dyn std::error::Error>> {
    let source = format!(
        "package demo\nfun nested() = {}1{}\n",
        "target(".repeat(600),
        ")".repeat(600)
    );
    let extraction = Engine::default().extract_source_graph_only(
        Path::new("src/Deep.kt"),
        "src/Deep.kt",
        source.as_bytes(),
    )?;
    let evidence = extraction
        .semantic_evidence
        .ok_or("missing bounded evidence")?;
    assert!(
        evidence
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "kotlin_traversal_limit")
    );
    Ok(())
}

fn slice<'a>(range: &compass_languages::EvidenceRange, source: &'a [u8]) -> &'a str {
    let start = usize::try_from(range.start_byte).unwrap_or_default();
    let end = usize::try_from(range.end_byte).unwrap_or_default();
    std::str::from_utf8(source.get(start..end).unwrap_or_default()).unwrap_or_default()
}
