use std::error::Error;
use std::path::Path;

use compass_languages::{
    BindingKind, CandidateRelation, Engine, EvidenceLimits, LanguageCapability, SemanticRole,
    UniversalEvidenceQualification, UniversalEvidenceRegistry, validate_evidence,
};

#[test]
fn csharp_emits_bounded_full_language_evidence_without_a_replaced_raw_graph()
-> Result<(), Box<dyn Error>> {
    let source = br#"global using Text = System.String;
using Microsoft.AspNetCore.Mvc;
using Demo.Data;

namespace Demo.Api;

public interface IWorker { Result Run(Input input); }
public record Input(string Value);
public record Result(string Value);

[ApiController]
[Route("api/[controller]")]
public partial class UsersController : ControllerBase, IWorker
{
    public const int Limit = 4;
    private readonly Repository repository;
    public Repository Repository { get; init; }

    public UsersController(Repository repository) { this.repository = repository; }

    [HttpGet("{id}")]
    public Result Run(Input input)
    {
        Repository local = new Repository();
        local.Load(input.Value);
        return new Result(input.Value);
    }

    public Result Run(Input input, int limit) => Run(input);
}
"#;
    let extraction = Engine::default().extract_source_combined(
        Path::new("/repo/src/Demo.Api/UsersController.cs"),
        "src/Demo.Api/UsersController.cs",
        source,
    )?;
    assert_eq!(
        extraction.graph.error, None,
        "graph={:#?}",
        extraction.graph
    );
    assert!(extraction.graph.nodes.is_empty());
    assert!(extraction.graph.edges.is_empty());
    assert!(
        extraction.graph.raw_calls.is_none(),
        "graph={:#?}",
        extraction.graph
    );
    let evidence = extraction
        .graph
        .semantic_evidence
        .as_ref()
        .ok_or("missing C# universal evidence")?;
    validate_evidence(evidence, EvidenceLimits::default())?;
    assert_eq!(evidence.pipeline.id, "compass.csharp");
    assert_eq!(evidence.pipeline.version, 1);
    assert_eq!(
        evidence.pipeline.qualification,
        UniversalEvidenceQualification::Qualifying
    );
    for capability in [
        LanguageCapability::Namespaces,
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
        assert!(evidence.pipeline.capabilities.contains(&capability));
    }
    let controller = evidence
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name == "Demo.Api.UsersController")
        .ok_or_else(|| format!("missing controller: {:#?}", evidence.declarations))?;
    assert_eq!(controller.kind, "class");
    assert!(controller.direct_bases_complete);
    for kind in ["constructor", "property", "field", "constant", "parameter"] {
        assert!(
            evidence
                .declarations
                .iter()
                .any(|declaration| declaration.kind == kind),
            "missing {kind}: {:#?}",
            evidence.declarations
        );
    }
    let overloads = evidence
        .declarations
        .iter()
        .filter(|declaration| declaration.qualified_name == "Demo.Api.UsersController::Run")
        .collect::<Vec<_>>();
    assert_eq!(overloads.len(), 2);
    assert_eq!(
        overloads
            .iter()
            .map(|declaration| declaration.parameter_count)
            .collect::<std::collections::BTreeSet<_>>(),
        [Some(1), Some(2)].into_iter().collect()
    );
    assert!(evidence.bindings.iter().any(|binding| {
        binding.kind == BindingKind::ImportAlias
            && binding.spelling == "Text"
            && binding.qualified_target == "System.String"
    }));
    assert!(evidence.occurrences.iter().any(|occurrence| {
        occurrence.role == SemanticRole::Annotation && occurrence.spelling == "HttpGet"
    }));
    for relation in [
        CandidateRelation::Calls,
        CandidateRelation::Constructs,
        CandidateRelation::Annotates,
        CandidateRelation::Extends,
        CandidateRelation::TypeOf,
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
    for occurrence in &evidence.occurrences {
        let start = usize::try_from(occurrence.range.start_byte)?;
        let end = usize::try_from(occurrence.range.end_byte)?;
        assert!(start < end && end <= source.len());
    }
    Ok(())
}

#[test]
fn csharp_evidence_is_deterministic_and_parser_recovery_is_explicit() -> Result<(), Box<dyn Error>>
{
    let source = b"namespace Demo; class Broken { void Run() { Missing() } }";
    let mut engine = Engine::default();
    let first = engine.extract_source(Path::new("Broken.cs"), source)?;
    let second = engine.extract_source(Path::new("Broken.cs"), source)?;
    assert_eq!(first.semantic_evidence, second.semantic_evidence);
    assert_eq!(first.error, None, "extraction={first:#?}");
    let evidence = first.semantic_evidence.ok_or("missing C# evidence")?;
    assert!(evidence.diagnostics.iter().any(|diagnostic| {
        matches!(
            diagnostic.code.as_str(),
            "parser_error" | "partial_parser_recovery"
        )
    }));
    Ok(())
}

#[test]
fn csharp_missing_tokens_publish_bounded_partial_evidence() -> Result<(), Box<dyn Error>> {
    for source in [
        b"namespace Demo; class Broken { void Run(".as_slice(),
        b"namespace Demo; class Broken :".as_slice(),
        b"namespace Demo; class Broken { string Value =>".as_slice(),
        b"namespace Demo; public union Choice(int, string);".as_slice(),
    ] {
        let extraction = Engine::default().extract_source(Path::new("Broken.cs"), source)?;
        assert_eq!(extraction.error, None, "extraction={extraction:#?}");
        let evidence = extraction
            .semantic_evidence
            .as_ref()
            .ok_or("missing bounded partial C# evidence")?;
        assert!(!evidence.declarations.is_empty());
        assert!(evidence.diagnostics.iter().any(|diagnostic| {
            matches!(
                diagnostic.code.as_str(),
                "parser_error" | "partial_parser_recovery"
            )
        }));
        assert!(evidence.diagnostics.iter().all(|diagnostic| {
            diagnostic.range.as_ref().is_none_or(|range| {
                range.start_byte < range.end_byte
                    && usize::try_from(range.end_byte).is_ok_and(|end| end <= source.len())
            })
        }));
    }
    Ok(())
}

#[test]
fn csharp_evidence_identity_is_independent_of_checkout_root() -> Result<(), Box<dyn Error>> {
    let source = b"using Microsoft.AspNetCore.Mvc; class Controller { [HttpGet] void Run() {} }";
    let first = Engine::default().extract_source_combined(
        Path::new("/checkout-a/src/Controller.cs"),
        "src/Controller.cs",
        source,
    )?;
    let second = Engine::default().extract_source_combined(
        Path::new("/checkout-b/src/Controller.cs"),
        "src/Controller.cs",
        source,
    )?;
    assert_eq!(
        first.graph.semantic_evidence, second.graph.semantic_evidence,
        "C# evidence identities must derive from the portable source identity"
    );
    Ok(())
}

#[test]
fn csharp_pipeline_is_registered_as_qualifying() -> Result<(), Box<dyn Error>> {
    let pipeline = UniversalEvidenceRegistry::pipeline("csharp").ok_or("missing C# pipeline")?;
    assert_eq!(
        pipeline.qualification,
        UniversalEvidenceQualification::Qualifying
    );
    Ok(())
}
