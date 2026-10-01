#[test]
fn python_super_call_resolves_only_the_exact_direct_base_method() {
    let provider = extract(
        "pkg/base.py",
        b"class Base:\n    def run(self):\n        return None\n",
    );
    let caller_source =
        b"from pkg.base import Base\nclass Child(Base):\n    def run(self):\n        super().run()\n";
    let caller = extract("pkg/child.py", caller_source);
    let resolved = compass_resolve::resolve(
        &[provider, caller],
        &HashMap::from([(
            "pkg/child.py".to_owned(),
            String::from_utf8(caller_source.to_vec()).expect("source"),
        )]),
    );
    let base_run = resolved
        .nodes
        .iter()
        .find(|node| {
            node.string("symbol_kind") == "method"
                && node.string("source_file") == "pkg/base.py"
                && node.string("qualified_name") == "pkg.base.Base::run"
        })
        .unwrap_or_else(|| panic!("base method; nodes={:#?}", resolved.nodes));

    let calls = resolved
        .edges
        .iter()
        .filter(|edge| edge.string("relation") == "calls" && edge.target == base_run.id)
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].string("source_location"), "L4");
    assert_eq!(
        calls[0].string("resolution_rule"),
        "direct-receiver-successor-dispatch"
    );
}

#[test]
fn python_super_call_follows_a_complete_single_inheritance_chain() {
    let provider = extract(
        "pkg/base.py",
        b"class Grandparent:\n    def run(self):\n        return None\nclass Parent(Grandparent):\n    pass\n",
    );
    let caller_source =
        b"from pkg.base import Parent\nclass Child(Parent):\n    def run(self):\n        super().run()\n";
    let caller = extract("pkg/child.py", caller_source);
    let resolved = compass_resolve::resolve(
        &[provider, caller],
        &HashMap::from([(
            "pkg/child.py".to_owned(),
            String::from_utf8(caller_source.to_vec()).expect("source"),
        )]),
    );
    let grandparent_run = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "pkg.base.Grandparent::run")
        .unwrap_or_else(|| panic!("grandparent method; nodes={:#?}", resolved.nodes));

    let calls = resolved
        .edges
        .iter()
        .filter(|edge| edge.string("relation") == "calls" && edge.target == grandparent_run.id)
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].string("source_location"), "L4");
    assert_eq!(
        calls[0].string("resolution_rule"),
        "linearized-receiver-dispatch"
    );
}

#[test]
fn python_local_class_call_resolves_an_inherited_class_method() {
    let provider = extract(
        "pkg/base.py",
        b"class Base:\n    @classmethod\n    def check(cls):\n        return []\n",
    );
    let caller_source = b"from pkg.base import Base\ndef verify():\n    class Model(Base):\n        pass\n    return Model.check()\n";
    let caller = extract("pkg/checks.py", caller_source);
    let resolved = compass_resolve::resolve(
        &[provider, caller],
        &HashMap::from([(
            "pkg/checks.py".to_owned(),
            String::from_utf8(caller_source.to_vec()).expect("source"),
        )]),
    );
    let base_check = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "pkg.base.Base::check")
        .unwrap_or_else(|| panic!("base method; nodes={:#?}", resolved.nodes));

    let calls = resolved
        .edges
        .iter()
        .filter(|edge| edge.string("relation") == "calls" && edge.target == base_check.id)
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].string("source_location"), "L5");
    assert_eq!(
        calls[0].string("resolution_rule"),
        "linearized-receiver-dispatch"
    );
}

#[test]
fn python_rebound_local_class_receiver_does_not_invent_inherited_dispatch() {
    let source = b"class Base:\n    @classmethod\n    def check(cls):\n        return []\ndef verify(replacement):\n    class Model(Base):\n        pass\n    Model = replacement\n    return Model.check()\n";
    let extracted = extract("pkg/checks.py", source);
    let resolved = compass_resolve::resolve(
        &[extracted],
        &HashMap::from([(
            "pkg/checks.py".to_owned(),
            String::from_utf8(source.to_vec()).expect("source"),
        )]),
    );

    assert!(resolved.edges.iter().all(|edge| {
        edge.string("relation") != "calls"
            || edge.string("source_location") != "L9"
            || edge.string("resolution_rule") == "deferred-receiver"
    }));
}

