#![allow(clippy::expect_used, clippy::panic)]

use std::path::Path;

use compass_languages::{CandidateRelation, Engine, SemanticRole, validate_evidence};

fn extract(source: &[u8]) -> compass_languages::SemanticEvidenceBatch {
    Engine::default()
        .extract_source_universal_evidence(Path::new("fixture.rb"), "fixture.rb", source)
        .expect("Ruby universal evidence")
}

#[test]
fn emits_nested_types_reopenings_methods_mixins_and_exact_anchors() {
    let source = br#"module Billing
  module Auditable
  end
  class Invoice < Document
    include Auditable
    prepend Serializable
    extend ClassMethods
    def total(amount, tax = 0, *rest, &block)
      calculate(amount)
    end
    def self.build
      new(1)
    end
  end
end
class Billing::Invoice
  def reopened; end
end
"#;
    let evidence = extract(source);
    validate_evidence(&evidence, compass_languages::EvidenceLimits::default())
        .expect("validated Ruby evidence");
    assert!(
        evidence
            .declarations
            .iter()
            .any(|fact| fact.kind == "trait" && fact.qualified_name == "Billing::Auditable")
    );
    assert!(
        evidence
            .declarations
            .iter()
            .any(|fact| fact.kind == "class" && fact.qualified_name == "Billing::Invoice")
    );
    assert!(
        evidence
            .declarations
            .iter()
            .any(|fact| fact.qualified_name == "Billing::Invoice#total")
    );
    assert!(
        evidence
            .declarations
            .iter()
            .any(|fact| fact.qualified_name == "Billing::Invoice.build")
    );
    assert!(evidence.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::UsesTrait
            && candidate.constraints.qualified_name.as_deref() == Some("Billing::Auditable")
    }));
    assert!(evidence.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Extends
            && candidate.constraints.qualified_name.as_deref() == Some("Billing::Document")
    }));
    assert!(evidence.occurrences.iter().all(|occurrence| {
        occurrence.role != SemanticRole::Call
            || occurrence.range.end_byte > occurrence.range.start_byte
    }));
}

#[test]
fn emits_unqualified_mixin_call_from_a_cross_file_style_test_case() {
    let evidence = extract(
        br#"class AsyncAdapterTest < ActionCable::TestCase
  include CommonSubscriptionAdapterTest
end
"#,
    );
    assert!(evidence.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::UsesTrait
            && candidate.target_spelling == "CommonSubscriptionAdapterTest"
            && candidate.constraints.qualified_name.as_deref()
                == Some("AsyncAdapterTest::CommonSubscriptionAdapterTest")
    }));
}

#[test]
fn emits_singleton_setters_and_attribute_methods_with_exact_identity() {
    let evidence = extract(
        br#"module ActiveRecord
  module QueryLogs
    class << self
      attr_accessor :tags
      def tags=(value); end
    end
  end
end
class QueryLogsTest
  def test
    ActiveRecord::QueryLogs.tags = [1]
    ActiveRecord::QueryLogs.tags
  end
end
"#,
    );
    assert!(evidence.declarations.iter().any(|declaration| {
        declaration.kind == "method" && declaration.qualified_name == "ActiveRecord::QueryLogs.tags"
    }));
    assert!(evidence.declarations.iter().any(|declaration| {
        declaration.kind == "method"
            && declaration.qualified_name == "ActiveRecord::QueryLogs.tags="
    }));
    assert!(evidence.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Calls && candidate.target_spelling == "tags="
    }));
    assert!(evidence.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Calls && candidate.target_spelling == "tags"
    }));
}

#[test]
fn emits_mixins_inside_class_new_blocks() {
    let evidence = extract(
        br#"class LayoutsRactorTest
  def build
    Class.new do
      include AbstractController::Rendering
    end
  end
end
"#,
    );
    assert!(evidence.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::UsesTrait
            && candidate.target_spelling == "Rendering"
    }));
}

#[test]
fn rejects_dynamic_dispatch_and_malformed_regions_without_fabricating_edges() {
    let source = br#"class Example
  def run(name)
    send(name)
    require(name)
    def broken(
  end
end
"#;
    let evidence = extract(source);
    assert!(
        evidence
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "dynamic_dispatch_unresolved")
    );
    assert!(
        evidence
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "dynamic_require_unresolved")
    );
    assert!(
        evidence
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "partial_parser_recovery")
    );
    assert!(evidence.candidates.iter().all(|candidate| {
        candidate.constraints.qualified_name.as_deref() != Some("Example#send")
    }));
}

