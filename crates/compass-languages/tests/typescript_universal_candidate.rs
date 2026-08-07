#![allow(clippy::expect_used)]

use std::path::Path;

use compass_languages::{
    CandidateRelation, Engine, EvidenceLimits, SemanticRole, SymbolNamespace, validate_evidence,
};

fn candidate(path: &str, source: &[u8]) -> compass_languages::SemanticEvidenceBatch {
    Engine::default()
        .extract_source_universal_candidate_evidence(Path::new(path), path, source)
        .expect("candidate evidence")
}

#[test]
fn typescript_candidate_emits_direct_declarations_scopes_imports_and_calls() {
    let source = br#"import type { User as UserType } from "./types";
import Widget, * as widgets from "./widget";
interface User { id: string }
type ID = string;
class App extends Base implements Runnable {
    field: User;
    method(value: User): ID { return helper(value); }
}
const helper = (value: User): ID => value.id;
export { App, helper };
new App();
"#;
    let batch = candidate("src/app.ts", source);
    validate_evidence(&batch, EvidenceLimits::default()).expect("valid evidence");
    assert_eq!(batch.adapter.language, "typescript");
    assert_eq!(batch.adapter.version, 3);
    assert_eq!(batch.adapter.dialect.as_deref(), Some("ts"));
    assert!(
        batch
            .declarations
            .iter()
            .any(|declaration| declaration.name == "App" && declaration.kind == "class")
    );
    assert!(
        batch
            .declarations
            .iter()
            .any(|declaration| declaration.name == "User" && declaration.kind == "interface")
    );
    assert!(
        batch
            .declarations
            .iter()
            .any(|declaration| declaration.name == "helper" && declaration.kind == "function")
    );
    assert!(
        batch
            .declarations
            .iter()
            .any(|declaration| { declaration.name == "value" && declaration.kind == "parameter" })
    );
    assert!(batch.scopes.iter().any(|scope| scope.kind == "class"));
    assert!(
        batch
            .bindings
            .iter()
            .any(|binding| binding.spelling == "UserType")
    );
    let user_type = batch
        .bindings
        .iter()
        .find(|binding| binding.spelling == "UserType")
        .expect("type-only import binding");
    assert_eq!(user_type.namespace, Some(SymbolNamespace::Type));
    assert!(user_type.type_only);
    let widgets = batch
        .bindings
        .iter()
        .find(|binding| binding.spelling == "widgets")
        .expect("namespace import binding");
    assert_eq!(widgets.namespace, Some(SymbolNamespace::Namespace));
    assert!(!widgets.type_only);
    let app = batch
        .declarations
        .iter()
        .find(|declaration| declaration.name == "App")
        .expect("class declaration");
    assert_eq!(app.namespace, Some(SymbolNamespace::ValueAndType));
    let user = batch
        .declarations
        .iter()
        .find(|declaration| declaration.name == "User")
        .expect("interface declaration");
    assert_eq!(user.namespace, Some(SymbolNamespace::Type));
    assert!(
        batch
            .bindings
            .iter()
            .any(|binding| binding.spelling == "Widget")
    );
    assert!(
        batch
            .occurrences
            .iter()
            .any(|occurrence| occurrence.role == SemanticRole::Call)
    );
    assert!(batch
        .candidates
        .iter()
        .any(|candidate| candidate.relation == compass_languages::CandidateRelation::Constructs));
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::Extends
            || candidate.relation == compass_languages::CandidateRelation::Implements
    }));
}

#[test]
fn javascript_candidate_emits_jsx_commonjs_and_dynamic_import_evidence() {
    let source = br#"const Button = () => null;
export function render() { return <Button />; }
const helper = require("./helper");
module.exports = render;
export async function load() { return import("./lazy.js"); }
"#;
    let batch = candidate("src/render.jsx", source);
    assert_eq!(batch.adapter.language, "javascript");
    assert_eq!(batch.adapter.version, 3);
    assert_eq!(batch.adapter.dialect.as_deref(), Some("jsx"));
    assert!(
        batch
            .bindings
            .iter()
            .any(|binding| binding.spelling == "helper")
    );
    let helper = batch
        .bindings
        .iter()
        .find(|binding| binding.spelling == "helper")
        .expect("CommonJS binding");
    assert_eq!(helper.namespace, Some(SymbolNamespace::Namespace));
    assert_eq!(helper.qualified_target, "./helper::*");
    assert!(!helper.type_only);
    assert!(
        batch
            .occurrences
            .iter()
            .any(|occurrence| { occurrence.context.as_deref() == Some("jsx") })
    );
    assert!(
        batch
            .occurrences
            .iter()
            .any(|occurrence| { occurrence.context.as_deref() == Some("commonjs") })
    );
    assert!(
        batch
            .occurrences
            .iter()
            .any(|occurrence| { occurrence.context.as_deref() == Some("dynamic_import") })
    );
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::Reexports
    }));

    let import_equals = candidate(
        "src/cjs-types.ts",
        br#"import axios = require("axios");
axios.get("/items");
"#,
    );
    assert!(import_equals.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::Imports
            && candidate.target_spelling == "axios"
            && candidate
                .occurrence_id
                .as_ref()
                .and_then(|id| {
                    import_equals
                        .occurrences
                        .iter()
                        .find(|occurrence| occurrence.id == *id)
                })
                .is_some_and(|occurrence| occurrence.context.as_deref() == Some("import_equals"))
    }));

    let javascript_inheritance = candidate(
        "src/canceled.js",
        br#"import AxiosError from "./AxiosError.js";
class CanceledError extends AxiosError {}
"#,
    );
    assert!(javascript_inheritance.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::Extends
            && candidate.target_spelling == "AxiosError"
            && candidate.constraints.qualified_name.as_deref() == Some("./AxiosError.js::default")
    }));

    let namespace_jsx = candidate(
        "src/components.tsx",
        br#"import * as UI from "./ui";
export function render() { return <UI.Button />; }
"#,
    );
    let jsx_member = namespace_jsx
        .candidates
        .iter()
        .find(|candidate| {
            candidate.relation == compass_languages::CandidateRelation::References
                && candidate.occurrence_id.is_some()
                && candidate.target_spelling == "Button"
        })
        .expect("namespace JSX member reference");
    assert_eq!(
        jsx_member.constraints.qualified_name.as_deref(),
        Some("./ui::Button")
    );
    assert!(jsx_member.binding_id.is_some());
}

#[test]
fn javascript_candidate_publishes_spread_free_default_object_members() {
    let batch = candidate(
        "src/utils.js",
        br#"const isNumber = value => typeof value === 'number';
const isString = value => typeof value === 'string';
export default { isNumber, isString };
"#,
    );
    validate_evidence(&batch, EvidenceLimits::default()).expect("default object evidence");
    let default_object = batch
        .declarations
        .iter()
        .find(|declaration| declaration.name == "default" && declaration.kind == "variable")
        .expect("default object declaration");
    assert_eq!(default_object.qualified_name, "utils.default");
    let is_number = batch
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name == "utils.default.isNumber")
        .expect("default object member declaration");
    assert_eq!(is_number.kind, "property");
    assert!(batch.bindings.iter().any(|binding| {
        binding.kind == compass_languages::BindingKind::Reexport
            && binding.spelling == "default"
            && binding.target_declaration_id.as_deref() == Some(default_object.id.as_str())
    }));

    let spread = candidate(
        "src/spread-default.js",
        br#"const base = { isNumber: value => true };
export default { ...base, isString: value => true };
"#,
    );
    validate_evidence(&spread, EvidenceLimits::default()).expect("spread default evidence");
    assert!(
        !spread
            .declarations
            .iter()
            .any(|declaration| declaration.qualified_name == "spread-default.default")
    );
    assert!(!spread.bindings.iter().any(|binding| {
        binding.kind == compass_languages::BindingKind::Reexport && binding.spelling == "default"
    }));
}

#[test]
fn typescript_candidate_publishes_wildcard_barrel_reexports() {
    let batch = candidate(
        "src/index.ts",
        br#"export * from "./values";
export * as values from "./values";
export type * from "./types";
"#,
    );
    validate_evidence(&batch, EvidenceLimits::default()).expect("wildcard reexport evidence");

    let wildcard = batch
        .bindings
        .iter()
        .find(|binding| {
            binding.kind == compass_languages::BindingKind::Reexport
                && binding.spelling == "*"
                && binding.qualified_target == "./values::*"
        })
        .expect("wildcard reexport binding");
    assert_eq!(wildcard.namespace, Some(SymbolNamespace::Namespace));
    assert!(!wildcard.type_only);
    let alias = batch
        .bindings
        .iter()
        .find(|binding| {
            binding.kind == compass_languages::BindingKind::Reexport
                && binding.spelling == "values"
                && binding.qualified_target == "./values::*"
        })
        .expect("namespace reexport alias");
    assert_eq!(alias.namespace, Some(SymbolNamespace::Namespace));
    assert!(!alias.type_only);
    assert!(batch.occurrences.iter().any(|occurrence| {
        occurrence.spelling == "*" && occurrence.context.as_deref() == Some("wildcard")
    }));

    let type_only = batch
        .bindings
        .iter()
        .find(|binding| {
            binding.kind == compass_languages::BindingKind::Reexport
                && binding.spelling == "*"
                && binding.qualified_target == "./types::*"
        })
        .expect("type-only wildcard reexport binding");
    assert!(type_only.type_only);
}

#[test]
fn javascript_commonjs_require_preserves_namespace_and_export_keys() {
    let source = br#"const api = require("./api");
const {
    run: execute,
    method,
    "literal-name": literal,
    [dynamic]: computed,
...rest
} = require("./api");
const indirect = wrap(require("./api"));
api.run();
execute();
method();
"#;
    let batch = candidate("src/consumer.js", source);
    validate_evidence(&batch, EvidenceLimits::default()).expect("valid require evidence");

    let api = batch
        .bindings
        .iter()
        .find(|binding| binding.spelling == "api")
        .expect("direct require namespace binding");
    assert_eq!(api.namespace, Some(SymbolNamespace::Namespace));
    assert_eq!(api.qualified_target, "./api::*");

    for (local, target) in [
        ("execute", "./api::run"),
        ("method", "./api::method"),
        ("literal", "./api::literal-name"),
    ] {
        let actual = batch
            .bindings
            .iter()
            .find(|binding| binding.spelling == local)
            .map(|binding| binding.qualified_target.as_str());
        assert_eq!(
            actual,
            Some(target),
            "missing or incorrect require binding {local}"
        );
    }
    assert!(!batch.bindings.iter().any(|binding| {
        matches!(
            binding.spelling.as_str(),
            "computed" | "rest" | "dynamic" | "indirect"
        )
    }));
}

#[test]
fn javascript_commonjs_object_exports_publish_exact_named_reexports() {
    let source = br#"function run() {}
const alias = run;
module.exports = {
    run,
    alias,
    method() { return run(); },
    literal: true,
};
"#;
    let batch = candidate("src/object-export.js", source);
    validate_evidence(&batch, EvidenceLimits::default()).expect("CommonJS object evidence");

    let reexports = batch
        .bindings
        .iter()
        .filter(|binding| binding.kind == compass_languages::BindingKind::Reexport)
        .map(|binding| binding.spelling.as_str())
        .collect::<Vec<_>>();
    for name in ["default", "run", "alias", "method", "literal"] {
        assert!(
            reexports.contains(&name),
            "missing CommonJS object reexport {name}: {reexports:?}"
        );
    }

    let run = batch
        .declarations
        .iter()
        .find(|declaration| declaration.name == "run" && declaration.kind == "function")
        .expect("run declaration");
    let run_binding = batch
        .bindings
        .iter()
        .find(|binding| {
            binding.spelling == "run" && binding.kind == compass_languages::BindingKind::Reexport
        })
        .expect("run reexport");
    assert_eq!(
        run_binding.target_declaration_id.as_deref(),
        Some(run.id.as_str())
    );

    let method = batch
        .declarations
        .iter()
        .find(|declaration| declaration.name == "method" && declaration.kind == "method")
        .expect("method declaration");
    let method_binding = batch
        .bindings
        .iter()
        .find(|binding| {
            binding.spelling == "method" && binding.kind == compass_languages::BindingKind::Reexport
        })
        .expect("method reexport");
    assert_eq!(
        method_binding.target_declaration_id.as_deref(),
        Some(method.id.as_str())
    );

    let spread = candidate(
        "src/object-export-spread.js",
        br#"const other = getOther();
module.exports = { run, ...other };
"#,
    );
    assert!(spread.bindings.iter().any(|binding| {
        binding.spelling == "default" && binding.kind == compass_languages::BindingKind::Reexport
    }));
    assert!(!spread.bindings.iter().any(|binding| {
        binding.spelling == "run" && binding.kind == compass_languages::BindingKind::Reexport
    }));
}

#[test]
fn javascript_static_this_factory_tracks_new_instance_members() {
    let batch = candidate(
        "src/factory.js",
        br#"class Factory {
  static create() {
    const instance = new this();
    instance.run();
    return instance;
  }
  run() {}
}
Factory.create();
"#,
    );
    validate_evidence(&batch, EvidenceLimits::default()).expect("factory evidence");

    let factory = batch
        .declarations
        .iter()
        .find(|declaration| declaration.kind == "class" && declaration.name == "Factory")
        .expect("Factory declaration");
    let run = batch
        .declarations
        .iter()
        .find(|declaration| declaration.kind == "method" && declaration.name == "run")
        .expect("Factory.run declaration");
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::Constructs
            && candidate.target_spelling == "this"
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(factory.id.as_str())
    }));
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::Calls
            && candidate.target_spelling == "run"
            && candidate.constraints.exact_target_declaration_id.as_deref() == Some(run.id.as_str())
    }));
}

