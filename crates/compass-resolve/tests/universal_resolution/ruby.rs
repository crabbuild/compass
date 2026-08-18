use compass_resolve::evidence::{ResolutionDecision, ResolutionRule};

fn extract_ruby(path: &str, source: &[u8]) -> compass_languages::SemanticEvidenceBatch {
    Engine::default()
        .extract_source_universal_candidate_evidence(Path::new(path), path, source)
        .expect("Ruby universal evidence")
}

fn resolve_ruby(
    batches: &[compass_languages::SemanticEvidenceBatch],
) -> Vec<(CandidateRelation, ResolutionDecision)> {
    let index = UniversalResolutionIndex::new(batches, UniversalResolutionLimits::default())
        .expect("Ruby resolver index");
    batches
        .iter()
        .flat_map(|batch| {
            batch
                .candidates
                .iter()
                .map(|candidate| (candidate.relation, index.resolve(&candidate.id)))
        })
        .collect()
}

#[test]
fn ruby_resolution_is_qualified_and_method_space_aware() {
    let source = br#"module Billing
  module Auditable
  end
  class Document
    def save; end
  end
  class Invoice < Document
    include Auditable
    def save
      super
    end
    def run
      save
    end
    def self.build
      new
    end
  end
end
"#;
    let batch = extract_ruby("billing.rb", source);
    let decisions = resolve_ruby(std::slice::from_ref(&batch));
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::UsesTrait
            && matches!(
                decisions
                    .iter()
                    .find(|(relation, _)| *relation == CandidateRelation::UsesTrait)
                    .map(|(_, decision)| decision),
                Some(ResolutionDecision::Resolved { evidence, .. })
                    if evidence.rule == ResolutionRule::ExplicitBinding
            )
    }));
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Extends
            && matches!(
                decisions
                    .iter()
                    .find(|(relation, _)| *relation == CandidateRelation::Extends)
                    .map(|(_, decision)| decision),
                Some(ResolutionDecision::Resolved { evidence, .. })
                    if evidence.rule == ResolutionRule::ExactHierarchyBase
            )
    }));
    let calls = decisions
        .iter()
        .filter(|(relation, _)| *relation == CandidateRelation::Calls)
        .map(|(_, decision)| decision)
        .collect::<Vec<_>>();
    assert!(calls.iter().any(|decision| matches!(
        decision,
        ResolutionDecision::Resolved { evidence, .. }
            if evidence.rule == ResolutionRule::DirectReceiverSuccessorDispatch
                || evidence.rule == ResolutionRule::LinearizedReceiverDispatch
                || evidence.rule == ResolutionRule::MemberBinding
    )));
    assert!(batch.declarations.iter().any(|declaration| {
        declaration.qualified_name == "Billing::Invoice#save"
            && declaration.kind == "method"
    }));
    assert!(batch.declarations.iter().any(|declaration| {
        declaration.qualified_name == "Billing::Invoice.build"
            && declaration.kind == "method"
    }));
}

#[test]
fn ruby_duplicate_methods_are_ambiguous_and_cross_language_targets_are_rejected() {
    let first = extract_ruby("first.rb", b"class Example\n  def run; end\nend\n");
    let second = extract_ruby("second.rb", b"class Example\n  def run; end\nend\n");
    let caller = extract_ruby(
        "caller.rb",
        b"class Example\n  def call\n    run\n  end\nend\n",
    );
    let decisions = resolve_ruby(&[first, second, caller]);
    assert!(decisions.iter().any(|(relation, decision)| {
        *relation == CandidateRelation::Calls
            && matches!(decision, ResolutionDecision::Ambiguous { candidate_count } if *candidate_count >= 2)
    }));

    let python = Engine::default()
        .extract_source(
            Path::new("example.py"),
            b"class Example:\n    def run(self):\n        pass\n",
        )
        .expect("Python extraction")
        .semantic_evidence
        .expect("Python evidence");
    let ruby_caller = extract_ruby("ruby_caller.rb", b"def call; Example.run; end\n");
    let decisions = resolve_ruby(&[python, ruby_caller]);
    assert!(decisions.iter().all(|(relation, decision)| {
        *relation != CandidateRelation::Calls
            || !matches!(decision, ResolutionDecision::Resolved { .. })
    }));
}