#[test]
fn deep_recovery_is_bounded_and_reports_a_typed_limit() {
    let mut source = String::new();
    for _ in 0..40 {
        source.push_str("if true\n");
    }
    source.push_str("value = 1\n");
    for _ in 0..40 {
        source.push_str("end\n");
    }
    let evidence = extract(source.as_bytes());
    validate_evidence(&evidence, compass_languages::EvidenceLimits::default())
        .expect("bounded Ruby evidence");
    assert!(
        evidence
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "traversal_limit")
    );
}

#[test]
fn universal_evidence_is_deterministic_under_repeated_extraction() {
    let source = "class Café\n  def naïve(value = 1)\n    value\n  end\nend\n".as_bytes();
    let left = extract(source);
    let right = extract(source);
    assert_eq!(left, right);
}

#[test]
fn local_parameters_and_constructed_receivers_are_lexically_scoped() {
    let source = br#"class Invoice
  def total(value)
    value
    document = Document.new
    document.save
  end

  def other
    document.save
  end
end
class Document
  def save; end
end
"#;
    let evidence = extract(source);
    let calls = evidence
        .candidates
        .iter()
        .filter(|candidate| candidate.relation == CandidateRelation::Calls)
        .collect::<Vec<_>>();
    assert!(
        calls
            .iter()
            .all(|candidate| candidate.target_spelling != "value")
    );
    assert!(calls.iter().any(|candidate| {
        candidate.target_spelling == "save"
            && candidate.constraints.hierarchy.is_some()
            && candidate
                .occurrence_id
                .as_ref()
                .and_then(|id| {
                    evidence
                        .occurrences
                        .iter()
                        .find(|occurrence| &occurrence.id == id)
                })
                .is_some_and(|occurrence| occurrence.qualifier.as_deref() == Some("document"))
    }));
    assert!(calls.iter().any(|candidate| {
        candidate.target_spelling == "save"
            && candidate
                .occurrence_id
                .as_ref()
                .and_then(|id| {
                    evidence
                        .occurrences
                        .iter()
                        .find(|occurrence| &occurrence.id == id)
                })
                .is_some_and(|occurrence| occurrence.qualifier.as_deref() == Some("document"))
    }));
}

#[test]
fn production_ruby_uses_one_universal_publisher_and_rails_pack() {
    let source = br#"class UsersController
  def show; end
end
Rails.application.routes.draw do
  get '/users', to: 'users#show'
end
"#;
    let extraction = Engine::default()
        .extract_source_graph_only(Path::new("config/routes.rb"), "config/routes.rb", source)
        .expect("production Ruby extraction");
    assert_eq!(
        extraction
            .semantic_evidence
            .as_ref()
            .map(|evidence| evidence.pipeline.id.as_str()),
        Some("compass.ruby")
    );
    assert!(extraction.raw_calls.is_none());
    assert!(extraction.framework_facts.iter().any(|fact| {
        matches!(
            fact,
            compass_languages::RawFrameworkFact::Route(route)
                if route.detail.get("frameworkPack").and_then(serde_json::Value::as_str)
                    == Some("rails-ruby")
        )
    }));
}

#[test]
fn qualified_singleton_owners_and_constructor_calls_keep_method_spaces_separate() {
    let evidence = extract(
        br#"class Invoice
  def save; end
  def self.build; end
end
def Invoice.lookup; end
class Caller
  def run
    Invoice.build
    invoice = Invoice.new
    invoice.save
  end
end
"#,
    );
    assert!(evidence.declarations.iter().any(|declaration| {
        declaration.qualified_name == "Invoice#save" && declaration.kind == "method"
    }));
    assert!(evidence.declarations.iter().any(|declaration| {
        declaration.qualified_name == "Invoice.build" && declaration.kind == "method"
    }));
    assert!(evidence.declarations.iter().any(|declaration| {
        declaration.qualified_name == "Invoice.lookup" && declaration.kind == "method"
    }));
    assert!(
        evidence
            .candidates
            .iter()
            .any(|candidate| candidate.relation == CandidateRelation::Constructs)
    );
}

#[test]
fn class_singleton_scope_keeps_methods_in_the_owner_method_space() {
    let evidence = extract(
        br#"class Invoice
  class << self
    def build
      new
    end
  end
end
"#,
    );
    assert!(evidence.declarations.iter().any(|declaration| {
        declaration.qualified_name == "Invoice.build" && declaration.kind == "method"
    }));
    assert!(
        evidence
            .declarations
            .iter()
            .all(|declaration| { !declaration.qualified_name.contains("::<<") })
    );
}