#[test]
fn javascript_function_constructor_prototypes_resolve_instance_members() {
    let batch = candidate(
        "src/prototype.js",
        br#"function Legacy(value) { this.value = value; }
Legacy.prototype.helper = function helper() { return this.value; };
Legacy.prototype.run = function run(value) { return this.helper(value); };
const alias = Legacy.prototype;
alias.extra = function extra() { return this.value; };
const instance = new Legacy("value");
instance.run("next");
instance.extra();
Legacy.prototype.run("direct");
const dynamic = "run";
instance[dynamic]();
const unknown = {};
unknown.prototype.run = function() {};
"#,
    );
    validate_evidence(&batch, EvidenceLimits::default()).expect("prototype evidence");

    let run = batch
        .declarations
        .iter()
        .find(|declaration| {
            declaration
                .qualified_name
                .ends_with(".Legacy.prototype.run")
        })
        .expect("prototype run declaration");
    let helper = batch
        .declarations
        .iter()
        .find(|declaration| {
            declaration
                .qualified_name
                .ends_with(".Legacy.prototype.helper")
        })
        .expect("prototype helper declaration");
    let extra = batch
        .declarations
        .iter()
        .find(|declaration| {
            declaration
                .qualified_name
                .ends_with(".Legacy.prototype.extra")
        })
        .expect("aliased prototype declaration");
    let value = batch
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name.ends_with(".Legacy.value"))
        .expect("constructor instance field declaration");
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::Calls
            && candidate.target_spelling == "run"
            && candidate.constraints.exact_target_declaration_id.as_deref() == Some(run.id.as_str())
    }));
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::Calls
            && candidate.target_spelling == "helper"
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(helper.id.as_str())
    }));
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::Calls
            && candidate.target_spelling == "extra"
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(extra.id.as_str())
    }));
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::AccessesMember
            && candidate.target_spelling == "value"
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(value.id.as_str())
    }));
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::Constructs
            && candidate.target_spelling == "Legacy"
            && candidate.constraints.exact_target_declaration_id.is_some()
    }));
    // Dynamic computed properties and unresolved prototype receivers must not
    // collapse onto the source-proven `Legacy.prototype.run` declaration.
    let run_id = run.id.clone();
    assert!(!batch.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::Calls
            && candidate.target_spelling == "run"
            && candidate.constraints.exact_target_declaration_id.as_deref() == Some(run_id.as_str())
            && candidate
                .occurrence_id
                .as_ref()
                .is_some_and(|occurrence_id| {
                    batch
                        .occurrences
                        .iter()
                        .find(|occurrence| occurrence.id == *occurrence_id)
                        .is_some_and(|occurrence| {
                            occurrence.context.as_deref() == Some("dynamic_member_call")
                        })
                })
    }));
}

#[test]
fn typescript_source_compatible_object_arguments_keep_exact_call_targets() {
    let batch = candidate(
        "src/calls.ts",
        br#"type SourceLine = { text: string };
interface Options { enabled: boolean }
function read(lines: SourceLine[], index: number, start: number) {}
function configure(options: Options) {}
function select(options: Pick<Options, "enabled">) {}
const lines: SourceLine[] = [];
read(lines, 0, 1);
configure({ enabled: true });
const options: Options = { enabled: true };
select(options);
"#,
    );
    for name in ["read", "configure", "select"] {
        assert!(
            batch.candidates.iter().any(|candidate| {
                candidate.relation == compass_languages::CandidateRelation::Calls
                    && candidate.target_spelling == name
                    && candidate.constraints.exact_target_declaration_id.is_some()
            }),
            "missing exact call target for {name}"
        );
    }
}

#[test]
fn typescript_constrained_generic_calls_keep_exact_targets() {
    let batch = candidate(
        "src/generics.ts",
        br#"const arrayToEnum = <T extends string, U extends [T, ...T[]]>(items: U) => items[0];
arrayToEnum(["one", "two"]);
"#,
    );
    let declaration = batch
        .declarations
        .iter()
        .find(|declaration| declaration.name == "arrayToEnum")
        .expect("generic declaration");
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::Calls
            && candidate.target_spelling == "arrayToEnum"
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(declaration.id.as_str())
    }));
}

#[test]
fn typescript_constrained_generic_member_chains_resolve_exact_targets() {
    let batch = candidate(
        "src/generic_members.ts",
        br#"interface Shape { name: string }
class Box<T extends Shape> {
    value!: T;
    read() { return this.value.name; }
}
"#,
    );
    let name = batch
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name.ends_with(".Shape.name"))
        .expect("Shape.name declaration");
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::AccessesMember
            && candidate.target_spelling == "name"
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(name.id.as_str())
    }));
}

#[test]
fn typescript_unconstrained_generic_member_chains_fail_closed() {
    let batch = candidate(
        "src/generic_unconstrained.ts",
        br#"function read<T>(value: T) {
    return value.name;
}
"#,
    );
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::AccessesMember
            && candidate.target_spelling == "name"
            && candidate.constraints.exact_target_declaration_id.is_none()
    }));
}

#[test]
fn typescript_callable_property_members_accept_rest_calls() {
    let batch = candidate(
        "src/callable_property.ts",
        br#"class Mocker {
    pick = (...args: any[]): any => args[0];
    value() { return this.pick(1, 2); }
}
"#,
    );
    let pick = batch
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name.ends_with(".Mocker.pick"))
        .expect("callable property declaration");
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::Calls
            && candidate.target_spelling == "pick"
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(pick.id.as_str())
    }));
}

#[test]
fn typescript_contextual_callback_member_types_resolve_exact_targets() {
    let batch = candidate(
        "src/contextual_callback.ts",
        br#"interface Common { issues: string[] }
type Callback = { run: (value: string, ctx: Common) => void };
function use(callback: Callback["run"]) {}
use((value, ctx) => ctx.issues);
"#,
    );
    let issues = batch
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name.ends_with(".Common.issues"))
        .expect("Common.issues declaration");
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::AccessesMember
            && candidate.target_spelling == "issues"
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(issues.id.as_str())
    }));
    let use_function = batch
        .declarations
        .iter()
        .find(|declaration| declaration.name == "use")
        .expect("use declaration");
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::Calls
            && candidate.target_spelling == "use"
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(use_function.id.as_str())
    }));
}

#[test]
fn typescript_literal_indexed_type_aliases_resolve_nominal_receivers() {
    let batch = candidate(
        "src/indexed-alias.ts",
        br#"interface Nested { run(): void }
interface Item { nested: Nested; inspect(): void }
type NestedAlias = Item["nested"];
function use(value: NestedAlias) {
    value.run();
}
function dynamic<T>(value: Item, key: T) {
    value[key];
}
"#,
    );
    let run = batch
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name.ends_with(".Nested.run"))
        .expect("Nested.run declaration");
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Calls
            && candidate.target_spelling == "run"
            && candidate.constraints.exact_target_declaration_id.as_deref() == Some(run.id.as_str())
    }));
    // A computed generic key is not a source-proven property and must remain
    // unresolved even when the receiver has a nominal annotation.
    assert!(!batch.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::AccessesMember
            && candidate.target_spelling == "key"
            && candidate.constraints.exact_target_declaration_id.is_some()
    }));
}

#[test]
fn typescript_indexed_type_alias_ambiguity_does_not_choose_a_union_member() {
    let batch = candidate(
        "src/indexed-ambiguous.ts",
        br#"interface First { run(): void }
interface Second { run(): void }
type Ambiguous = First | Second;
type Maybe = Ambiguous["run"];
function use(value: Maybe) {
    value();
}
"#,
    );
    let first_run = batch
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name.ends_with(".First.run"))
        .expect("First.run declaration");
    let second_run = batch
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name.ends_with(".Second.run"))
        .expect("Second.run declaration");
    assert!(!batch.candidates.iter().any(|candidate| {
        candidate
            .constraints
            .exact_target_declaration_id
            .as_deref()
            .is_some_and(|id| id == first_run.id || id == second_run.id)
    }));
}

#[test]
fn typescript_member_call_return_types_resolve_chained_members() {
    let batch = candidate(
        "src/member_return.ts",
        br#"class Maybe {
    optional() { return this; }
    value!: string;
}
class Box {
    nullable(): Maybe { return new Maybe(); }
    read() { return this.nullable().optional().value; }
}
"#,
    );
    let optional = batch
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name.ends_with(".Maybe.optional"))
        .expect("Maybe.optional declaration");
    let value = batch
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name.ends_with(".Maybe.value"))
        .expect("Maybe.value declaration");
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::Calls
            && candidate.target_spelling == "optional"
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(optional.id.as_str())
    }));
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::AccessesMember
            && candidate.target_spelling == "value"
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(value.id.as_str())
    }));
}

#[test]
fn typescript_member_call_return_types_resolve_inherited_methods() {
    let batch = candidate(
        "src/member_return_inherited.ts",
        br#"class Base { optional() { return this; } }
class Child extends Base {}
class Box {
    make(): Child { return new Child(); }
    read() { return this.make().optional(); }
}
"#,
    );
    let optional = batch
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name.ends_with(".Base.optional"))
        .expect("Base.optional declaration");
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::Calls
            && candidate.target_spelling == "optional"
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(optional.id.as_str())
    }));
}

#[test]
fn typescript_inherited_generic_member_types_resolve_exact_members() {
    let batch = candidate(
        "src/inherited_generic.ts",
        br#"interface Def { flag: boolean }
class Base<T> { value!: T; }
class Child extends Base<Def> { read() { const value = this.value; return value.flag; } }
"#,
    );
    let flag = batch
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name.ends_with(".Def.flag"))
        .expect("Def.flag declaration");
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::AccessesMember
            && candidate.target_spelling == "flag"
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(flag.id.as_str())
    }));
}

#[test]
fn typescript_interface_extends_members_resolve_exact_targets() {
    let batch = candidate(
        "src/interface-extends.ts",
        br#"type Callback = (issue: string) => string;
interface BaseDef { errorMap?: Callback | undefined }
interface ObjectDef extends BaseDef { shape: () => object }
function read(def: ObjectDef) {
    return def.errorMap?.("issue");
}
"#,
    );
    let error_map = batch
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name.ends_with(".BaseDef.errorMap"))
        .expect("BaseDef.errorMap declaration");
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Calls
            && candidate.target_spelling == "errorMap"
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(error_map.id.as_str())
    }));
}

#[test]
fn typescript_generic_instantiated_member_chains_resolve_constraint_targets() {
    let batch = candidate(
        "src/generic-member-chain.ts",
        br#"class Schema { _parseAsync() {} }
interface Definition<T extends Schema> { left: T }
class Intersection<T extends Schema> {
    _def!: Definition<T>;
    run() { this._def.left._parseAsync(); }
}
"#,
    );
    let parse_async = batch
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name.ends_with(".Schema._parseAsync"))
        .expect("Schema._parseAsync declaration");
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Calls
            && candidate.target_spelling == "_parseAsync"
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(parse_async.id.as_str())
    }));
}

#[test]
fn typescript_callable_property_return_types_resolve_generic_member_chains() {
    let batch = candidate(
        "src/callable-property-return.ts",
        br#"class Schema { _parse() {} }
interface Definition<T extends Schema> { getter: () => T }
class Lazy<T extends Schema> {
    _def!: Definition<T>;
    run() { this._def.getter()._parse(); }
}
"#,
    );
    let parse = batch
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name.ends_with(".Schema._parse"))
        .expect("Schema._parse declaration");
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Calls
            && candidate.target_spelling == "_parse"
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(parse.id.as_str())
    }));
}

#[test]
fn typescript_static_callable_aliases_resolve_exact_property_targets() {
    let batch = candidate(
        "src/static-callable-alias.ts",
        br#"function createSchema(value: string) { return value; }
class SchemaFactory {
    static create = createSchema;
    run() { return SchemaFactory.create("value"); }
}
"#,
    );
    let create = batch
        .declarations
        .iter()
        .find(|declaration| {
            declaration
                .qualified_name
                .ends_with(".SchemaFactory.create")
        })
        .expect("SchemaFactory.create declaration");
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Calls
            && candidate.target_spelling == "create"
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(create.id.as_str())
    }));
}

#[test]
fn typescript_variable_factory_aliases_preserve_declared_return_receivers() {
    let batch = candidate(
        "src/variable-factory-alias.ts",
        br#"class Schema {
    optional() {}
    static create(): Schema { return new Schema(); }
}
const create = Schema.create;
function run() { create().optional(); }
"#,
    );
    let optional = batch
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name.ends_with(".Schema.optional"))
        .expect("Schema.optional declaration");
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Calls
            && candidate.target_spelling == "optional"
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(optional.id.as_str())
    }));
}

#[test]
fn typescript_type_assertion_receivers_preserve_exact_generic_members() {
    let batch = candidate(
        "src/type-assertion-receiver.ts",
        br#"class Schema { parseAsync() {} }
interface Definition<T extends Schema> { value: T }
class Holder<T extends Schema> {
    _def!: Definition<T>;
    run() { (this._def as Definition<T>).value.parseAsync(); }
}
"#,
    );
    let parse_async = batch
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name.ends_with(".Schema.parseAsync"))
        .expect("Schema.parseAsync declaration");
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Calls
            && candidate.target_spelling == "parseAsync"
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(parse_async.id.as_str())
    }));
}