#[test]
fn ruby_require_relative_resolves_only_to_an_exact_contained_source_file() {
    let imported = extract_ruby("lib/billing/document.rb", b"class Billing::Document; end\n");
    let importer = extract_ruby(
        "lib/billing/invoice.rb",
        b"require_relative \"document\"\nclass Billing::Invoice; end\n",
    );
    let decisions = resolve_ruby(&[imported, importer]);
    assert!(decisions.iter().any(|(relation, decision)| {
        *relation == CandidateRelation::Imports
            && matches!(decision, ResolutionDecision::ResolvedInventory { evidence, .. }
                if evidence.rule == ResolutionRule::ExactSourceInventory)
    }));

    let unrelated = extract_ruby("lib/other/document.rb", b"class Other::Document; end\n");
    let decisions = resolve_ruby(&[unrelated, extract_ruby(
        "lib/billing/invoice.rb",
        b"require_relative \"document\"\n",
    )]);
    assert!(decisions.iter().all(|(relation, decision)| {
        *relation != CandidateRelation::Imports
            || !matches!(decision, ResolutionDecision::ResolvedInventory { .. })
    }));
}

#[test]
fn ruby_relative_constants_resolve_across_files_by_lexical_owner() {
    let environment = extract_ruby(
        "lib/rubocop/cli/environment.rb",
        br#"module RuboCop
  class CLI
    class Environment
    end
  end
end
"#,
    );
    let caller = extract_ruby(
        "lib/rubocop/cli.rb",
        br#"module RuboCop
  class CLI
    def run
      Environment.new
    end
  end
end
"#,
    );
    let environment_id = environment
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name == "RuboCop::CLI::Environment")
        .map(|declaration| declaration.id.clone())
        .expect("environment declaration");
    let index = UniversalResolutionIndex::new(
        &[environment.clone(), caller.clone()],
        UniversalResolutionLimits::default(),
    )
    .expect("Ruby resolver index");
    let constructor = caller
        .candidates
        .iter()
        .find(|candidate| candidate.relation == CandidateRelation::Constructs)
        .expect("relative constructor candidate");
    let decision = index.resolve(&constructor.id);
    assert!(matches!(
        decision,
        ResolutionDecision::Resolved { declaration_id, evidence }
            if declaration_id == environment_id
                && evidence.rule == ResolutionRule::ExactLexicalDeclaration
    ));
}

#[test]
fn ruby_cross_file_singleton_calls_resolve_on_module_receivers() {
    let provider = extract_ruby(
        "lib/arel.rb",
        br#"module Arel
  def self.sql(value); end
end
"#,
    );
    let caller = extract_ruby("test/query.rb", b"Arel.sql(\"users.id\")\n");
    let target_id = provider
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name == "Arel.sql")
        .map(|declaration| declaration.id.clone())
        .expect("singleton method declaration");
    let index = UniversalResolutionIndex::new(
        &[provider.clone(), caller.clone()],
        UniversalResolutionLimits::default(),
    )
    .expect("Ruby resolver index");
    let call = caller
        .candidates
        .iter()
        .find(|candidate| candidate.relation == CandidateRelation::Calls)
        .expect("qualified singleton call candidate");
    assert!(matches!(
        index.resolve(&call.id),
        ResolutionDecision::Resolved { declaration_id, .. } if declaration_id == target_id
    ));
}

#[test]
fn ruby_nested_owner_resolves_top_level_singleton_calls_lexically() {
    let provider = extract_ruby(
        "lib/arel.rb",
        br#"module Arel
  def self.sql(value); end
end
"#,
    );
    let caller = extract_ruby(
        "test/query.rb",
        b"class QueryTest\n  def run\n    Arel.sql(\"users.id\")\n  end\nend\n",
    );
    let target_id = provider
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name == "Arel.sql")
        .map(|declaration| declaration.id.clone())
        .expect("singleton method declaration");
    let index = UniversalResolutionIndex::new(
        &[provider.clone(), caller.clone()],
        UniversalResolutionLimits::default(),
    )
    .expect("Ruby resolver index");
    let call = caller
        .candidates
        .iter()
        .find(|candidate| candidate.relation == CandidateRelation::Calls)
        .expect("qualified singleton call candidate");
    let decision = index.resolve(&call.id);
    assert!(matches!(
        decision,
        ResolutionDecision::Resolved { declaration_id, .. } if declaration_id == target_id
    ));
}