#[test]
fn python_super_call_uses_complete_c3_order_across_multiple_bases() {
    let source = b"class Left:\n    pass\nclass Right:\n    def run(self):\n        return None\nclass Child(Left, Right):\n    def run(self):\n        super().run()\n";
    let extracted = extract("pkg/models.py", source);
    let resolved = compass_resolve::resolve(
        &[extracted],
        &HashMap::from([(
            "pkg/models.py".to_owned(),
            String::from_utf8(source.to_vec()).expect("source"),
        )]),
    );
    let right_run = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "pkg.models.Right::run")
        .unwrap_or_else(|| panic!("right method; nodes={:#?}", resolved.nodes));

    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.target == right_run.id
            && edge.string("source_location") == "L8"
            && edge.string("resolution_rule") == "linearized-receiver-dispatch"
    }));
}

#[test]
fn python_super_call_marks_a_later_base_behind_unknown_ancestry_as_possible() {
    let source = b"from external import Unknown\nclass Known:\n    def run(self):\n        return None\nclass Child(Unknown, Known):\n    def run(self):\n        super().run()\n";
    let extracted = extract("pkg/models.py", source);
    let resolved = compass_resolve::resolve(
        &[extracted],
        &HashMap::from([(
            "pkg/models.py".to_owned(),
            String::from_utf8(source.to_vec()).expect("source"),
        )]),
    );

    let known_run = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "pkg.models.Known::run")
        .unwrap_or_else(|| panic!("known method; nodes={:#?}", resolved.nodes));
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.target == known_run.id
            && edge.string("source_location") == "L7"
            && edge.string("confidence") == "INFERRED"
            && edge.string("resolution_rule") == "incomplete-hierarchy-receiver-dispatch"
    }));
}

#[test]
fn python_bound_method_receiver_resolves_exact_class_method() {
    let source = b"class Model:\n    def check(self):\n        return None\n\n    def verify(self):\n        return self.check()\n";
    let extracted = extract("pkg/models.py", source);
    let resolved = compass_resolve::resolve(
        &[extracted],
        &HashMap::from([(
            "pkg/models.py".to_owned(),
            String::from_utf8(source.to_vec()).expect("source"),
        )]),
    );
    let check = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "pkg.models.Model::check")
        .unwrap_or_else(|| panic!("check method; nodes={:#?}", resolved.nodes));

    let calls = resolved
        .edges
        .iter()
        .filter(|edge| edge.string("relation") == "calls" && edge.target == check.id)
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].string("source_location"), "L6");
    assert_eq!(
        calls[0].string("resolution_rule"),
        "linearized-receiver-dispatch"
    );
}

#[test]
fn python_super_call_with_multiple_bases_cannot_terminal_match_an_unrelated_method() {
    let source = b"class Unrelated:\n    def run(self):\n        return None\nclass Left:\n    pass\nclass Right:\n    pass\nclass Child(Left, Right):\n    def run(self):\n        super().run()\n";
    let extracted = extract("pkg/models.py", source);
    let resolved = compass_resolve::resolve(
        &[extracted],
        &HashMap::from([(
            "pkg/models.py".to_owned(),
            String::from_utf8(source.to_vec()).expect("source"),
        )]),
    );

    assert!(resolved.edges.iter().all(|edge| {
        edge.string("relation") != "calls"
            || edge.string("source_location") != "L9"
            || edge.string("resolution_rule") == "deferred-receiver"
    }));
}

#[test]
fn python_bound_receiver_reaches_a_later_leaf_separated_base() {
    let source = b"from external import Unknown\nclass Leaf:\n    pass\nclass Branch(Unknown):\n    def run(self):\n        return None\nclass Child(Leaf, Branch):\n    def call(self):\n        return self.run()\n";
    let extracted = extract("pkg/models.py", source);
    let resolved = compass_resolve::resolve(
        &[extracted],
        &HashMap::from([(
            "pkg/models.py".to_owned(),
            String::from_utf8(source.to_vec()).expect("source"),
        )]),
    );
    let branch_run = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "pkg.models.Branch::run")
        .unwrap_or_else(|| panic!("branch method; nodes={:#?}", resolved.nodes));

    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.target == branch_run.id
            && edge.string("source_location") == "L9"
            && edge.string("resolution_rule") == "linearized-receiver-dispatch"
    }));
}