#[test]
fn typescript_destructured_inline_object_types_resolve_exact_members() {
    let batch = candidate(
        "src/featured.tsx",
        br#"type FeatureData = { name: string; link: string; lightImage: string; darkImage: string };
function Featured(props: { data: FeatureData }) {
    const { data: feature } = props;
    return feature.name + feature.link;
}
"#,
    );
    for property in ["name", "link"] {
        let declaration = batch
            .declarations
            .iter()
            .find(|declaration| {
                declaration.kind == "property"
                    && declaration
                        .qualified_name
                        .ends_with(&format!(".FeatureData.{property}"))
            })
            .expect("FeatureData member declaration");
        assert!(batch.candidates.iter().any(|candidate| {
            candidate.relation == compass_languages::CandidateRelation::AccessesMember
                && candidate.target_spelling == property
                && candidate.constraints.exact_target_declaration_id.as_deref()
                    == Some(declaration.id.as_str())
        }));
    }
}

#[test]
fn typescript_generic_member_call_returns_preserve_inherited_members() {
    let batch = candidate(
        "src/generic-member-return.ts",
        br#"class Base<T> { optional() {} }
class Nullable<T> extends Base<T> {}
class Schema<T> {
    nullable(): Nullable<this> { return new Nullable<this>(); }
    nullish() { return this.nullable().optional(); }
}
"#,
    );
    let optional = batch
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name.ends_with(".Base.optional"))
        .expect("Base.optional declaration");
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Calls
            && candidate.target_spelling == "optional"
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(optional.id.as_str())
    }));
}

#[test]
fn typescript_nominal_parameter_annotations_resolve_exact_members() {
    let batch = candidate(
        "src/context.ts",
        br#"interface Common { issues: string[]; contextualErrorMap?: string }
interface ParseContext { common: Common; path: string[]; data: unknown }
function report(ctx: ParseContext) {
    ctx.common.issues;
    ctx.common.contextualErrorMap;
    ctx.path;
    ctx.data;
}
"#,
    );
    for property in ["common", "path", "data"] {
        let declaration = batch
            .declarations
            .iter()
            .find(|declaration| {
                declaration.kind == "property"
                    && declaration
                        .qualified_name
                        .ends_with(&format!(".ParseContext.{property}"))
            })
            .expect("ParseContext member declaration");
        assert!(batch.candidates.iter().any(|candidate| {
            candidate.relation == compass_languages::CandidateRelation::AccessesMember
                && candidate.target_spelling == property
                && candidate.constraints.exact_target_declaration_id.as_deref()
                    == Some(declaration.id.as_str())
        }));
    }
    for property in ["issues", "contextualErrorMap"] {
        let declaration = batch
            .declarations
            .iter()
            .find(|declaration| {
                declaration.kind == "property"
                    && declaration
                        .qualified_name
                        .ends_with(&format!(".Common.{property}"))
            })
            .expect("Common nested member declaration");
        assert!(batch.candidates.iter().any(|candidate| {
            candidate.relation == compass_languages::CandidateRelation::AccessesMember
                && candidate.target_spelling == property
                && candidate.constraints.exact_target_declaration_id.as_deref()
                    == Some(declaration.id.as_str())
        }));
    }
}

#[test]
fn typescript_discriminated_union_guards_resolve_exact_branch_members() {
    let batch = candidate(
        "src/discriminated.ts",
        br#"type Success = { success: true; data: string };
type Failure = { success: false; error: Error };
type Result = Success | Failure;
function read(result: Result) {
    if (result.success) return result.data;
    return result.error;
}
"#,
    );
    let data = batch
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name.ends_with(".Success.data"))
        .expect("success data declaration");
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::AccessesMember
            && candidate.target_spelling == "data"
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(data.id.as_str())
    }));
}

#[test]
fn typescript_in_guards_resolve_the_unique_union_member_owner() {
    let batch = candidate(
        "src/in-guard.ts",
        br#"type Ready = { run(): void };
type Pending = { wait(): void };
type State = Ready | Pending;
function use(state: State) {
    if ("run" in state) state.run();
    if ("missing" in state) state.run();
}
"#,
    );
    let run = batch
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name.ends_with(".Ready.run"))
        .expect("Ready.run declaration");
    let runs = batch
        .candidates
        .iter()
        .filter(|candidate| {
            candidate.relation == CandidateRelation::Calls && candidate.target_spelling == "run"
        })
        .collect::<Vec<_>>();
    assert_eq!(runs.len(), 2);
    assert!(runs.iter().any(|candidate| {
        candidate.constraints.exact_target_declaration_id.as_deref() == Some(run.id.as_str())
    }));
    assert!(
        runs.iter()
            .any(|candidate| { candidate.constraints.exact_target_declaration_id.is_none() })
    );
}

#[test]
fn typescript_string_discriminant_guards_resolve_exact_branch_members() {
    let batch = candidate(
        "src/string-discriminated.ts",
        br#"class ReadySchema { parse() {} }
class OtherSchema { other() {} }
type Ready = { kind: "ready"; schema: ReadySchema };
type Other = { kind: "other"; schema: OtherSchema };
type State = Ready | Other;
function read(state: State) {
    if (state.kind === "ready") return state.schema.parse();
    return state.schema.other();
}
"#,
    );
    let parse = batch
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name.ends_with(".ReadySchema.parse"))
        .expect("ReadySchema.parse declaration");
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Calls
            && candidate.target_spelling == "parse"
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(parse.id.as_str())
    }));
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Calls
            && candidate.target_spelling == "other"
            && candidate.constraints.exact_target_declaration_id.is_none()
    }));
}

#[test]
fn typescript_string_discriminant_guards_resolve_callable_union_members() {
    let batch = candidate(
        "src/callable-discriminated.ts",
        br#"type Transform = { kind: "transform"; transform: (value: string) => void };
type Refinement = { kind: "refinement"; refinement: (value: string) => void };
type Effect = Transform | Refinement;
function run(effect: Effect) {
    if (effect.kind === "transform") effect.transform("value");
    else effect.refinement("value");
}
"#,
    );
    let transform = batch
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name.ends_with(".Transform.transform"))
        .expect("Transform.transform declaration");
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Calls
            && candidate.target_spelling == "transform"
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(transform.id.as_str())
    }));
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Calls
            && candidate.target_spelling == "refinement"
            && candidate.constraints.exact_target_declaration_id.is_none()
    }));
}

#[test]
fn typescript_string_discriminant_guards_follow_nullable_member_values() {
    let batch = candidate(
        "src/nullable-callable-discriminated.ts",
        br#"type Transform = { kind: "transform"; transform: (value: string) => void };
type Refinement = { kind: "refinement"; refinement: (value: string) => void };
type Effect = Transform | Refinement;
interface Definition { effect: Effect }
class Holder {
    _def!: Definition;
    run() {
        const effect = this._def.effect || null;
        if (effect.kind === "transform") effect.transform("value");
    }
}
"#,
    );
    let transform = batch
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name.ends_with(".Transform.transform"))
        .expect("Transform.transform declaration");
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Calls
            && candidate.target_spelling == "transform"
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(transform.id.as_str())
    }));
}

#[test]
fn typescript_union_inline_object_parameters_resolve_optional_members() {
    let batch = candidate(
        "src/inline-union.ts",
        br#"class Schema {
    datetime(options?: string | { precision?: number | null; offset?: boolean }) {
        return options?.precision;
    }
}
"#,
    );
    let precision = batch
        .declarations
        .iter()
        .find(|declaration| {
            declaration.kind == "property" && declaration.qualified_name.ends_with(".precision")
        })
        .expect("inline precision declaration");
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::AccessesMember
            && candidate.target_spelling == "precision"
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(precision.id.as_str())
    }));
}

#[test]
fn typescript_implements_arguments_match_source_declared_interfaces() {
    let batch = candidate(
        "src/implements-assignability.ts",
        br#"interface Input { value: string }
class Lazy implements Input { value = "ok"; }
class Consumer { run(value: Input) {} }
const consumer = new Consumer();
const lazy = new Lazy();
consumer.run(lazy);
"#,
    );
    let run = batch
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name.ends_with(".Consumer.run"))
        .expect("Consumer.run declaration");
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::Calls
            && candidate.target_spelling == "run"
            && candidate.constraints.exact_target_declaration_id.as_deref() == Some(run.id.as_str())
    }));
}

#[test]
fn typescript_index_signature_values_resolve_nominal_member_calls() {
    let batch = candidate(
        "src/index-signature.ts",
        br#"class Schema { run() {} }
interface Shape { [key: string]: Schema }
function use(shape: Shape, key: string) {
    const value = shape[key];
    value.run();
}
"#,
    );
    let run = batch
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name.ends_with(".Schema.run"))
        .expect("Schema.run declaration");
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::Calls
            && candidate.target_spelling == "run"
            && candidate.constraints.exact_target_declaration_id.as_deref() == Some(run.id.as_str())
    }));
}

#[test]
fn typescript_flow_sensitive_reassignment_selects_latest_nominal_receiver() {
    let batch = candidate(
        "src/reassignment.ts",
        br#"class First { run() {} }
class Second { run() {} }
let current = new First();
current = new Second();
current.run();
"#,
    );
    let first_run = batch
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name.ends_with(".First.run"))
        .expect("First.run declaration");
    let second_run = batch
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name.ends_with(".Second.run"))
        .expect("Second.run declaration");
    let calls = batch
        .candidates
        .iter()
        .filter(|candidate| {
            candidate.relation == compass_languages::CandidateRelation::Calls
                && candidate.target_spelling == "run"
        })
        .collect::<Vec<_>>();
    assert!(
        calls.iter().any(|candidate| {
            candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(second_run.id.as_str())
        }),
        "latest source assignment should own the member call"
    );
    assert!(!calls.iter().any(|candidate| {
        candidate.constraints.exact_target_declaration_id.as_deref() == Some(first_run.id.as_str())
    }));
}

#[test]
fn typescript_flow_sensitive_branch_assignment_fails_closed() {
    let batch = candidate(
        "src/branch-reassignment.ts",
        br#"class First { run() {} }
class Second { run() {} }
let current = new First();
if (flag) {
    current = new Second();
}
current.run();
"#,
    );
    assert!(!batch.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::Calls
            && candidate.target_spelling == "run"
            && candidate.constraints.exact_target_declaration_id.is_some()
    }));
}

#[test]
fn typescript_flow_sensitive_unknown_assignment_blocks_stale_receiver() {
    let batch = candidate(
        "src/unknown-reassignment.ts",
        br#"class First { run() {} }
let current = new First();
current = getUnknown();
current.run();
"#,
    );
    assert!(!batch.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::Calls
            && candidate.target_spelling == "run"
            && candidate.constraints.exact_target_declaration_id.is_some()
    }));
}

#[test]
fn javascript_flow_sensitive_reassignment_selects_latest_nominal_receiver() {
    let batch = candidate(
        "src/reassignment.js",
        br#"class First { run() {} }
class Second { run() {} }
let current = new First();
current = new Second();
current.run();
"#,
    );
    let second_run = batch
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name.ends_with(".Second.run"))
        .expect("Second.run declaration");
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::Calls
            && candidate.target_spelling == "run"
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(second_run.id.as_str())
    }));
}

#[test]
fn javascript_flow_sensitive_var_reassignment_inside_function_selects_latest_receiver() {
    let batch = candidate(
        "src/var-reassignment.js",
        br#"class First { run() {} }
class Second { run() {} }
function use() {
    var current = new First();
    current = new Second();
    current.run();
}
"#,
    );
    let second_run = batch
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name.ends_with(".Second.run"))
        .expect("Second.run declaration");
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::Calls
            && candidate.target_spelling == "run"
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(second_run.id.as_str())
    }));
}

#[test]
fn typescript_flow_sensitive_reassignment_is_ordered_by_use_site() {
    let batch = candidate(
        "src/ordered-reassignment.ts",
        br#"class First { run() {} }
class Second { run() {} }
let current = new First();
current.run();
current = new Second();
current.run();
"#,
    );
    let first_run = batch
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name.ends_with(".First.run"))
        .expect("First.run declaration");
    let second_run = batch
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name.ends_with(".Second.run"))
        .expect("Second.run declaration");
    let calls = batch
        .candidates
        .iter()
        .filter(|candidate| {
            candidate.relation == compass_languages::CandidateRelation::Calls
                && candidate.target_spelling == "run"
        })
        .collect::<Vec<_>>();
    assert_eq!(
        calls
            .iter()
            .filter(|candidate| {
                candidate.constraints.exact_target_declaration_id.as_deref()
                    == Some(first_run.id.as_str())
            })
            .count(),
        1
    );
    assert_eq!(
        calls
            .iter()
            .filter(|candidate| {
                candidate.constraints.exact_target_declaration_id.as_deref()
                    == Some(second_run.id.as_str())
            })
            .count(),
        1
    );
}

#[test]
fn typescript_flow_sensitive_compound_assignment_blocks_stale_receiver() {
    let batch = candidate(
        "src/compound-reassignment.ts",
        br#"class First { run() {} }
let current = new First();
current += unknownValue;
current.run();
"#,
    );
    assert!(!batch.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::Calls
            && candidate.target_spelling == "run"
            && candidate.constraints.exact_target_declaration_id.is_some()
    }));
}

#[test]
fn typescript_flow_sensitive_typed_call_assignment_uses_return_receiver() {
    let batch = candidate(
        "src/call-reassignment.ts",
        br#"class First { run() {} }
class Second { run() {} }
function makeSecond(): Second { return new Second(); }
let current = new First();
current = makeSecond();
current.run();
"#,
    );
    let second_run = batch
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name.ends_with(".Second.run"))
        .expect("Second.run declaration");
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::Calls
            && candidate.target_spelling == "run"
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(second_run.id.as_str())
    }));
}

#[test]
fn typescript_flow_sensitive_local_alias_preserves_source_receiver() {
    let batch = candidate(
        "src/local-alias.ts",
        br#"class First { run() {} }
let current = new First();
const alias = current;
alias.run();
"#,
    );
    let run = batch
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name.ends_with(".First.run"))
        .expect("First.run declaration");
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::Calls
            && candidate.target_spelling == "run"
            && candidate.constraints.exact_target_declaration_id.as_deref() == Some(run.id.as_str())
    }));
}