#[test]
fn ruby_reopened_module_singleton_calls_resolve_with_shared_identity() {
    let first = extract_ruby("lib/arel/nodes.rb", b"module Arel; end\n");
    let provider = extract_ruby(
        "lib/arel.rb",
        br#"module Arel
  def self.sql(value); end
end
"#,
    );
    let caller = extract_ruby("test/query.rb", b"Arel.sql(\"users.id\")\n");
    let target_id = provider
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name == "Arel.sql")
        .map(|declaration| declaration.id.clone())
        .expect("singleton method declaration");
    let index = UniversalResolutionIndex::new(
        &[first, provider, caller.clone()],
        UniversalResolutionLimits::default(),
    )
    .expect("Ruby resolver index");
    let call = caller
        .candidates
        .iter()
        .find(|candidate| candidate.relation == CandidateRelation::Calls)
        .expect("qualified singleton call candidate");
    assert!(matches!(
        index.resolve(&call.id),
        ResolutionDecision::Resolved { declaration_id, .. } if declaration_id == target_id
    ));
}

#[test]
fn ruby_relative_mixins_resolve_across_files_by_lexical_owner() {
    let support = extract_ruby(
        "lib/support.rb",
        br#"module Support
end
"#,
    );
    let caller = extract_ruby(
        "lib/rubocop/cli.rb",
        br#"module RuboCop
  class CLI
    include Support
  end
end
"#,
    );
    let support_id = support
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name == "Support")
        .map(|declaration| declaration.id.clone())
        .expect("support module declaration");
    let index = UniversalResolutionIndex::new(
        &[support, caller.clone()],
        UniversalResolutionLimits::default(),
    )
    .expect("Ruby resolver index");
    let mixin = caller
        .candidates
        .iter()
        .find(|candidate| candidate.relation == CandidateRelation::UsesTrait)
        .expect("relative mixin candidate");
    assert!(matches!(
        index.resolve(&mixin.id),
        ResolutionDecision::Resolved { declaration_id, evidence }
            if declaration_id == support_id
                && evidence.rule == ResolutionRule::ExactLexicalDeclaration
    ));
}

#[test]
fn ruby_mixins_inside_blocks_resolve_to_the_exact_trait() {
    let trait_batch = extract_ruby("lib/rendering.rb", b"module AbstractController::Rendering; end\n");
    let caller = extract_ruby(
        "lib/layouts.rb",
        br#"class Layouts
  def build
    Class.new do
      include AbstractController::Rendering
    end
  end
end
"#,
    );
    let target_id = trait_batch
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name == "AbstractController::Rendering")
        .map(|declaration| declaration.id.clone())
        .expect("trait declaration");
    let index = UniversalResolutionIndex::new(
        &[trait_batch, caller.clone()],
        UniversalResolutionLimits::default(),
    )
    .expect("Ruby resolver index");
    let mixin = caller
        .candidates
        .iter()
        .find(|candidate| candidate.relation == CandidateRelation::UsesTrait)
        .expect("block mixin candidate");
    assert!(matches!(
        index.resolve(&mixin.id),
        ResolutionDecision::Resolved { declaration_id, .. } if declaration_id == target_id
    ));
}

#[test]
fn ruby_reopened_type_constructors_share_one_graph_identity() {
    let first = extract_ruby("first.rb", b"class Example; end\n");
    let second = extract_ruby("second.rb", b"class Example; end\n");
    let caller = extract_ruby("caller.rb", b"Example.new\n");
    let index = UniversalResolutionIndex::new(
        &[first.clone(), second.clone(), caller.clone()],
        UniversalResolutionLimits::default(),
    )
    .expect("Ruby resolver index");
    let constructor = caller
        .candidates
        .iter()
        .find(|candidate| candidate.relation == CandidateRelation::Constructs)
        .expect("constructor candidate");
    let graph_node_id = first
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name == "Example")
        .map(|declaration| declaration.graph_node_id.clone())
        .expect("Example declaration");
    let decision = index.resolve(&constructor.id);
    assert!(matches!(
        decision,
        ResolutionDecision::Resolved { declaration_id, .. }
            if first.declarations.iter().chain(second.declarations.iter()).any(|declaration| {
                declaration.id == declaration_id && declaration.graph_node_id == graph_node_id
            })
    ));
}