#[test]
fn python_bound_receiver_marks_a_later_base_behind_unknown_ancestry_as_possible() {
    let source = b"from external import Unknown\nclass Known:\n    def run(self):\n        return None\nclass Child(Unknown, Known):\n    def call(self):\n        return self.run()\n";
    let extracted = extract("pkg/models.py", source);
    let resolved = compass_resolve::resolve(
        &[extracted],
        &HashMap::from([(
            "pkg/models.py".to_owned(),
            String::from_utf8(source.to_vec()).expect("source"),
        )]),
    );

    let known_run = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "pkg.models.Known::run")
        .unwrap_or_else(|| panic!("known method; nodes={:#?}", resolved.nodes));
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.target == known_run.id
            && edge.string("source_location") == "L7"
            && edge.string("confidence") == "INFERRED"
            && edge.string("resolution_rule") == "incomplete-hierarchy-receiver-dispatch"
    }));
}

#[test]
fn python_mixin_receiver_discovers_closed_world_descendant_dispatch() {
    let source = b"class Provider:\n    def run(self):\n        return None\nclass Mixin:\n    def call(self):\n        return self.run()\nclass Concrete(Provider, Mixin):\n    pass\n";
    let extracted = extract("pkg/models.py", source);
    let resolved = compass_resolve::resolve(
        &[extracted],
        &HashMap::from([(
            "pkg/models.py".to_owned(),
            String::from_utf8(source.to_vec()).expect("source"),
        )]),
    );
    let provider_run = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "pkg.models.Provider::run")
        .unwrap_or_else(|| panic!("provider method; nodes={:#?}", resolved.nodes));

    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.target == provider_run.id
            && edge.string("source_location") == "L6"
            && edge.string("confidence") == "INFERRED"
            && edge.string("resolution_rule") == "closed-world-receiver-dispatch"
    }));
}

#[test]
fn python_mixin_receiver_marks_a_first_base_target_with_external_ancestry_as_possible() {
    let source = b"from external import Unknown\nclass Provider(Unknown):\n    def run(self):\n        return None\nclass Mixin:\n    def call(self):\n        return self.run()\nclass Concrete(Provider, Mixin):\n    pass\n";
    let extracted = extract("pkg/models.py", source);
    let resolved = compass_resolve::resolve(
        &[extracted],
        &HashMap::from([(
            "pkg/models.py".to_owned(),
            String::from_utf8(source.to_vec()).expect("source"),
        )]),
    );
    let provider_run = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "pkg.models.Provider::run")
        .unwrap_or_else(|| panic!("provider method; nodes={:#?}", resolved.nodes));

    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.target == provider_run.id
            && edge.string("source_location") == "L7"
            && edge.string("confidence") == "INFERRED"
            && edge.string("resolution_rule") == "incomplete-hierarchy-receiver-dispatch"
    }));
}

#[test]
fn python_mixin_receiver_preserves_every_proven_descendant_target() {
    let source = b"class First:\n    def run(self):\n        return None\nclass Second:\n    def run(self):\n        return None\nclass Mixin:\n    def call(self):\n        return self.run()\nclass UsesFirst(First, Mixin):\n    pass\nclass UsesSecond(Second, Mixin):\n    pass\n";
    let extracted = extract("pkg/models.py", source);
    let resolved = compass_resolve::resolve(
        &[extracted],
        &HashMap::from([(
            "pkg/models.py".to_owned(),
            String::from_utf8(source.to_vec()).expect("source"),
        )]),
    );
    let targets = resolved
        .edges
        .iter()
        .filter(|edge| {
            edge.string("relation") == "calls"
                && edge.string("source_location") == "L9"
                && edge.string("resolution_rule") == "closed-world-receiver-dispatch"
        })
        .map(|edge| edge.target.as_str())
        .collect::<BTreeSet<_>>();
    let expected = resolved
        .nodes
        .iter()
        .filter(|node| {
            matches!(
                node.string("qualified_name").as_str(),
                "pkg.models.First::run" | "pkg.models.Second::run"
            )
        })
        .map(|node| node.id.as_str())
        .collect::<BTreeSet<_>>();

    assert_eq!(targets, expected);
    assert!(
        resolved
            .edges
            .iter()
            .filter(|edge| {
                targets.contains(edge.target.as_str())
                    && edge.string("source_location") == "L9"
                    && edge.string("resolution_rule") == "closed-world-receiver-dispatch"
            })
            .all(|edge| edge.string("confidence") == "INFERRED")
    );
}