#[test]
fn javascript_flow_alias_escape_and_dynamic_mutation_fail_closed() {
    let cases = [
        (
            "src/escaped-alias.js",
            br#"class First { run() {} }
let current = new First();
consume(current);
current.run();
"#
            .as_slice(),
        ),
        (
            "src/eval-alias.js",
            br#"class First { run() {} }
let current = new First();
eval("current = unknownValue");
current.run();
"#
            .as_slice(),
        ),
        (
            "src/proxy-alias.js",
            br#"class First { run() {} }
let current = new First();
const wrapped = new Proxy(current, {});
current.run();
"#
            .as_slice(),
        ),
        (
            "src/member-write-alias.js",
            br#"class First { run() {} }
let current = new First();
current.run = replacement;
current.run();
"#
            .as_slice(),
        ),
        (
            "src/closure-alias.js",
            br#"class First { run() {} }
let current = new First();
function later() { current.run(); }
current.run();
"#
            .as_slice(),
        ),
        (
            "src/hoisted-closure-alias.js",
            br#"class First { run() {} }
function later() { current.run(); }
let current = new First();
current.run();
"#
            .as_slice(),
        ),
        (
            "src/with-alias.js",
            br#"class First { run() {} }
let current = new First();
with (scope) { current.run(); }
current.run();
"#
            .as_slice(),
        ),
    ];
    for (path, source) in cases {
        let batch = candidate(path, source);
        assert!(
            !batch.candidates.iter().any(|candidate| {
                candidate.relation == compass_languages::CandidateRelation::Calls
                    && candidate.target_spelling == "run"
                    && candidate.constraints.exact_target_declaration_id.is_some()
            }),
            "dynamic alias case unexpectedly resolved: {path}"
        );
    }
}

#[test]
fn javascript_const_this_alias_survives_a_closure_capture() {
    let batch = candidate(
        "src/const-this-alias.js",
        br#"class CancelToken {
    constructor() {
        const token = this;
        token._listeners = null;
        const later = () => token.subscribe();
        later();
    }
    subscribe() {}
}
new CancelToken();
"#,
    );
    let subscribe = batch
        .declarations
        .iter()
        .find(|declaration| {
            declaration
                .qualified_name
                .ends_with(".CancelToken.subscribe")
        })
        .expect("CancelToken.subscribe declaration");
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Calls
            && candidate.target_spelling == "subscribe"
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(subscribe.id.as_str())
    }));
}

#[test]
fn javascript_const_structural_alias_uses_property_scoped_mutation_barriers() {
    let batch = candidate(
        "src/const-structural-closure.js",
        br#"const config = {
    inspect() {}
};
config.other = 1;
const later = () => config.inspect();
later();
"#,
    );
    let inspect = batch
        .declarations
        .iter()
        .find(|declaration| declaration.name == "inspect")
        .expect("config.inspect declaration");
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Calls
            && candidate.target_spelling == "inspect"
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(inspect.id.as_str())
    }));

    let overwritten = candidate(
        "src/const-structural-overwrite.js",
        br#"const config = { inspect() {} };
config.inspect = replacement;
config.inspect();
"#,
    );
    assert!(!overwritten.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Calls
            && candidate.target_spelling == "inspect"
            && candidate.constraints.exact_target_declaration_id.is_some()
    }));

    let spread = candidate(
        "src/const-structural-spread.js",
        br#"const base = { inspect() {} };
const config = { ...base };
const later = () => config.inspect();
later();
"#,
    );
    assert!(!spread.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Calls
            && candidate.target_spelling == "inspect"
            && candidate.constraints.exact_target_declaration_id.is_some()
    }));

    let inline = candidate(
        "src/inline-structural-closure.js",
        br#"const key = Symbol('state');
class Service {
    constructor() {
        const state = (this[key] = { inspect() {}, other: 0 });
        state.other = 1;
        const later = () => state.inspect();
        later();
    }
}
new Service();
"#,
    );
    let inline_inspect = inline
        .declarations
        .iter()
        .find(|declaration| {
            declaration.name == "inspect"
                && declaration
                    .qualified_name
                    .ends_with(".Service.constructor.state.inspect")
        })
        .expect("inline state.inspect declaration");
    assert!(inline.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Calls
            && candidate.target_spelling == "inspect"
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(inline_inspect.id.as_str())
    }));

    let inline_overwritten = candidate(
        "src/inline-structural-overwrite.js",
        br#"const key = Symbol('state');
class Service {
    constructor() {
        const state = (this[key] = { inspect() {} });
        state.inspect = replacement;
        state.inspect();
    }
}
new Service();
"#,
    );
    assert!(!inline_overwritten.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Calls
            && candidate.target_spelling == "inspect"
            && candidate.constraints.exact_target_declaration_id.is_some()
    }));

    let separate = candidate(
        "src/inline-structural-separate.js",
        br#"const firstKey = Symbol('first');
const secondKey = Symbol('second');
class Service {
    constructor() {
        const first = (this[firstKey] = { inspect() { return 1; } });
        const second = (this[secondKey] = { inspect() { return 2; } });
        first.inspect();
        second.inspect();
    }
}
new Service();
"#,
    );
    let first_inspect = separate
        .declarations
        .iter()
        .find(|declaration| {
            declaration
                .qualified_name
                .ends_with(".Service.constructor.first.inspect")
        })
        .expect("first inline inspect declaration");
    let second_inspect = separate
        .declarations
        .iter()
        .find(|declaration| {
            declaration
                .qualified_name
                .ends_with(".Service.constructor.second.inspect")
        })
        .expect("second inline inspect declaration");
    let separate_calls = separate
        .candidates
        .iter()
        .filter(|candidate| {
            candidate.relation == CandidateRelation::Calls && candidate.target_spelling == "inspect"
        })
        .collect::<Vec<_>>();
    assert!(separate_calls.iter().any(|candidate| {
        candidate.constraints.exact_target_declaration_id.as_deref()
            == Some(first_inspect.id.as_str())
    }));
    assert!(separate_calls.iter().any(|candidate| {
        candidate.constraints.exact_target_declaration_id.as_deref()
            == Some(second_inspect.id.as_str())
    }));

    let chained = candidate(
        "src/inline-structural-chained.js",
        br#"const key = Symbol('state');
class Service {
    constructor() {
        const state = (this[key] = this[key] = { inspect() {}, other: 0 });
        state.other = 1;
        const later = () => state.inspect();
        later();
    }
}
new Service();
"#,
    );
    let chained_inspect = chained
        .declarations
        .iter()
        .find(|declaration| {
            declaration.name == "inspect"
                && declaration
                    .qualified_name
                    .ends_with(".Service.constructor.state.inspect")
        })
        .expect("chained inline state.inspect declaration");
    assert!(chained.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Calls
            && candidate.target_spelling == "inspect"
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(chained_inspect.id.as_str())
    }));

    let chained_compound = candidate(
        "src/inline-structural-chained-compound.js",
        br#"const key = Symbol('state');
class Service {
    constructor() {
        const state = (this[key] = this[key] += { inspect() {} });
        const later = () => state.inspect();
        later();
    }
}
new Service();
"#,
    );
    assert!(!chained_compound.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Calls
            && candidate.target_spelling == "inspect"
            && candidate.constraints.exact_target_declaration_id.is_some()
    }));
}

#[test]
fn javascript_nominal_member_writes_keep_exact_source_targets() {
    let source = br#"class Service {
    constructor() {
        this.kind = 'initial';
    }
}
const service = new Service();
service.kind = 'updated';
"#;
    let batch = candidate("src/nominal-member-write.js", source);
    let write_start = source
        .windows(b"service.kind".len())
        .rposition(|window| window == b"service.kind")
        .expect("service.kind write")
        .saturating_add(b"service.".len());
    let write_has_exact_target = batch.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::AccessesMember
            && candidate.target_spelling == "kind"
            && candidate
                .occurrence_id
                .as_ref()
                .and_then(|occurrence_id| {
                    batch
                        .occurrences
                        .iter()
                        .find(|occurrence| occurrence.id == *occurrence_id)
                })
                .is_some_and(|occurrence| occurrence.range.start_byte == write_start as u64)
            && candidate.constraints.exact_target_declaration_id.is_some()
    });
    assert!(
        write_has_exact_target,
        "nominal assignment write lost its exact target: {:#?}",
        batch.candidates
    );

    let compound_source = br#"class Service {
    constructor() {
        this.kind = 1;
    }
}
const service = new Service();
service.kind += 1;
"#;
    let compound = candidate("src/nominal-member-compound.js", compound_source);
    let compound_start = compound_source
        .windows(b"service.kind".len())
        .rposition(|window| window == b"service.kind")
        .expect("compound service.kind write")
        .saturating_add(b"service.".len());
    assert!(!compound.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::AccessesMember
            && candidate.target_spelling == "kind"
            && candidate
                .occurrence_id
                .as_ref()
                .and_then(|occurrence_id| {
                    compound
                        .occurrences
                        .iter()
                        .find(|occurrence| occurrence.id == *occurrence_id)
                })
                .is_some_and(|occurrence| occurrence.range.start_byte == compound_start as u64)
            && candidate.constraints.exact_target_declaration_id.is_some()
    }));

    let static_source = br#"class Service {
    static from() {
        const service = new Service();
        service.kind = 'updated';
        return service;
    }
    constructor() {
        this.kind = 'initial';
    }
}
Service.from();
"#;
    let static_batch = candidate("src/nominal-static-write.js", static_source);
    let static_start = static_source
        .windows(b"service.kind".len())
        .rposition(|window| window == b"service.kind")
        .expect("static service.kind write")
        .saturating_add(b"service.".len());
    assert!(static_batch.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::AccessesMember
            && candidate.target_spelling == "kind"
            && candidate
                .occurrence_id
                .as_ref()
                .and_then(|occurrence_id| {
                    static_batch
                        .occurrences
                        .iter()
                        .find(|occurrence| occurrence.id == *occurrence_id)
                })
                .is_some_and(|occurrence| occurrence.range.start_byte == static_start as u64)
            && candidate.constraints.exact_target_declaration_id.is_some()
    }));

    let escaped_source = br#"class Service {
    constructor() {
        this.kind = 'initial';
    }
}
function consume(value) {
    return value;
}
const service = new Service();
consume(service);
service.kind;
"#;
    let escaped = candidate("src/nominal-escape-read.js", escaped_source);
    let escaped_start = escaped_source
        .windows(b"service.kind".len())
        .rposition(|window| window == b"service.kind")
        .expect("escaped service.kind read")
        .saturating_add(b"service.".len());
    assert!(escaped.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::AccessesMember
            && candidate.target_spelling == "kind"
            && candidate
                .occurrence_id
                .as_ref()
                .and_then(|occurrence_id| {
                    escaped
                        .occurrences
                        .iter()
                        .find(|occurrence| occurrence.id == *occurrence_id)
                })
                .is_some_and(|occurrence| occurrence.range.start_byte == escaped_start as u64)
            && candidate.constraints.exact_target_declaration_id.is_some()
    }));

    let nested_source = br#"class Service {
    constructor() {
        this.kind = 'initial';
    }
}
function consume(value) {
    return value;
}
const service = new Service();
consume(service);
service.nested.kind = 'unknown';
"#;
    let nested = candidate("src/nominal-nested-escape-write.js", nested_source);
    let nested_start = nested_source
        .windows(b"service.nested.kind".len())
        .rposition(|window| window == b"service.nested.kind")
        .expect("nested escaped service.kind write")
        .saturating_add(b"service.nested.".len());
    assert!(!nested.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::AccessesMember
            && candidate.target_spelling == "kind"
            && candidate
                .occurrence_id
                .as_ref()
                .and_then(|occurrence_id| {
                    nested
                        .occurrences
                        .iter()
                        .find(|occurrence| occurrence.id == *occurrence_id)
                })
                .is_some_and(|occurrence| occurrence.range.start_byte == nested_start as u64)
            && candidate.constraints.exact_target_declaration_id.is_some()
    }));
}

#[test]
fn typescript_homomorphic_mapped_alias_preserves_nominal_member_targets() {
    let batch = candidate(
        "src/mapped-alias.ts",
        br#"interface Item { inspect(): void }
type Copy = { [K in keyof Item]: Item[K] };
function use(value: Copy) { value.inspect(); }
"#,
    );
    let inspect = batch
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name.ends_with(".Item.inspect"))
        .expect("Item.inspect declaration");
    assert!(
        batch.candidates.iter().any(|candidate| {
            candidate.relation == compass_languages::CandidateRelation::Calls
                && candidate.target_spelling == "inspect"
                && candidate.constraints.exact_target_declaration_id.as_deref()
                    == Some(inspect.id.as_str())
        }),
        "declarations: {:#?}\ncandidates: {:#?}",
        batch.declarations,
        batch.candidates
    );
}

#[test]
fn typescript_generic_homomorphic_mapped_alias_substitutes_nominal_target() {
    let batch = candidate(
        "src/generic-mapped-alias.ts",
        br#"interface Item { inspect(): void }
type Copy<T> = { [K in keyof T]: T[K] };
function use(value: Copy<Item>) { value.inspect(); }
"#,
    );
    let inspect = batch
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name.ends_with(".Item.inspect"))
        .expect("Item.inspect declaration");
    assert!(
        batch.candidates.iter().any(|candidate| {
            candidate.relation == compass_languages::CandidateRelation::Calls
                && candidate.target_spelling == "inspect"
                && candidate.constraints.exact_target_declaration_id.as_deref()
                    == Some(inspect.id.as_str())
        }),
        "declarations: {:#?}\ncandidates: {:#?}",
        batch.declarations,
        batch.candidates
    );
}

