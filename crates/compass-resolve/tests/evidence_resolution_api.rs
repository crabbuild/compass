use compass_resolve::evidence::{ResolutionDecision, ResolutionEvidence, ResolutionRule};

const ALL_RULES: [ResolutionRule; 16] = [
    ResolutionRule::ExactSourceDeclaration,
    ResolutionRule::ExactLexicalDeclaration,
    ResolutionRule::ExplicitBinding,
    ResolutionRule::ProjectModuleBinding,
    ResolutionRule::MemberBinding,
    ResolutionRule::DeferredReceiver,
    ResolutionRule::WildcardBinding,
    ResolutionRule::UniqueModuleOrPackage,
    ResolutionRule::ExactHierarchyBase,
    ResolutionRule::DirectReceiverSuccessorDispatch,
    ResolutionRule::LinearizedReceiverDispatch,
    ResolutionRule::ClosedWorldReceiverDispatch,
    ResolutionRule::IncompleteHierarchyReceiverDispatch,
    ResolutionRule::RustAssociatedType,
    ResolutionRule::ExactSourceInventory,
    ResolutionRule::QualifiedExternal,
];

#[test]
fn every_resolution_rule_retains_public_evidence() {
    for rule in ALL_RULES {
        let decision = ResolutionDecision::Resolved {
            declaration_id: "declaration".to_owned(),
            evidence: ResolutionEvidence {
                rule,
                candidate_count: 1,
            },
        };
        assert!(matches!(
            decision,
            ResolutionDecision::Resolved {
                ref declaration_id,
                evidence: ResolutionEvidence {
                    rule: actual_rule,
                    candidate_count: 1,
                },
            } if declaration_id == "declaration" && actual_rule == rule
        ));
    }
}

#[test]
fn every_resolution_decision_shape_is_explicit() {
    let exact_inventory = ResolutionDecision::ResolvedInventory {
        graph_node_id: "node".to_owned(),
        evidence: ResolutionEvidence {
            rule: ResolutionRule::ExactSourceInventory,
            candidate_count: 1,
        },
    };
    let external = ResolutionDecision::QualifiedExternal {
        qualified_name: "external::Target".to_owned(),
        evidence: ResolutionEvidence {
            rule: ResolutionRule::QualifiedExternal,
            candidate_count: 1,
        },
    };
    let deferred = ResolutionDecision::DeferredReceiver {
        qualified_name: "receiver.member".to_owned(),
        evidence: ResolutionEvidence {
            rule: ResolutionRule::DeferredReceiver,
            candidate_count: 1,
        },
    };

    assert!(matches!(
        exact_inventory,
        ResolutionDecision::ResolvedInventory { ref graph_node_id, .. } if graph_node_id == "node"
    ));
    assert!(matches!(
        external,
        ResolutionDecision::QualifiedExternal { ref qualified_name, .. }
            if qualified_name == "external::Target"
    ));
    assert!(matches!(
        deferred,
        ResolutionDecision::DeferredReceiver { ref qualified_name, .. }
            if qualified_name == "receiver.member"
    ));
    assert_eq!(
        ResolutionDecision::Ambiguous { candidate_count: 3 },
        ResolutionDecision::Ambiguous { candidate_count: 3 }
    );
    assert_eq!(
        ResolutionDecision::Unresolved,
        ResolutionDecision::Unresolved
    );
}