#[test]
fn python_possible_dispatch_fails_closed_when_descendants_exceed_the_lookup_bound() {
    let source = b"class First:\n    def run(self):\n        return None\nclass Second:\n    def run(self):\n        return None\nclass Third:\n    def run(self):\n        return None\nclass Mixin:\n    def call(self):\n        return self.run()\nclass UsesFirst(First, Mixin):\n    pass\nclass UsesSecond(Second, Mixin):\n    pass\nclass UsesThird(Third, Mixin):\n    pass\n";
    let extracted = extract("pkg/models.py", source);
    let evidence = extracted
        .semantic_evidence
        .clone()
        .into_iter()
        .collect::<Vec<_>>();
    let limits = UniversalResolutionLimits {
        candidates_per_lookup: 2,
        ..UniversalResolutionLimits::default()
    };
    let index = UniversalResolutionIndex::new(&evidence, limits).expect("bounded index");
    let mut nodes = extracted.nodes.clone();
    let mut edges = extracted.edges.clone();
    index.materialize(&mut nodes, &mut edges);

    assert!(edges.iter().all(|edge| {
        !matches!(
            edge.string("resolution_rule").as_str(),
            "closed-world-receiver-dispatch" | "incomplete-hierarchy-receiver-dispatch"
        )
    }));
}

#[test]
fn python_bound_receiver_keeps_exact_target_and_marks_subclass_override_as_possible() {
    let source = b"class Base:\n    def run(self):\n        return None\n    def call(self):\n        return self.run()\nclass Child(Base):\n    def run(self):\n        return None\n";
    let extracted = extract("pkg/models.py", source);
    let resolved = compass_resolve::resolve(
        &[extracted],
        &HashMap::from([(
            "pkg/models.py".to_owned(),
            String::from_utf8(source.to_vec()).expect("source"),
        )]),
    );
    let calls = resolved
        .edges
        .iter()
        .filter(|edge| edge.string("relation") == "calls" && edge.string("source_location") == "L5")
        .map(|edge| {
            (
                resolved
                    .nodes
                    .iter()
                    .find(|node| node.id == edge.target)
                    .map(|node| node.string("qualified_name"))
                    .expect("call target"),
                edge.string("confidence"),
                edge.string("resolution_rule"),
            )
        })
        .collect::<BTreeSet<_>>();

    assert!(calls.contains(&(
        "pkg.models.Base::run".to_owned(),
        "EXTRACTED".to_owned(),
        "linearized-receiver-dispatch".to_owned(),
    )));
    assert!(calls.contains(&(
        "pkg.models.Child::run".to_owned(),
        "INFERRED".to_owned(),
        "closed-world-receiver-dispatch".to_owned(),
    )));
}

#[test]
fn python_mixin_receiver_does_not_match_an_unrelated_same_name_member() {
    let source = b"class Mixin:\n    def call(self):\n        return self.run()\nclass Unrelated:\n    def run(self):\n        return None\n";
    let extracted = extract("pkg/models.py", source);
    let resolved = compass_resolve::resolve(
        &[extracted],
        &HashMap::from([(
            "pkg/models.py".to_owned(),
            String::from_utf8(source.to_vec()).expect("source"),
        )]),
    );

    assert!(!resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.string("source_location") == "L3"
            && edge.string("resolution_rule") != "deferred-receiver"
    }));
}