#[test]
fn typescript_generic_literal_indexed_alias_substitutes_nominal_target() {
    let batch = candidate(
        "src/generic-indexed-alias.ts",
        br#"interface Nested { run(): void }
interface Item { nested: Nested }
type NestedOf<T> = T["nested"];
function use(value: NestedOf<Item>) { value.run(); }
"#,
    );
    let run = batch
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name.ends_with(".Nested.run"))
        .expect("Nested.run declaration");
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Calls
            && candidate.target_spelling == "run"
            && candidate.constraints.exact_target_declaration_id.as_deref() == Some(run.id.as_str())
    }));
}

#[test]
fn typescript_keyof_identity_projection_preserves_nominal_members() {
    let batch = candidate(
        "src/keyof-identity.ts",
        br#"interface Item { inspect(): void }
type Copy<T> = Pick<T, keyof T>;
type Empty<T> = Omit<T, keyof T>;
function use(value: Copy<Item>) { value.inspect(); }
function rejected(value: Empty<Item>) { value.inspect(); }
"#,
    );
    let inspect = batch
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name.ends_with(".Item.inspect"))
        .expect("Item.inspect declaration");
    let calls = batch
        .candidates
        .iter()
        .filter(|candidate| {
            candidate.relation == CandidateRelation::Calls && candidate.target_spelling == "inspect"
        })
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 2);
    assert!(calls.iter().any(|candidate| {
        candidate.constraints.exact_target_declaration_id.as_deref() == Some(inspect.id.as_str())
    }));
    assert!(
        calls
            .iter()
            .any(|candidate| { candidate.constraints.exact_target_declaration_id.is_none() })
    );
}

#[test]
fn typescript_keyof_projection_with_competing_base_fails_closed() {
    let batch = candidate(
        "src/keyof-ambiguous.ts",
        br#"interface First { inspect(): void }
interface Second { inspect(): void }
type Keys = keyof First | keyof Second;
type Picked = Pick<First, Keys>;
function use(value: Picked) { value.inspect(); }
"#,
    );
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Calls
            && candidate.target_spelling == "inspect"
            && candidate.constraints.exact_target_declaration_id.is_none()
    }));
}

#[test]
fn typescript_non_homomorphic_mapped_alias_fails_closed() {
    let batch = candidate(
        "src/non-homomorphic-mapped-alias.ts",
        br#"interface Item { inspect(): void }
type Getters<T> = { [K in keyof T as `get${Capitalize<string & K>}`]: () => T[K] };
function use(value: Getters<Item>) { value.inspect(); }
"#,
    );
    assert!(
        batch.candidates.iter().any(|candidate| {
            candidate.relation == compass_languages::CandidateRelation::Calls
                && candidate.target_spelling == "inspect"
                && candidate.constraints.exact_target_declaration_id.is_none()
        }),
        "declarations: {:#?}\ncandidates: {:#?}",
        batch.declarations,
        batch.candidates
    );
}

#[test]
fn typescript_nested_mapped_member_does_not_promote_outer_alias() {
    let batch = candidate(
        "src/nested-mapped-alias.ts",
        br#"interface Item { inspect(): void }
type Nested = { nested: { [K in keyof Item]: Item[K] } };
function use(value: Nested) { value.inspect(); }
"#,
    );
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::Calls
            && candidate.target_spelling == "inspect"
            && candidate.constraints.exact_target_declaration_id.is_none()
    }));
}

#[test]
fn typescript_array_index_receiver_preserves_nominal_element_members() {
    let batch = candidate(
        "src/array-index.ts",
        br#"interface Item { inspect(): void }
function use(values: Item[]) { values[0].inspect(); }
"#,
    );
    let inspect = batch
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name.ends_with(".Item.inspect"))
        .expect("Item.inspect declaration");
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::Calls
            && candidate.target_spelling == "inspect"
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(inspect.id.as_str())
    }));
}

#[test]
fn typescript_tuple_index_receiver_preserves_nominal_element_members() {
    let batch = candidate(
        "src/tuple-index.ts",
        br#"interface Item { inspect(): void }
type Pair = [Item, string];
function use(pair: Pair) { pair[0].inspect(); }
"#,
    );
    let inspect = batch
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name.ends_with(".Item.inspect"))
        .expect("Item.inspect declaration");
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::Calls
            && candidate.target_spelling == "inspect"
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(inspect.id.as_str())
    }));
}

#[test]
fn typescript_generic_array_member_chain_substitutes_element_receiver() {
    let batch = candidate(
        "src/generic-array-index.ts",
        br#"interface Item { inspect(): void }
interface Box<T> { values: T[] }
function use(box: Box<Item>) { box.values[0].inspect(); }
"#,
    );
    let inspect = batch
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name.ends_with(".Item.inspect"))
        .expect("Item.inspect declaration");
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::Calls
            && candidate.target_spelling == "inspect"
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(inspect.id.as_str())
    }));
}

#[test]
fn typescript_standard_array_container_preserves_nominal_element_members() {
    let batch = candidate(
        "src/readonly-array-index.ts",
        br#"interface Item { inspect(): void }
function use(values: ReadonlyArray<Item>) { values[0].inspect(); }
"#,
    );
    let inspect = batch
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name.ends_with(".Item.inspect"))
        .expect("Item.inspect declaration");
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::Calls
            && candidate.target_spelling == "inspect"
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(inspect.id.as_str())
    }));
}

#[test]
fn typescript_dynamic_tuple_index_fails_closed() {
    let batch = candidate(
        "src/dynamic-tuple-index.ts",
        br#"interface Item { inspect(): void }
function use(pair: [Item, string], index: number) { pair[index].inspect(); }
"#,
    );
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::Calls
            && candidate.target_spelling == "inspect"
            && candidate.constraints.exact_target_declaration_id.is_none()
    }));
}

#[test]
fn typescript_generic_tuple_member_chain_substitutes_element_receiver() {
    let batch = candidate(
        "src/generic-tuple-index.ts",
        br#"interface Item { inspect(): void }
interface Box<T> { pair: [T, string] }
function use(box: Box<Item>) { box.pair[0].inspect(); }
"#,
    );
    let inspect = batch
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name.ends_with(".Item.inspect"))
        .expect("Item.inspect declaration");
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::Calls
            && candidate.target_spelling == "inspect"
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(inspect.id.as_str())
    }));
}

#[test]
fn typescript_generic_function_return_substitutes_nominal_argument() {
    let batch = candidate(
        "src/generic-return.ts",
        br#"class Item { inspect(): void {} }
function identity<T>(value: T): T { return value; }
function fixed<T>(value: T): Item { return new Item(); }
function use() {
    identity(new Item()).inspect();
    identity<Item>(new Item()).inspect();
    fixed("value").inspect();
}
"#,
    );
    let inspect = batch
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name.ends_with(".Item.inspect"))
        .expect("Item.inspect declaration");
    let calls = batch
        .candidates
        .iter()
        .filter(|candidate| {
            candidate.relation == compass_languages::CandidateRelation::Calls
                && candidate.target_spelling == "inspect"
        })
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 3);
    assert!(calls.iter().all(|candidate| {
        candidate.constraints.exact_target_declaration_id.as_deref() == Some(inspect.id.as_str())
    }));
}

#[test]
fn typescript_imported_callable_return_publishes_bounded_marker() {
    let batch = candidate(
        "src/imported-call.ts",
        br#"import { make } from "./factory";
class Item { inspect(): void {} }
function use(value: Item) { make(value).inspect(); }
"#,
    );
    let use_declaration = batch
        .declarations
        .iter()
        .find(|declaration| declaration.name == "use")
        .expect("use declaration");
    assert_eq!(use_declaration.signature.as_deref(), Some("|params:Item"));
    let inspect = batch
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name.ends_with(".Item.inspect"))
        .expect("Item.inspect declaration");
    let call = batch
        .candidates
        .iter()
        .find(|candidate| {
            candidate.relation == CandidateRelation::Calls && candidate.target_spelling == "inspect"
        })
        .expect("imported callable return member call");
    assert_eq!(
        call.constraints.exact_target_declaration_id, None,
        "cross-file call result must be resolved by the project resolver"
    );
    assert!(
        call.constraints
            .qualified_name
            .as_deref()
            .is_some_and(|qualified| qualified.contains("#call<"))
    );
    assert!(!call.constraints.allow_external);
    let local_call = batch.candidates.iter().find(|candidate| {
        candidate.relation == CandidateRelation::Calls
            && candidate.target_spelling == "inspect"
            && candidate.constraints.exact_target_declaration_id == Some(inspect.id.clone())
    });
    assert!(local_call.is_none());
}

#[test]
fn typescript_imported_callable_return_preserves_explicit_generic_marker() {
    let batch = candidate(
        "src/imported-explicit-call.ts",
        br#"import { identity } from "./factory";
class Item { inspect(): void {} }
function use(value: Item) { identity<Item>(value).inspect(); }
"#,
    );
    let call = batch
        .candidates
        .iter()
        .find(|candidate| {
            candidate.relation == CandidateRelation::Calls && candidate.target_spelling == "inspect"
        })
        .expect("explicit imported callable return member call");
    assert!(
        call.constraints
            .qualified_name
            .as_deref()
            .is_some_and(|qualified| qualified.contains("#call<") && qualified.contains("#types<"))
    );
    assert!(!call.constraints.allow_external);
}

#[test]
fn typescript_imported_callable_properties_publish_bounded_markers() {
    let api_batch = candidate(
        "src/api.ts",
        br#"import type { Item } from "./item";
interface TypedApi { make: (value: Item) => Item }
export declare const typed: TypedApi;
export const api = {
    make: (value: Item): Item => value,
    identity: <T>(value: T): T => value,
};
"#,
    );
    let make = api_batch
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name.ends_with(".api.make"))
        .expect("callable object property declaration");
    assert_eq!(make.signature.as_deref(), Some("|params:Item|return:Item"));
    let typed = api_batch
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name.ends_with(".typed"))
        .expect("typed object declaration");
    assert_eq!(typed.signature.as_deref(), Some("|type:TypedApi"));
    let batch = candidate(
        "src/imported-properties.ts",
        br#"import { api, typed } from "./api";
import type { Item } from "./item";
export function use(value: Item) {
    api.make(value).inspect();
    api.identity<Item>(value).inspect();
    typed.make(value).inspect();
}
"#,
    );
    let inspect_calls = batch
        .candidates
        .iter()
        .filter(|candidate| {
            candidate.relation == CandidateRelation::Calls && candidate.target_spelling == "inspect"
        })
        .collect::<Vec<_>>();
    assert_eq!(inspect_calls.len(), 3);
    assert!(inspect_calls.iter().all(|candidate| {
        candidate
            .constraints
            .qualified_name
            .as_deref()
            .is_some_and(|qualified| qualified.contains("#call<"))
            && !candidate.constraints.allow_external
    }));
    assert!(inspect_calls.iter().any(|candidate| {
        candidate
            .constraints
            .qualified_name
            .as_deref()
            .is_some_and(|qualified| qualified.contains("#types<"))
    }));
}

#[test]
fn typescript_generic_function_array_return_preserves_element_receiver() {
    let batch = candidate(
        "src/generic-return-array.ts",
        br#"class Item { inspect(): void {} }
function collect<T>(value: T): T[] { return [value]; }
function use() { collect(new Item())[0].inspect(); }
"#,
    );
    let inspect = batch
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name.ends_with(".Item.inspect"))
        .expect("Item.inspect declaration");
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::Calls
            && candidate.target_spelling == "inspect"
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(inspect.id.as_str())
    }));
}

#[test]
fn typescript_generic_function_nominal_container_return_preserves_member_receiver() {
    let batch = candidate(
        "src/generic-return-container.ts",
        br#"class Item { inspect(): void {} }
interface Box<T> { value: T }
function box<T>(value: T): Box<T> { return { value }; }
function use() { box(new Item()).value.inspect(); }
"#,
    );
    let inspect = batch
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name.ends_with(".Item.inspect"))
        .expect("Item.inspect declaration");
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::Calls
            && candidate.target_spelling == "inspect"
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(inspect.id.as_str())
    }));
}

#[test]
fn typescript_generic_function_return_fails_closed_without_inference() {
    let batch = candidate(
        "src/generic-return-unknown.ts",
        br#"declare function identity<T>(): T;
function use() { identity().inspect(); }
"#,
    );
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::Calls
            && candidate.target_spelling == "inspect"
            && candidate.constraints.exact_target_declaration_id.is_none()
    }));
}

#[test]
fn typescript_generic_function_return_fails_closed_on_conflicting_inference() {
    let batch = candidate(
        "src/generic-return-conflict.ts",
        br#"class Item { inspect(): void {} }
class Other {}
function choose<T>(first: T, second: T): T { return first; }
function use() { choose(new Item(), new Other()).inspect(); }
"#,
    );
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::Calls
            && candidate.target_spelling == "inspect"
            && candidate.constraints.exact_target_declaration_id.is_none()
    }));
}

#[test]
fn typescript_non_nullable_receiver_preserves_nominal_member_target() {
    let batch = candidate(
        "src/non-nullable-receiver.ts",
        br#"class Item { inspect(): void {} }
function use(value: NonNullable<Item | undefined>) { value.inspect(); }
"#,
    );
    let inspect = batch
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name.ends_with(".Item.inspect"))
        .expect("Item.inspect declaration");
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::Calls
            && candidate.target_spelling == "inspect"
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(inspect.id.as_str())
    }));
}

