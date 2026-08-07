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
    assert_eq!(batch.adapter.version, 2);
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
    assert_eq!(batch.adapter.version, 2);
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
    assert_eq!(helper.namespace, Some(SymbolNamespace::Value));
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