#[test]
fn python_mixin_receiver_rejects_an_inconsistent_descendant_c3() {
    let source = b"class X:\n    pass\nclass Y:\n    pass\nclass A(X, Y):\n    def run(self):\n        return None\nclass B(Y, X):\n    pass\nclass Mixin:\n    def call(self):\n        return self.run()\nclass Broken(A, B, Mixin):\n    pass\n";
    let extracted = extract("pkg/models.py", source);
    let resolved = compass_resolve::resolve(
        &[extracted],
        &HashMap::from([(
            "pkg/models.py".to_owned(),
            String::from_utf8(source.to_vec()).expect("source"),
        )]),
    );

    assert!(!resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.string("source_location") == "L12"
            && edge.string("resolution_rule") != "deferred-receiver"
    }));
}

#[test]
fn python_mixin_super_discovers_the_member_after_it_in_descendant_c3() {
    let source = b"class Mixin:\n    def call(self):\n        return super().run()\nclass Provider:\n    def run(self):\n        return None\nclass Concrete(Mixin, Provider):\n    pass\n";
    let extracted = extract("pkg/models.py", source);
    let resolved = compass_resolve::resolve(
        &[extracted],
        &HashMap::from([(
            "pkg/models.py".to_owned(),
            String::from_utf8(source.to_vec()).expect("source"),
        )]),
    );
    let provider_run = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "pkg.models.Provider::run")
        .unwrap_or_else(|| panic!("provider method; nodes={:#?}", resolved.nodes));

    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.target == provider_run.id
            && edge.string("source_location") == "L3"
            && edge.string("confidence") == "INFERRED"
            && edge.string("resolution_rule") == "closed-world-receiver-dispatch"
    }));
}

#[test]
fn python_bound_receiver_resolves_a_source_proven_class_callable_alias() {
    let source = b"def helper():\n    return None\n\nclass UsesHelper:\n    helper_alias = helper\n\n    def run(self):\n        return self.helper_alias()\n";
    let extracted = extract("pkg/models.py", source);
    let resolved = compass_resolve::resolve(
        &[extracted],
        &HashMap::from([(
            "pkg/models.py".to_owned(),
            String::from_utf8(source.to_vec()).expect("source"),
        )]),
    );
    let helper = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "pkg.models.helper")
        .unwrap_or_else(|| panic!("helper function; nodes={:#?}", resolved.nodes));

    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.target == helper.id
            && edge.string("source_location") == "L8"
            && edge.string("resolution_rule") == "linearized-receiver-dispatch"
    }));
}

#[test]
fn python_shadowed_super_call_does_not_bind_the_builtin_hierarchy() {
    let source = b"class Base:\n    def run(self):\n        return None\nclass Child(Base):\n    def run(self, super):\n        super().run()\n";
    let extracted = extract("pkg/models.py", source);
    let resolved = compass_resolve::resolve(
        &[extracted],
        &HashMap::from([(
            "pkg/models.py".to_owned(),
            String::from_utf8(source.to_vec()).expect("source"),
        )]),
    );

    assert!(resolved.edges.iter().all(|edge| {
        edge.string("relation") != "calls" || edge.string("source_location") != "L6"
    }));
}