#[test]
fn typescript_awaited_and_readonly_receivers_preserve_nominal_member_targets() {
    let batch = candidate(
        "src/utility-receivers.ts",
        br#"class Item { inspect(): void {} }
function useAwaited(value: Awaited<Promise<Item>>) { value.inspect(); }
function useReadonly(value: Readonly<Item>) { value.inspect(); }
"#,
    );
    let inspect = batch
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name.ends_with(".Item.inspect"))
        .expect("Item.inspect declaration");
    let calls = batch
        .candidates
        .iter()
        .filter(|candidate| {
            candidate.relation == compass_languages::CandidateRelation::Calls
                && candidate.target_spelling == "inspect"
        })
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 2);
    assert!(calls.iter().all(|candidate| {
        candidate.constraints.exact_target_declaration_id.as_deref() == Some(inspect.id.as_str())
    }));
}

#[test]
fn typescript_pick_and_omit_receivers_project_exact_members() {
    let batch = candidate(
        "src/pick-omit-receivers.ts",
        br#"interface Options { enabled(): void; debug(): void }
function use(picked: Pick<Options, "enabled">, omitted: Omit<Options, "debug">) {
    picked.enabled();
    picked.debug();
    omitted.enabled();
    omitted.debug();
}
"#,
    );
    let enabled = batch
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name.ends_with(".Options.enabled"))
        .expect("Options.enabled declaration");
    let calls = batch
        .candidates
        .iter()
        .filter(|candidate| {
            candidate.relation == CandidateRelation::Calls
                && matches!(candidate.target_spelling.as_str(), "enabled" | "debug")
        })
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 4);
    assert_eq!(
        calls
            .iter()
            .filter(|candidate| candidate.target_spelling == "enabled")
            .count(),
        2
    );
    assert!(
        calls
            .iter()
            .filter(|candidate| candidate.target_spelling == "enabled")
            .all(|candidate| {
                candidate.constraints.exact_target_declaration_id.as_deref()
                    == Some(enabled.id.as_str())
            })
    );
    assert!(
        calls
            .iter()
            .filter(|candidate| candidate.target_spelling == "debug")
            .all(|candidate| candidate.constraints.exact_target_declaration_id.is_none())
    );

    let unknown = candidate(
        "src/pick-unknown-key.ts",
        br#"interface Options { enabled(): void }
type Key = string;
function use(value: Pick<Options, Key>) { value.enabled(); }
"#,
    );
    assert!(unknown.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Calls
            && candidate.target_spelling == "enabled"
            && candidate.constraints.exact_target_declaration_id.is_none()
    }));
}

#[test]
fn typescript_exclude_and_extract_narrow_nominal_union_receivers() {
    let batch = candidate(
        "src/exclude-extract-receivers.ts",
        br#"class Item { inspect(): void {} }
class Other { other(): void {} }
function exclude(value: Exclude<Item | undefined, undefined>) { value.inspect(); }
function extract(value: Extract<Item | Other, Item>) { value.inspect(); }
function ambiguous(value: Exclude<Item | Other, undefined>) { value.inspect(); }
"#,
    );
    let inspect = batch
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name.ends_with(".Item.inspect"))
        .expect("Item.inspect declaration");
    let calls = batch
        .candidates
        .iter()
        .filter(|candidate| {
            candidate.relation == CandidateRelation::Calls && candidate.target_spelling == "inspect"
        })
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 3);
    assert_eq!(
        calls
            .iter()
            .filter(|candidate| candidate.constraints.exact_target_declaration_id.is_some())
            .count(),
        2
    );
    assert!(calls.iter().any(|candidate| {
        candidate.constraints.exact_target_declaration_id.as_deref() == Some(inspect.id.as_str())
    }));
    assert!(
        calls
            .iter()
            .any(|candidate| { candidate.constraints.exact_target_declaration_id.is_none() })
    );
}

#[test]
fn typescript_conditional_receivers_select_unique_nominal_branch() {
    let batch = candidate(
        "src/conditional-receivers.ts",
        br#"class Item { inspect(): void {} }
class Other { other(): void {} }
type Choose<T> = T extends Item ? Item : Other;
type ChooseObject<T> = T extends object ? T : never;
function selected(value: Choose<Item>) { value.inspect(); }
function rejected(value: Choose<Other>) { value.inspect(); }
function union(value: Choose<Item | Other>) { value.inspect(); }
function direct(value: Item extends Item ? Item : Other) { value.inspect(); }
function object(value: ChooseObject<Item>) { value.inspect(); }
"#,
    );
    let inspect = batch
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name.ends_with(".Item.inspect"))
        .expect("Item.inspect declaration");
    let calls = batch
        .candidates
        .iter()
        .filter(|candidate| {
            candidate.relation == CandidateRelation::Calls && candidate.target_spelling == "inspect"
        })
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 5);
    assert_eq!(
        calls
            .iter()
            .filter(|candidate| candidate.constraints.exact_target_declaration_id.is_some())
            .count(),
        3
    );
    assert_eq!(
        calls
            .iter()
            .filter(|candidate| candidate.constraints.exact_target_declaration_id.is_none())
            .count(),
        2
    );
    assert!(calls.iter().any(|candidate| {
        candidate.constraints.exact_target_declaration_id.as_deref() == Some(inspect.id.as_str())
    }));
}

#[test]
fn typescript_mapped_modifier_aliases_preserve_nominal_member_targets() {
    let batch = candidate(
        "src/mapped-modifiers.ts",
        br#"class Item { inspect(): void {} }
type MutableRequired<T> = { -readonly [K in keyof T]-?: T[K] };
type ReadonlyOptional<T> = { +readonly [K in keyof T]+?: T[K] };
function use(mutable: MutableRequired<Item>, readonlyValue: ReadonlyOptional<Item>) {
    mutable.inspect();
    readonlyValue.inspect();
}
"#,
    );
    let inspect = batch
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name.ends_with(".Item.inspect"))
        .expect("Item.inspect declaration");
    let calls = batch
        .candidates
        .iter()
        .filter(|candidate| {
            candidate.relation == CandidateRelation::Calls && candidate.target_spelling == "inspect"
        })
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 2);
    assert!(calls.iter().all(|candidate| {
        candidate.constraints.exact_target_declaration_id.as_deref() == Some(inspect.id.as_str())
    }));
}

#[test]
fn typescript_non_nullable_multi_nominal_union_fails_closed() {
    let batch = candidate(
        "src/non-nullable-ambiguous.ts",
        br#"class First { inspect(): void {} }
class Second { inspect(): void {} }
function use(value: NonNullable<First | Second | undefined>) { value.inspect(); }
"#,
    );
    let inspect_calls = batch
        .candidates
        .iter()
        .filter(|candidate| {
            candidate.relation == compass_languages::CandidateRelation::Calls
                && candidate.target_spelling == "inspect"
        })
        .collect::<Vec<_>>();
    assert_eq!(inspect_calls.len(), 1);
    assert!(
        inspect_calls[0]
            .constraints
            .exact_target_declaration_id
            .is_none()
    );
}

#[test]
fn typescript_generic_index_signature_values_resolve_member_calls() {
    let batch = candidate(
        "src/generic-index-signature.ts",
        br#"class Schema { optional() {} }
type Shape = { [key: string]: Schema };
class ObjectHolder<T extends Shape> {
    shape!: T;
    run(key: string) {
        const field = this.shape[key]!;
        field.optional();
    }
}
"#,
    );
    let optional = batch
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name.ends_with(".Schema.optional"))
        .expect("Schema.optional declaration");
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Calls
            && candidate.target_spelling == "optional"
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(optional.id.as_str())
    }));
}

#[test]
fn typescript_callable_member_aliases_resolve_property_targets() {
    let batch = candidate(
        "src/callable-alias.ts",
        br#"class Service {
    run(value: string) {}
    alias = this.run;
    use() { this.alias("ok"); }
}
"#,
    );
    let alias = batch
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name.ends_with(".Service.alias"))
        .expect("alias property declaration");
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::Calls
            && candidate.target_spelling == "alias"
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(alias.id.as_str())
    }));
}

#[test]
fn typescript_optional_callable_union_properties_resolve_exact_targets() {
    let batch = candidate(
        "src/optional-callable.ts",
        br#"type Callback = (value: string) => string;
interface Hooks { callback?: Callback | undefined }
function use(hooks: Hooks) {
    return hooks.callback?.("value");
}
"#,
    );
    let callback = batch
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name.ends_with(".Hooks.callback"))
        .expect("Hooks.callback declaration");
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Calls
            && candidate.target_spelling == "callback"
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(callback.id.as_str())
    }));
}

#[test]
fn candidate_preserves_named_and_anonymous_default_exports() {
    let anonymous = candidate(
        "src/default.ts",
        b"export default function() { return 1; }\n",
    );
    assert!(
        anonymous
            .declarations
            .iter()
            .any(|declaration| { declaration.name == "default" && declaration.kind == "function" })
    );
    assert!(anonymous.bindings.iter().any(|binding| {
        binding.spelling == "default" && binding.kind == compass_languages::BindingKind::Reexport
    }));

    let named = candidate("src/named.ts", b"function run() {}\nexport default run;\n");
    assert!(named.bindings.iter().any(|binding| {
        binding.spelling == "default" && binding.kind == compass_languages::BindingKind::Reexport
    }));
}

#[test]
fn candidate_evidence_is_deterministic_and_parser_recovery_is_diagnosed() {
    let source = b"export function broken( { return 1;\n";
    let first = candidate("src/broken.ts", source);
    let second = candidate("src/broken.ts", source);
    assert_eq!(first, second);
    assert!(
        first
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "partial_parser_recovery")
    );
    assert!(first.declarations.iter().all(|declaration| {
        declaration.range.start_byte <= declaration.range.end_byte
            && usize::try_from(declaration.range.end_byte).unwrap_or(usize::MAX) <= source.len()
    }));
}

#[test]
fn candidate_resolves_only_proven_nominal_receivers_and_exact_members() {
    let source = br#"class Widget {
    field = 1;
    run() { this.field; }
    invoke() { this.run(); }
}