#[test]
fn python_callable_values_are_references_without_invocation_evidence() {
    let provider = extract(
        "pkg/provider.py",
        br#"class ValidationError(Exception):
    pass

def callback():
    return None
"#,
    );
    let consumer = extract(
        "app.py",
        br#"from pkg.provider import ValidationError, callback
from outside import unknown

def register(consume):
    consume(ValidationError)
    consume(callback)
    consume(unknown)
"#,
    );
    let resolved = compass_resolve::resolve(&[provider, consumer], &HashMap::new());
    let validation_error = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "pkg.provider.ValidationError")
        .expect("ValidationError class");
    let callback = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "pkg.provider.callback")
        .expect("callback function");
    let unknown = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "outside.unknown")
        .expect("external unknown reference");

    let validation_edges = resolved
        .edges
        .iter()
        .filter(|edge| edge.target == validation_error.id)
        .collect::<Vec<_>>();
    assert!(
        resolved.edges.iter().any(|edge| {
            edge.target == validation_error.id
                && edge.string("relation") == "references"
                && edge.string("source_file") == "app.py"
                && edge.string("source_location") == "L5"
        }),
        "validation edges: {validation_edges:#?}"
    );
    assert!(resolved.edges.iter().all(|edge| {
        !(edge.string("relation") == "indirect_call"
            && resolved
                .nodes
                .iter()
                .any(|node| node.id == edge.target && node.string("label") == "ValidationError"))
    }));
    let callback_edges = resolved
        .edges
        .iter()
        .filter(|edge| edge.target == callback.id)
        .collect::<Vec<_>>();
    assert!(
        resolved.edges.iter().any(|edge| {
            edge.target == callback.id
                && edge.string("relation") == "references"
                && edge.string("context") == "argument"
        }),
        "callback edges: {callback_edges:#?}"
    );
    assert!(resolved.edges.iter().all(|edge| {
        !(edge.target == callback.id && edge.string("relation") == "indirect_call")
    }));
    assert_eq!(unknown.string("symbol_kind"), "variable");
    assert!(resolved.edges.iter().any(|edge| {
        edge.target == unknown.id
            && edge.string("relation") == "references"
            && edge.string("context") == "argument"
    }));
    assert!(resolved.edges.iter().all(|edge| {
        !(edge.target == unknown.id && edge.string("relation") == "indirect_call")
    }));
}

#[test]
fn python_recursive_calls_preserve_exact_self_loops_and_occurrences() {
    let source = b"def recurse(value):\n    if value:\n        return recurse(value - 1)\n    return recurse(0)\n\nclass Walker:\n    def walk(self, value):\n        if value:\n            return self.walk(value - 1)\n        return None\n";
    let extracted = extract("pkg/recursive.py", source);
    let resolved = compass_resolve::resolve(
        &[extracted],
        &HashMap::from([(
            "pkg/recursive.py".to_owned(),
            String::from_utf8(source.to_vec()).expect("source"),
        )]),
    );
    let recurse = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "pkg.recursive.recurse")
        .unwrap_or_else(|| panic!("recursive function; nodes={:#?}", resolved.nodes));
    let walk = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "pkg.recursive.Walker::walk")
        .unwrap_or_else(|| panic!("recursive method; nodes={:#?}", resolved.nodes));

    let recursive_calls = resolved
        .edges
        .iter()
        .filter(|edge| {
            edge.source == recurse.id
                && edge.target == recurse.id
                && edge.string("relation") == "calls"
        })
        .collect::<Vec<_>>();
    assert_eq!(recursive_calls.len(), 2);
    assert_eq!(
        recursive_calls
            .iter()
            .map(|edge| edge.string("source_location"))
            .collect::<BTreeSet<_>>(),
        BTreeSet::from(["L3".to_owned(), "L4".to_owned()])
    );
    assert!(resolved.edges.iter().any(|edge| {
        edge.source == walk.id
            && edge.target == walk.id
            && edge.string("relation") == "calls"
            && edge.string("source_location") == "L9"
    }));

    let published = build_from_extraction(&resolved, true, None);
    assert_eq!(
        published
            .links
            .iter()
            .filter(|edge| {
                edge.source == recurse.id
                    && edge.target == recurse.id
                    && edge.string("relation") == "calls"
            })
            .count(),
        2,
        "recursive occurrences must survive graph endpoint normalization"
    );
}

#[test]
fn python_shadowed_or_unknown_receivers_never_invent_recursive_calls() {
    let source = b"def parameter_shadow(parameter_shadow):\n    return parameter_shadow()\n\ndef local_shadow():\n    local_shadow = callback\n    return local_shadow()\n\ndef closure_shadow(closure_shadow):\n    def inner():\n        return closure_shadow()\n    return inner()\n\ndef annotated_shadow():\n    annotated_shadow: object\n    return annotated_shadow()\n\ndef augmented_shadow():\n    augmented_shadow += callback\n    return augmented_shadow()\n\nclass Walker:\n    def walk_other(self, other):\n        return other.walk_other()\n";
    let extracted = extract("pkg/not_recursive.py", source);
    let resolved = compass_resolve::resolve(
        &[extracted],
        &HashMap::from([(
            "pkg/not_recursive.py".to_owned(),
            String::from_utf8(source.to_vec()).expect("source"),
        )]),
    );
    let declarations = resolved
        .nodes
        .iter()
        .filter(|node| {
            matches!(
                node.string("qualified_name").as_str(),
                "pkg.not_recursive.parameter_shadow"
                    | "pkg.not_recursive.local_shadow"
                    | "pkg.not_recursive.closure_shadow"
                    | "pkg.not_recursive.annotated_shadow"
                    | "pkg.not_recursive.augmented_shadow"
                    | "pkg.not_recursive.Walker::walk_other"
            )
        })
        .map(|node| node.id.as_str())
        .collect::<HashSet<_>>();

    let false_shadow_edges = resolved
        .edges
        .iter()
        .filter(|edge| {
            edge.string("relation") == "calls" && declarations.contains(edge.target.as_str())
        })
        .map(|edge| {
            (
                edge.string("source_location"),
                edge.string("resolution_rule"),
                edge.target.clone(),
            )
        })
        .collect::<Vec<_>>();
    assert!(
        false_shadow_edges.is_empty(),
        "false calls through statically shadowed names: {false_shadow_edges:?}"
    );
}

#[test]
fn python_global_directive_preserves_module_recursive_call() {
    let source = b"def global_recurse():\n    global global_recurse\n    return global_recurse()\n";
    let extracted = extract("pkg/global_recursive.py", source);
    let resolved = compass_resolve::resolve(
        &[extracted],
        &HashMap::from([(
            "pkg/global_recursive.py".to_owned(),
            String::from_utf8(source.to_vec()).expect("source"),
        )]),
    );
    let recurse = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "pkg.global_recursive.global_recurse")
        .unwrap_or_else(|| panic!("global recursive function; nodes={:#?}", resolved.nodes));

    assert!(resolved.edges.iter().any(|edge| {
        edge.source == recurse.id
            && edge.target == recurse.id
            && edge.string("relation") == "calls"
            && edge.string("source_location") == "L3"
    }));
}
#[test]
fn python_unit_of_work_attribute_calls_resolve_through_typed_fields() {
    let repositories = extract(
        "app/repositories.py",
        b"class Repository:\n    def get(self, key):\n        return key\n",
    );
    let uow = extract("app/uow.py", b"from app.repositories import Repository\nclass UnitOfWork:\n    field_lookup_tables: Repository\n");
    let service = extract("app/services.py", b"from app.uow import UnitOfWork\nclass MutationService:\n    def update(self, uow: UnitOfWork):\n        uow.field_lookup_tables.get(1)\n        uow.field_lookup_tables.get(2)\n");
    let resolved = compass_resolve::resolve(&[repositories, uow, service], &HashMap::new());
    let target = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "app.repositories.Repository::get")
        .expect("repository method");
    let calls = resolved
        .edges
        .iter()
        .filter(|edge| edge.string("relation") == "calls" && edge.target == target.id)
        .collect::<Vec<_>>();
    assert_eq!(
        calls.len(),
        2,
        "both source occurrences must resolve: {:#?}",
        resolved.edges
    );
    assert_eq!(calls[0].string("source_file"), "app/services.py");
    assert_ne!(
        calls[0].attributes.get("start_byte"),
        calls[1].attributes.get("start_byte")
    );
}

#[test]
fn python_injected_dependencies_and_constructor_assignments_keep_member_targets() {
    let source = b"class Repository:\n    def get(self):\n        return None\nclass UnitOfWork:\n    def __init__(self, repository: Repository):\n        self.repository = repository\nclass Service:\n    def __init__(self, uow: UnitOfWork):\n        self.uow = uow\n    def run(self):\n        self.uow.repository.get()\nclass ConstructedService:\n    def __init__(self):\n        self.repository = Repository()\n    def run(self):\n        self.repository.get()\n";
    let extracted = extract("app/services.py", source);
    let resolved = compass_resolve::resolve(&[extracted], &HashMap::new());
    let target = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "app.services.Repository::get")
        .expect("repository method");
    let calls = resolved
        .edges
        .iter()
        .filter(|edge| edge.string("relation") == "calls" && edge.target == target.id)
        .collect::<Vec<_>>();
    assert_eq!(
        calls.len(),
        2,
        "injected and constructed receiver calls: {:#?}",
        resolved.edges
    );
}