const object = new Widget();
object.run();
object.field;
object["field"];
const key = "field";
object[key];
function run() {}
unknown.run();
"#;
    let batch = candidate("src/widget.ts", source);
    validate_evidence(&batch, EvidenceLimits::default()).expect("valid evidence");

    let run = batch
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name.ends_with(".Widget.run"))
        .expect("method declaration");
    let field = batch
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name.ends_with(".Widget.field"))
        .expect("field declaration");

    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::Calls
            && candidate.target_spelling == "run"
            && candidate.constraints.exact_target_declaration_id.as_deref() == Some(run.id.as_str())
    }));
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::AccessesMember
            && candidate.target_spelling == "field"
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(field.id.as_str())
    }));
    assert_eq!(
        batch
            .candidates
            .iter()
            .filter(|candidate| {
                candidate.relation == compass_languages::CandidateRelation::AccessesMember
                    && candidate.target_spelling == "field"
                    && candidate.constraints.exact_target_declaration_id.as_deref()
                        == Some(field.id.as_str())
            })
            .count(),
        3,
        "this, dot, and literal computed members resolve to the same field"
    );
    // `unknown.run()` is dynamic and must not be attributed to the unrelated
    // top-level `run` declaration.
    assert!(!batch.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::Calls
            && candidate.target_spelling == "run"
            && candidate.constraints.qualified_name.as_deref() == Some("run")
    }));
    assert!(!batch.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::AccessesMember
            && candidate.target_spelling == "key"
    }));

    let imported = candidate(
        "src/imported.ts",
        b"import * as api from \"./api\";\napi.run();\n",
    );
    let imported_call = imported
        .candidates
        .iter()
        .find(|candidate| {
            candidate.relation == compass_languages::CandidateRelation::Calls
                && candidate.target_spelling == "run"
        })
        .expect("namespace member call candidate");
    assert_eq!(
        imported_call.constraints.qualified_name.as_deref(),
        Some("./api::run")
    );
    assert!(imported_call.binding_id.is_some());

    let overloads = candidate(
        "src/overloads.ts",
        b"function run() {}\nfunction run(value: string) {}\nrun();\nrun(\"x\");\n",
    );
    let overload_calls = overloads
        .candidates
        .iter()
        .filter(|candidate| {
            candidate.relation == compass_languages::CandidateRelation::Calls
                && candidate.target_spelling == "run"
        })
        .collect::<Vec<_>>();
    assert_eq!(overload_calls.len(), 2);
    assert!(overload_calls.iter().any(|candidate| {
        candidate.constraints.argument_count == Some(0)
            && candidate.constraints.exact_target_declaration_id.is_some()
    }));
    assert!(overload_calls.iter().any(|candidate| {
        candidate.constraints.argument_count == Some(1)
            && candidate.constraints.exact_target_declaration_id.is_some()
    }));

    let typed_overloads = candidate(
        "src/typed-overloads.ts",
        b"function parse(value: string) {}\nfunction parse(value: number) {}\nparse(\"text\");\nparse(42);\n",
    );
    let parse_declarations = typed_overloads
        .declarations
        .iter()
        .filter(|declaration| declaration.name == "parse")
        .collect::<Vec<_>>();
    assert_eq!(parse_declarations.len(), 2);
    let parse_calls = typed_overloads
        .candidates
        .iter()
        .filter(|candidate| {
            candidate.relation == compass_languages::CandidateRelation::Calls
                && candidate.target_spelling == "parse"
        })
        .collect::<Vec<_>>();
    assert_eq!(parse_calls.len(), 2);
    assert!(parse_calls.iter().all(|candidate| {
        candidate.constraints.argument_types.len() == 1
            && candidate.constraints.argument_types[0].is_some()
            && candidate.constraints.exact_target_declaration_id.is_some()
    }));
    let string_parse = parse_calls
        .iter()
        .find(|candidate| candidate.constraints.argument_types[0].as_deref() == Some("string"))
        .expect("string overload");
    let number_parse = parse_calls
        .iter()
        .find(|candidate| candidate.constraints.argument_types[0].as_deref() == Some("number"))
        .expect("number overload");
    assert_ne!(
        string_parse.constraints.exact_target_declaration_id,
        number_parse.constraints.exact_target_declaration_id
    );

    let constructed = candidate(
        "src/constructed.ts",
        b"class Constructed { constructor(value: string) {} }\nnew Constructed(\"ok\");\n",
    );
    assert!(constructed.declarations.iter().any(|declaration| {
        declaration.kind == "constructor" && declaration.name == "constructor"
    }));
    assert!(constructed.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::Constructs
            && candidate.target_spelling == "Constructed"
            && candidate.constraints.argument_count == Some(1)
            && candidate.constraints.argument_types == [Some("string".to_owned())]
    }));

    let hierarchy = candidate(
        "src/hierarchy.ts",
        b"class Base { run() {} }\nclass Child extends Base { call() { super.run(); } }\n",
    );
    let base_run = hierarchy
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name.ends_with(".Base.run"))
        .expect("base method declaration");
    assert!(hierarchy.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::Calls
            && candidate.target_spelling == "run"
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(base_run.id.as_str())
    }));

    let imported_hierarchy = candidate(
        "src/imported-hierarchy.ts",
        b"import * as bases from \"./bases\";\nclass Child extends bases.Base { call() { super.run(); } }\n",
    );
    let imported_base = imported_hierarchy
        .candidates
        .iter()
        .find(|candidate| {
            candidate.relation == compass_languages::CandidateRelation::Extends
                && candidate.target_spelling == "Base"
        })
        .expect("qualified imported base candidate");
    assert_eq!(
        imported_base.constraints.qualified_name.as_deref(),
        Some("./bases::Base")
    );
    let imported_super = imported_hierarchy
        .candidates
        .iter()
        .find(|candidate| {
            candidate.relation == compass_languages::CandidateRelation::Calls
                && candidate.target_spelling == "run"
        })
        .expect("qualified imported super call");
    assert_eq!(
        imported_super.constraints.qualified_name.as_deref(),
        Some("./bases::Base.run")
    );

    let inherited_constructor = candidate(
        "src/inherited-constructor.ts",
        b"class Base { constructor(value: string, count: number) {} }\nclass Child extends Base {}\nnew Child(\"ok\", 1);\nnew Child();\n",
    );
    let child_constructions = inherited_constructor
        .candidates
        .iter()
        .filter(|candidate| {
            candidate.relation == compass_languages::CandidateRelation::Constructs
                && candidate.target_spelling == "Child"
        })
        .collect::<Vec<_>>();
    assert_eq!(child_constructions.len(), 2);
    assert!(child_constructions.iter().any(|candidate| {
        candidate.constraints.argument_count == Some(2)
            && candidate.constraints.exact_target_declaration_id.is_some()
    }));
    assert!(child_constructions.iter().any(|candidate| {
        candidate.constraints.argument_count == Some(0)
            && candidate.constraints.exact_target_declaration_id.is_none()
    }));

    let arity_negative = candidate(
        "src/arity.ts",
        b"function exact(value: string) {}\nexact();\nexact(\"ok\");\nfunction optional(value?: string) {}\noptional(\"ok\");\n",
    );
    let exact_calls = arity_negative
        .candidates
        .iter()
        .filter(|candidate| {
            candidate.relation == compass_languages::CandidateRelation::Calls
                && candidate.target_spelling == "exact"
        })
        .collect::<Vec<_>>();
    assert_eq!(
        exact_calls.len(),
        2,
        "the incompatible call remains unresolved"
    );
    assert!(exact_calls.iter().any(|candidate| {
        candidate.constraints.argument_count == Some(1)
            && candidate.constraints.exact_target_declaration_id.is_some()
    }));
    assert!(exact_calls.iter().any(|candidate| {
        candidate.constraints.argument_count == Some(0)
            && candidate.constraints.exact_target_declaration_id.is_none()
    }));
    assert!(arity_negative.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::Calls
            && candidate.target_spelling == "optional"
            && candidate.constraints.argument_count == Some(1)
    }));

    let private = candidate(
        "src/private.ts",
        b"class Secret { #run() {} call() { this.#run(); } }\nnew Secret().call();\n",
    );
    let private_run = private
        .declarations
        .iter()
        .find(|declaration| declaration.name == "#run")
        .expect("private method declaration");
    assert!(private.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::Calls
            && candidate.target_spelling == "#run"
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(private_run.id.as_str())
    }));
}

#[test]
fn javascript_candidate_infers_object_literal_members_and_lexical_calls() {
    let source = br#"const config = {
    legacyAgreements: { Stytch: "gold" },
    sponsorsToIgnore: ["axios"],
};
const frozen = { active: true } as const;
function send404(response, body) { response.end(body); }
class Server {
    static handle(response) { send404(response); }
}

config.legacyAgreements;
config.sponsorsToIgnore;
frozen.active;
"#;
    let batch = candidate("src/server.js", source);
    validate_evidence(&batch, EvidenceLimits::default()).expect("valid evidence");

    let agreements = batch
        .declarations
        .iter()
        .find(|declaration| {
            declaration
                .qualified_name
                .ends_with(".config.legacyAgreements")
        })
        .expect("object-literal property declaration");
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::AccessesMember
            && candidate.target_spelling == "legacyAgreements"
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(agreements.id.as_str())
    }));
    let ignored = batch
        .declarations
        .iter()
        .find(|declaration| {
            declaration
                .qualified_name
                .ends_with(".config.sponsorsToIgnore")
        })
        .expect("array-valued object-literal property declaration");
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::AccessesMember
            && candidate.target_spelling == "sponsorsToIgnore"
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(ignored.id.as_str())
    }));
    let active = batch
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name.ends_with(".frozen.active"))
        .expect("as-const object property declaration");
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::AccessesMember
            && candidate.target_spelling == "active"
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(active.id.as_str())
    }));

    let send404 = batch
        .declarations
        .iter()
        .find(|declaration| declaration.name == "send404")
        .expect("function declaration");
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::Calls
            && candidate.target_spelling == "send404"
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(send404.id.as_str())
    }));
}

#[test]
fn javascript_flow_assignment_object_literal_selects_exact_members() {
    let batch = candidate(
        "src/object-reassignment.js",
        br#"function read(flag) {
    let auth = undefined;
    auth = { username: "user", password: "secret" };
    return auth.username;
}
"#,
    );
    let username = batch
        .declarations
        .iter()
        .find(|declaration| declaration.name == "username")
        .expect("object assignment member declaration");
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::AccessesMember
            && candidate.target_spelling == "username"
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(username.id.as_str())
    }));
}

#[test]
fn javascript_flow_assignment_object_literal_branch_fails_closed() {
    let batch = candidate(
        "src/object-branch-reassignment.js",
        br#"function read(flag) {
    let auth = undefined;
    if (flag) {
        auth = { username: "user" };
    }
    return auth.username;
}
"#,
    );
    assert!(!batch.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::AccessesMember
            && candidate.target_spelling == "username"
            && candidate.constraints.exact_target_declaration_id.is_some()
    }));
}

#[test]
fn javascript_object_literal_methods_resolve_this_members() {
    let batch = candidate(
        "src/object-this.js",
        br#"const api = {
    write() {},
    remove() { this.write(); }
};
api.remove();
"#,
    );
    let write = batch
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name == "object-this.api.write")
        .expect("object method declaration");
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Calls
            && candidate.target_spelling == "write"
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(write.id.as_str())
    }));

    let spread = candidate(
        "src/object-this-spread.js",
        br#"const base = { write() {} };
const api = {
    ...base,
    remove() { this.write(); }
};
api.remove();
"#,
    );
    assert!(!spread.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Calls
            && candidate.target_spelling == "write"
            && candidate.constraints.exact_target_declaration_id.is_some()
    }));
}

#[test]
fn javascript_object_flow_member_reads_and_literal_writes_resolve_exact_targets() {
    let batch = candidate(
        "src/object-flow.js",
        br#"const response = { request: requestValue, data: null };
const later = () => {
    response.data = responseValue;
    return response.request;
};
later();

const key = Symbol('internals');
class Stream {
    constructor() {
        const internals = (this[key] = { isCaptured: false });
        this.on('newListener', () => {
            if (!internals.isCaptured) {
                internals.isCaptured = true;
            }
        });
    }
}
new Stream();
"#,
    );
    let request = batch
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name == "object-flow.response.request")
        .expect("response.request declaration");
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::AccessesMember
            && candidate.target_spelling == "request"
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(request.id.as_str())
    }));
    let is_captured = batch
        .declarations
        .iter()
        .find(|declaration| {
            declaration.qualified_name == "object-flow.Stream.constructor.internals.isCaptured"
        })
        .expect("internals.isCaptured declaration");
    assert_eq!(
        batch
            .candidates
            .iter()
            .filter(|candidate| {
                candidate.relation == CandidateRelation::AccessesMember
                    && candidate.target_spelling == "isCaptured"
                    && candidate.constraints.exact_target_declaration_id.as_deref()
                        == Some(is_captured.id.as_str())
            })
            .count(),
        2
    );

    let nested = candidate(
        "src/object-flow-nested.js",
        br#"function register(transport, data) {
    transport.request({}, function handleResponse(res) {
        const response = {
            status: res.status,
            headers: new Headers(res.headers),
            request: res.request,
        };
        response.data = data;
        if (streaming) {
            settle(resolve, reject, response);
        } else {
            function handleStreamEnd() {
                try {
                    return consume(response.request, response);
                } catch (err) {
                    return err;
                }
            }
            handleStreamEnd();
        }
    });
}
"#,
    );
    let nested_request = nested
        .declarations
        .iter()
        .find(|declaration| {
            declaration.qualified_name == "object-flow-nested.register.response.request"
        })
        .expect("nested response.request declaration");
    assert!(nested.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::AccessesMember
            && candidate.target_spelling == "request"
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(nested_request.id.as_str())
    }));

    let escaped = candidate(
        "src/object-flow-escape.js",
        br#"const response = { request: requestValue };
consume(response);
response.request;
"#,
    );
    let escaped_request = escaped
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name == "object-flow-escape.response.request")
        .expect("escaped response.request declaration");
    assert!(!escaped.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::AccessesMember
            && candidate.target_spelling == "request"
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(escaped_request.id.as_str())
    }));

    let nested_escape = candidate(
        "src/object-flow-nested-escape.js",
        br#"const response = { request: requestValue };
const later = () => consume(response);
later();
response.request;
"#,
    );
    let nested_escape_request = nested_escape
        .declarations
        .iter()
        .find(|declaration| {
            declaration.qualified_name == "object-flow-nested-escape.response.request"
        })
        .expect("nested escaped response.request declaration");
    assert!(!nested_escape.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::AccessesMember
            && candidate.target_spelling == "request"
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(nested_escape_request.id.as_str())
    }));
}

#[test]
fn javascript_stable_object_property_assignments_resolve_exact_members() {
    let batch = candidate(
        "src/stable-object-assignment.js",
        br#"const validators = {};
validators.transitional = function transitional() {};
validators.spelling = (value) => value;
validators.transitional(false);
validators.spelling('value');
"#,
    );
    let transitional = batch
        .declarations
        .iter()
        .find(|declaration| declaration.name == "transitional")
        .expect("transitional property declaration");
    let spelling = batch
        .declarations
        .iter()
        .find(|declaration| declaration.name == "spelling")
        .expect("spelling property declaration");
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::AccessesMember
            && candidate.target_spelling == "transitional"
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(transitional.id.as_str())
    }));
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Calls
            && candidate.target_spelling == "transitional"
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(transitional.id.as_str())
    }));
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == CandidateRelation::Calls
            && candidate.target_spelling == "spelling"
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(spelling.id.as_str())
    }));
}

#[test]
fn candidate_emits_curated_external_builtin_evidence_and_respects_shadowing() {
    let batch = candidate(
        "src/builtins.ts",
        br#"Array.from(items);
console.log("ready");
new Date();
new Set().add(1);
new Map().get(key);
new Date().getTime();
Error("broken");
"#,
    );
    validate_evidence(&batch, EvidenceLimits::default()).expect("valid builtin evidence");

    let external = |relation, spelling: &str, qualified: &str| {
        batch.candidates.iter().any(|candidate| {
            candidate.relation == relation
                && candidate.target_spelling == spelling
                && candidate.constraints.allow_external
                && candidate.constraints.module_or_package.as_deref() == Some("javascript.global")
                && candidate.constraints.qualified_name.as_deref() == Some(qualified)
        })
    };
    assert!(external(
        compass_languages::CandidateRelation::Calls,
        "from",
        "global::Array.from"
    ));
    assert!(external(
        compass_languages::CandidateRelation::Calls,
        "log",
        "global::console.log"
    ));
    assert!(external(
        compass_languages::CandidateRelation::Constructs,
        "Date",
        "global::Date"
    ));
    assert!(external(
        compass_languages::CandidateRelation::Calls,
        "add",
        "global::Set.add"
    ));
    assert!(external(
        compass_languages::CandidateRelation::Calls,
        "get",
        "global::Map.get"
    ));
    assert!(external(
        compass_languages::CandidateRelation::Calls,
        "getTime",
        "global::Date.getTime"
    ));
    assert!(external(
        compass_languages::CandidateRelation::Calls,
        "Error",
        "global::Error"
    ));
    let invalid_instance_member = candidate(
        "src/invalid-builtin-member.ts",
        b"new ArrayBuffer().add(1);\nnew WeakMap().values();\n",
    );
    assert!(!invalid_instance_member.candidates.iter().any(|candidate| {
        candidate.constraints.module_or_package.as_deref() == Some("javascript.global")
            && (candidate.target_spelling == "add" || candidate.target_spelling == "values")
    }));

    let shadowed = candidate(
        "src/shadowed-builtins.ts",
        b"const Array = { from() {} };\nArray.from(items);\n",
    );
    assert!(!shadowed.candidates.iter().any(|candidate| {
        candidate.constraints.module_or_package.as_deref() == Some("javascript.global")
    }));
}