#[test]
fn python_chained_calls_follow_cross_file_return_contracts() {
    let provider = extract("app/repositories.py", b"class Repository:\n    def get(self):\n        return None\ndef factory() -> Repository:\n    return Repository()\n");
    let source = b"from app.repositories import factory, Repository\nclass Service:\n    def execute(self):\n        factory().get()\n        Repository().get()\n";
    let resolved = compass_resolve::resolve(
        &[provider, extract("app/services.py", source)],
        &HashMap::from([(
            "app/services.py".to_owned(),
            String::from_utf8(source.to_vec()).expect("source"),
        )]),
    );
    let target = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "app.repositories.Repository::get")
        .expect("repository method");
    let calls = resolved
        .edges
        .iter()
        .filter(|edge| edge.string("relation") == "calls" && edge.target == target.id)
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 2, "{:#?}", resolved.edges);
    assert!(
        calls
            .iter()
            .all(|edge| edge.string("confidence") == "EXTRACTED")
    );
}

#[test]
fn python_receiver_field_reassignments_do_not_reuse_stale_constructor_types() {
    let source = b"class Repository:\n    def get(self):\n        return None\nclass Service:\n    def __init__(self, repo: Repository):\n        self.repo = repo\n    def replace(self, other):\n        self.repo = other\n    def execute(self):\n        return self.repo.get()\n";
    let resolved = compass_resolve::resolve(
        &[extract("app/service.py", source)],
        &HashMap::from([(
            "app/service.py".to_owned(),
            String::from_utf8(source.to_vec()).expect("source"),
        )]),
    );
    let calls = resolved
        .edges
        .iter()
        .filter(|edge| {
            edge.string("relation") == "calls" && edge.string("source_location") == "L10"
        })
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].string("resolution_rule"), "deferred-receiver");
    assert_eq!(calls[0].string("confidence"), "INFERRED");
    assert!(resolved.nodes.iter().any(|node| node.id == calls[0].target
        && node.attributes.get("deferred_receiver") == Some(&serde_json::Value::Bool(true))));
}

#[test]
fn python_exception_construction_and_catch_types_retain_dependencies() {
    let source = b"class ValidationError(Exception):\n    pass\ndef validate():\n    raise ValidationError()\ndef handle():\n    try:\n        validate()\n    except ValidationError:\n        return None\n";
    let resolved =
        compass_resolve::resolve(&[extract("app/validation.py", source)], &HashMap::new());
    let error = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "app.validation.ValidationError")
        .expect("error class");
    for line in ["L4", "L8"] {
        assert!(
            resolved.edges.iter().any(|edge| edge.target == error.id
                && edge.string("source_location") == line
                && matches!(edge.string("relation").as_str(), "calls" | "references")),
            "missing exception dependency at {line}"
        );
    }
}

#[test]
fn python_typed_factory_method_chains_validate_and_resolve() {
    let source = b"class Repository:\n    def get(self):\n        return None\nclass UnitOfWork:\n    def repository(self) -> Repository:\n        return Repository()\nclass Service:\n    def factory(self) -> UnitOfWork:\n        return UnitOfWork()\n    def run(self, uow: UnitOfWork):\n        self.factory().repository().get()\n        uow.repository().get()\n";
    let resolved = compass_resolve::resolve(&[extract("app/service.py", source)], &HashMap::new());
    let repository = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "app.service.Repository::get")
        .expect("repository");
    let calls = resolved
        .edges
        .iter()
        .filter(|edge| edge.string("relation") == "calls" && edge.target == repository.id)
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 2, "{:#?}", resolved.edges);
    assert!(
        calls
            .iter()
            .all(|edge| edge.string("confidence") == "EXTRACTED")
    );
}