#[test]
fn candidate_declares_for_of_bindings_with_exact_source_anchors() {
    let batch = candidate(
        "src/loops.ts",
        b"for (const _ of DATA) consume(_);\nfor (const { id } of rows) consume(id);\n",
    );
    let loop_bindings = batch
        .declarations
        .iter()
        .filter(|declaration| declaration.kind == "variable")
        .collect::<Vec<_>>();
    assert_eq!(
        loop_bindings
            .iter()
            .filter(|declaration| declaration.name == "_")
            .count(),
        1
    );
    assert!(loop_bindings.iter().any(|declaration| {
        declaration.name == "id"
            && declaration.range.start_byte
                == b"for (const _ of DATA) consume(_);\nfor (const { ".len() as u64
    }));

    let callbacks = candidate(
        "src/callbacks.js",
        b"rows.find((activeSponsor) => activeSponsor.slug === target);\n",
    );
    assert!(callbacks.declarations.iter().any(|declaration| {
        declaration.kind == "parameter" && declaration.name == "activeSponsor"
    }));

    let unparenthesized = candidate(
        "src/unparenthesized.js",
        b"rows.find(activeSponsor => activeSponsor.slug === target);\n",
    );
    assert!(unparenthesized.declarations.iter().any(|declaration| {
        declaration.kind == "parameter" && declaration.name == "activeSponsor"
    }));

    let caught = candidate(
        "src/catch.js",
        b"try { run(); } catch (error) { log(error); }\n",
    );
    assert!(
        caught
            .declarations
            .iter()
            .any(|declaration| { declaration.kind == "variable" && declaration.name == "error" })
    );
}

#[test]
fn javascript_callable_values_are_references_not_indirect_calls() {
    let source = br#"function onValue(value) {}
const alias = onValue;
const alias2 = alias;
const list = [onValue, alias2];
const object = { handler: alias };
consume(onValue);
consume(alias2);
consume(list[0]);
consume(object.handler);
const nonCallable = 1;
consume(nonCallable);
const maybe = Math.random() ? onValue : nonCallable;
consume(maybe);
"#;
    let batch = candidate("src/callback-values.js", source);
    validate_evidence(&batch, EvidenceLimits::default()).expect("callback value evidence");

    let on_value = batch
        .declarations
        .iter()
        .find(|declaration| declaration.name == "onValue" && declaration.kind == "function")
        .expect("callable source declaration");
    let alias2 = batch
        .declarations
        .iter()
        .find(|declaration| declaration.name == "alias2" && declaration.kind == "variable")
        .expect("second callable alias");
    let non_callable = batch
        .declarations
        .iter()
        .find(|declaration| declaration.name == "nonCallable")
        .expect("non-callable value");

    let references_to = |target: &str| {
        batch
            .candidates
            .iter()
            .filter(|candidate| {
                candidate.relation == CandidateRelation::References
                    && candidate.constraints.exact_target_declaration_id.as_deref() == Some(target)
            })
            .count()
    };
    assert!(
        references_to(&on_value.id) >= 2,
        "onValue callback references"
    );
    assert!(references_to(&alias2.id) >= 2, "alias2 callback references");
    assert_eq!(references_to(&non_callable.id), 0);
    assert!(
        !batch
            .candidates
            .iter()
            .any(|candidate| candidate.relation == CandidateRelation::IndirectCalls)
    );
}

#[test]
fn candidate_resolves_qualified_and_builtin_type_references_without_fallback_guesses() {
    let batch = candidate(
        "src/types.ts",
        br#"import * as Benchmark from "benchmark";
type Event = Benchmark.Event;
type Pending = Promise<string>;
type Context = ThisType<{ value: string }>;
type ElementRef = HTMLDivElement;
type Match = RegExpExecArray;
type ReactElement = React.ReactElement;
type NodeBuffer = Buffer;
type Box<T> = { value: T };
type Pair<T> = { left: T; right: T };
type Mapped<T> = { [Key in T]: string };
type Indexed = { [key: string]: unknown };
type WithThis = (this: Context, value: string) => void;
type LastOf<T> = T extends unknown ? infer Last : never;
type ParseFn = (schema: unknown, _params?: string) => void;
const typed: (_params?: string) => void = (_value) => {};
"#,
    );
    validate_evidence(&batch, EvidenceLimits::default()).expect("valid type evidence");
    let type_refs = batch
        .candidates
        .iter()
        .filter(|candidate| {
            candidate.relation == compass_languages::CandidateRelation::References
                && candidate
                    .occurrence_id
                    .as_ref()
                    .is_some_and(|occurrence_id| {
                        batch.occurrences.iter().any(|occurrence| {
                            occurrence.id == *occurrence_id
                                && occurrence.context.as_deref() == Some("type")
                        })
                    })
        })
        .collect::<Vec<_>>();
    let qualified = type_refs
        .iter()
        .find(|candidate| candidate.target_spelling == "Benchmark.Event")
        .expect("qualified imported type reference");
    assert_eq!(
        qualified.constraints.qualified_name.as_deref(),
        Some("benchmark::Event")
    );
    assert!(qualified.binding_id.is_some());
    for (spelling, target) in [
        ("Promise", "global::Promise"),
        ("ThisType", "typescript.lib::ThisType"),
        ("HTMLDivElement", "dom.lib::HTMLDivElement"),
        ("RegExpExecArray", "typescript.lib::RegExpExecArray"),
        ("React.ReactElement", "@types/react::React.ReactElement"),
        ("Buffer", "node.global::Buffer"),
    ] {
        let builtin = type_refs
            .iter()
            .find(|candidate| candidate.target_spelling == spelling)
            .expect("builtin type reference");
        assert!(builtin.constraints.allow_external);
        assert_eq!(builtin.constraints.qualified_name.as_deref(), Some(target));
    }
    assert!(type_refs.iter().any(|candidate| {
        candidate.target_spelling == "T"
            && candidate.constraints.exact_target_declaration_id.is_some()
    }));
    assert!(
        batch.declarations.iter().any(|declaration| {
            declaration.kind == "type_parameter" && declaration.name == "Key"
        })
    );
    assert!(
        batch
            .declarations
            .iter()
            .any(|declaration| { declaration.kind == "parameter" && declaration.name == "key" })
    );
    assert!(
        batch
            .declarations
            .iter()
            .any(|declaration| { declaration.kind == "parameter" && declaration.name == "this" })
    );
    assert!(
        batch.declarations.iter().any(|declaration| {
            declaration.kind == "type_parameter" && declaration.name == "Last"
        })
    );
    assert!(
        batch.declarations.iter().any(|declaration| {
            declaration.kind == "parameter" && declaration.name == "_params"
        })
    );

    let shadowed = candidate(
        "src/shadowed-type.ts",
        b"type Promise = { then(): void };\ntype Local = Promise;\n",
    );
    assert!(!shadowed.candidates.iter().any(|candidate| {
        candidate
            .occurrence_id
            .as_ref()
            .is_some_and(|occurrence_id| {
                shadowed.occurrences.iter().any(|occurrence| {
                    occurrence.id == *occurrence_id && occurrence.context.as_deref() == Some("type")
                })
            })
            && candidate.constraints.module_or_package.as_deref() == Some("javascript.global")
    }));
}

#[test]
fn candidate_preserves_dynamic_calls_and_proven_super_and_type_member_evidence() {
    let batch = candidate(
        "src/advanced.ts",
        br#"import * as schemas from "./schemas";
interface Coerced extends schemas._ZodString {}
class Base { constructor(value: string) {} }
class Child extends Base { constructor(value: string) { super(value); } }
const encoded = (encoder => encoder.encode)(new TextEncoder());
const dynamicMember = resolvers[method]();
const dynamicConstructor = new (factory || Base)();
"#,
    );
    validate_evidence(&batch, EvidenceLimits::default()).expect("advanced evidence");

    let type_member = batch
        .candidates
        .iter()
        .find(|candidate| {
            candidate.relation == compass_languages::CandidateRelation::AccessesMember
                && candidate.target_spelling == "_ZodString"
        })
        .expect("qualified heritage member access");
    assert_eq!(
        type_member.constraints.qualified_name.as_deref(),
        Some("./schemas::_ZodString")
    );
    assert!(
        type_member
            .occurrence_id
            .as_ref()
            .and_then(|id| batch
                .occurrences
                .iter()
                .find(|occurrence| occurrence.id == *id))
            .is_some_and(|occurrence| occurrence.context.as_deref() == Some("type_member"))
    );

    let base = batch
        .declarations
        .iter()
        .find(|declaration| declaration.name == "Base" && declaration.kind == "class")
        .expect("base declaration");
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::Calls
            && candidate.target_spelling == "super"
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(base.id.as_str())
    }));

    for context in ["dynamic_call", "dynamic_member_call", "dynamic_new"] {
        assert!(
            batch.candidates.iter().any(|candidate| {
                (candidate.relation == compass_languages::CandidateRelation::Calls
                    || candidate.relation == compass_languages::CandidateRelation::Constructs)
                    && candidate
                        .occurrence_id
                        .as_ref()
                        .and_then(|id| {
                            batch
                                .occurrences
                                .iter()
                                .find(|occurrence| occurrence.id == *id)
                        })
                        .is_some_and(|occurrence| occurrence.context.as_deref() == Some(context))
            }),
            "missing dynamic context {context}"
        );
    }
}

#[test]
fn javascript_candidate_keeps_block_bindings_distinct_and_hoists_var() {
    let batch = candidate(
        "src/blocks.js",
        br#"function run(flag) {
  if (flag) {
    const callback = () => 1;
    callback();
  } else {
    const callback = () => 2;
    callback();
  }
  var hoisted = () => 3;
  if (flag) {
    hoisted();
  }
}
"#,
    );
    validate_evidence(&batch, EvidenceLimits::default()).expect("block evidence");

    let mut call_target_names = batch
        .candidates
        .iter()
        .filter(|candidate| candidate.relation == compass_languages::CandidateRelation::Calls)
        .filter_map(|candidate| candidate.constraints.exact_target_declaration_id.as_deref())
        .filter_map(|id| {
            batch
                .declarations
                .iter()
                .find(|declaration| declaration.id == id)
        })
        .map(|declaration| declaration.name.as_str())
        .collect::<Vec<_>>();
    call_target_names.sort_unstable();
    assert_eq!(call_target_names, ["callback", "callback", "hoisted"]);
    assert!(batch.scopes.iter().any(|scope| scope.kind == "block"));
    assert!(batch.scopes.iter().any(|scope| scope.kind == "function"));
}

#[test]
fn object_literal_properties_do_not_shadow_unqualified_calls() {
    let batch = candidate(
        "src/object-shadow.js",
        br#"const object = { run: () => 1 };
run();
"#,
    );
    assert!(!batch.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::Calls
            && candidate.constraints.exact_target_declaration_id.is_some()
    }));
}

#[test]
fn candidate_tracks_type_alias_enum_and_string_named_members() {
    let source = br#"type Config = { name: string };
function makeConfig(): Config { return { name: "ready" }; }
const config = makeConfig();
config.name;

enum Status { Ready = "ready", Failed = "failed" }
Status.Ready;

class Box {
    "quoted" = 1;
    read() { this["quoted"]; }
}

new Box().read();
"#;
    let batch = candidate("src/typed-members.ts", source);
    validate_evidence(&batch, EvidenceLimits::default()).expect("typed member evidence");

    let config_name = batch
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name.ends_with(".Config.name"))
        .expect("type-literal member");
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::AccessesMember
            && candidate.target_spelling == "name"
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(config_name.id.as_str())
    }));

    let ready = batch
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name.ends_with(".Status.Ready"))
        .expect("enum member");
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::AccessesMember
            && candidate.target_spelling == "Ready"
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(ready.id.as_str())
    }));

    let quoted = batch
        .declarations
        .iter()
        .find(|declaration| {
            declaration.qualified_name.ends_with(".Box.quoted")
                && declaration.range.start_byte < source.len() as u64
        })
        .expect("string-named class member");
    assert!(batch.candidates.iter().any(|candidate| {
        candidate.relation == compass_languages::CandidateRelation::AccessesMember
            && candidate.target_spelling == "quoted"
            && candidate.constraints.exact_target_declaration_id.as_deref()
                == Some(quoted.id.as_str())
    }));
}

#[test]
fn typescript_candidate_preserves_imported_type_receiver_for_member_evidence() {
    let batch = candidate(
        "src/consumer.ts",
        br#"import type { Config } from "./types";
export function use(config: Config) { config.inspect(); }
"#,
    );
    validate_evidence(&batch, EvidenceLimits::default()).expect("imported member evidence");
    let binding = batch
        .bindings
        .iter()
        .find(|binding| binding.spelling == "Config")
        .expect("imported type binding");
    let candidates = batch
        .candidates
        .iter()
        .filter(|candidate| {
            matches!(
                candidate.relation,
                CandidateRelation::Calls | CandidateRelation::AccessesMember
            ) && candidate.target_spelling == "inspect"
        })
        .collect::<Vec<_>>();
    assert_eq!(candidates.len(), 2);
    for candidate in candidates {
        assert_eq!(candidate.binding_id.as_deref(), Some(binding.id.as_str()));
        assert_eq!(
            candidate.constraints.qualified_name.as_deref(),
            Some("./types::Config.inspect")
        );
    }
}
