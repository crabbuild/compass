#[test]
fn javascript_package_exports_choose_import_and_require_conditions()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let package = directory.path().join("packages/conditional/package.json");
    let import_target = directory.path().join("packages/conditional/src/import.ts");
    let require_target = directory
        .path()
        .join("packages/conditional/src/require.cjs");
    let wildcard_target = directory
        .path()
        .join("packages/conditional/src/features/button.ts");
    let typescript_consumer = directory.path().join("app/consumer.ts");
    let javascript_consumer = directory.path().join("app/consumer.cjs");
    let package_source = br##"{
        "name": "@example/conditional",
        "exports": {
            ".": {
                "import": "./src/import.ts",
                "require": "./src/require.cjs",
                "default": "./src/fallback.js"
            },
            "./features/*": {
                "import": "./src/features/*.ts"
            },
            "./fallback": ["./src/features/missing.ts", "./src/features/button.ts"]
        }
    }"##;
    let import_source = br#"export const imported = true;"#;
    let require_source = br#"module.exports = { required: true };"#;
    let wildcard_source = br#"export const button = true;"#;
    let typescript_source = br#"import { imported } from "@example/conditional";
import { button } from "@example/conditional/features/button";
import { button as fallback } from "@example/conditional/fallback";
export const value = imported && button && fallback;
"#;
    let javascript_source = br#"const { required } = require("@example/conditional");
module.exports = required;
"#;
    for (path, source) in [
        (&package, package_source.as_slice()),
        (&import_target, import_source.as_slice()),
        (&require_target, require_source.as_slice()),
        (&wildcard_target, wildcard_source.as_slice()),
        (&typescript_consumer, typescript_source.as_slice()),
        (&javascript_consumer, javascript_source.as_slice()),
    ] {
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(path, source)?;
    }
    let extractions = [
        extract(
            package.to_str().ok_or("non-UTF-8 fixture path")?,
            package_source,
        ),
        extract(
            import_target.to_str().ok_or("non-UTF-8 fixture path")?,
            import_source,
        ),
        extract(
            require_target.to_str().ok_or("non-UTF-8 fixture path")?,
            require_source,
        ),
        extract(
            wildcard_target.to_str().ok_or("non-UTF-8 fixture path")?,
            wildcard_source,
        ),
        extract(
            typescript_consumer
                .to_str()
                .ok_or("non-UTF-8 fixture path")?,
            typescript_source,
        ),
        extract(
            javascript_consumer
                .to_str()
                .ok_or("non-UTF-8 fixture path")?,
            javascript_source,
        ),
    ];
    let sources = [
        (&package, package_source.as_slice()),
        (&import_target, import_source.as_slice()),
        (&require_target, require_source.as_slice()),
        (&wildcard_target, wildcard_source.as_slice()),
        (&typescript_consumer, typescript_source.as_slice()),
        (&javascript_consumer, javascript_source.as_slice()),
    ]
    .into_iter()
    .map(|(path, source)| {
        Ok((
            path.to_str().ok_or("non-UTF-8 fixture path")?.to_owned(),
            String::from_utf8(source.to_vec())?,
        ))
    })
    .collect::<Result<HashMap<_, _>, Box<dyn std::error::Error>>>()?;

    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, directory.path());
    assert_eq!(resolved.error, None);
    for target in [&import_target, &require_target, &wildcard_target] {
        assert!(
            resolved
                .nodes
                .iter()
                .any(|node| source_matches(&node.string("source_file"), target))
        );
    }
    let module_edges = resolved
        .edges
        .iter()
        .filter(|edge| edge.string("relation") == "imports_from")
        .collect::<Vec<_>>();
    assert!(module_edges.iter().any(|edge| {
        source_matches(&edge.string("source_file"), &typescript_consumer)
            && target_source_matches(&resolved, &edge.target, &import_target)
            && edge.string("package_condition") == "import"
            && edge.string("resolution_rule") == "project-module-binding"
    }));
    assert!(module_edges.iter().any(|edge| {
        source_matches(&edge.string("source_file"), &typescript_consumer)
            && target_source_matches(&resolved, &edge.target, &wildcard_target)
            && edge.string("package_condition") == "import"
    }));
    assert!(module_edges.iter().any(|edge| {
        source_matches(&edge.string("source_file"), &typescript_consumer)
            && edge.string("module") == "@example/conditional/fallback"
            && target_source_matches(&resolved, &edge.target, &wildcard_target)
            && edge.string("package_condition") == "default"
    }));
    assert!(
        module_edges.iter().any(|edge| {
            source_matches(&edge.string("source_file"), &javascript_consumer)
                && target_source_matches(&resolved, &edge.target, &require_target)
                && edge.string("context") == "require"
                && edge.string("package_condition") == "require"
        }),
        "module_edges={module_edges:#?}"
    );
    Ok(())
}

#[test]
fn javascript_jsconfig_base_url_resolves_bare_module() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let config = root.join("jsconfig.json");
    let implementation = root.join("src/api.js");
    let consumer = root.join("app/consumer.js");
    for path in [&config, &implementation, &consumer] {
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
    }
    let config_source = br#"{"compilerOptions":{"baseUrl":"."}}"#;
    let implementation_source = br#"export function api() { return true; }"#;
    let consumer_source = br#"import { api } from "src/api";
export const value = api();
"#;
    for (path, source) in [
        (&config, config_source.as_slice()),
        (&implementation, implementation_source.as_slice()),
        (&consumer, consumer_source.as_slice()),
    ] {
        fs::write(path, source)?;
    }
    let extractions = [
        extract(
            implementation.to_str().ok_or("non-UTF-8 fixture path")?,
            implementation_source,
        ),
        extract(
            consumer.to_str().ok_or("non-UTF-8 fixture path")?,
            consumer_source,
        ),
    ];
    let sources = [
        (&config, config_source.as_slice()),
        (&implementation, implementation_source.as_slice()),
        (&consumer, consumer_source.as_slice()),
    ]
    .into_iter()
    .map(|(path, source)| {
        Ok((
            path.to_str().ok_or("non-UTF-8 fixture path")?.to_owned(),
            String::from_utf8(source.to_vec())?,
        ))
    })
    .collect::<Result<HashMap<_, _>, Box<dyn std::error::Error>>>()?;

    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert_eq!(resolved.error, None);
    assert!(
        resolved
            .nodes
            .iter()
            .any(|node| source_matches(&node.string("source_file"), &implementation))
    );
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "imports_from"
            && edge.string("module") == "src/api"
            && target_source_matches(&resolved, &edge.target, &implementation)
            && edge.string("resolution_rule") == "project-module-binding"
    }),);
    Ok(())
}

#[test]
fn javascript_relative_named_imports_repoint_to_the_unique_export()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let target = root.join("src/target.ts");
    let consumer = root.join("src/consumer.ts");
    let target_source = br#"export function greet() { return "hello"; }"#;
    let consumer_source = br#"import { greet } from "./target.js";
export function run() { return greet(); }
"#;
    for (path, source) in [
        (&target, target_source.as_slice()),
        (&consumer, consumer_source.as_slice()),
    ] {
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(path, source)?;
    }
    let extractions = [
        extract(
            target.to_str().ok_or("non-UTF-8 fixture path")?,
            target_source,
        ),
        extract(
            consumer.to_str().ok_or("non-UTF-8 fixture path")?,
            consumer_source,
        ),
    ];
    let sources = [
        (&target, target_source.as_slice()),
        (&consumer, consumer_source.as_slice()),
    ]
    .into_iter()
    .map(|(path, source)| {
        Ok((
            path.to_str().ok_or("non-UTF-8 fixture path")?.to_owned(),
            String::from_utf8(source.to_vec())?,
        ))
    })
    .collect::<Result<HashMap<_, _>, Box<dyn std::error::Error>>>()?;

    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert_eq!(resolved.error, None);
    let greet = resolved
        .nodes
        .iter()
        .find(|node| {
            node.label() == "greet()" && source_matches(&node.string("source_file"), &target)
        })
        .ok_or("missing greet declaration")?;
    let import = resolved
        .edges
        .iter()
        .find(|edge| {
            edge.string("relation") == "imports_from"
                && edge.string("module") == "./target.js"
                && edge.string("local_name") == "greet"
        })
        .ok_or("missing relative named import")?;
    assert_eq!(import.target, greet.id);
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.target == greet.id
            && source_matches(&edge.string("source_file"), &consumer)
    }));
    Ok(())
}

#[test]
fn javascript_package_imports_use_the_nearest_package_and_type_condition()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let package = root.join("packages/toolkit/package.json");
    let implementation = root.join("packages/toolkit/src/internal/tool.ts");
    let consumer = root.join("packages/toolkit/src/consumer.ts");
    let package_source = br##"{
  "name": "@example/toolkit",
  "imports": {
    "#internal/*": {
      "types": "./src/internal/*.ts",
      "default": "./src/internal/*.js"
    }
  }
}"##;
    let implementation_source = br#"export function tool() { return 1; }"#;
    let consumer_source = br##"import { tool } from "#internal/tool";
export const value = tool();
"##;
    for (path, source) in [
        (&package, package_source.as_slice()),
        (&implementation, implementation_source.as_slice()),
        (&consumer, consumer_source.as_slice()),
    ] {
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(path, source)?;
    }
    let extractions = [
        extract(
            package.to_str().ok_or("non-UTF-8 fixture path")?,
            package_source,
        ),
        extract(
            implementation.to_str().ok_or("non-UTF-8 fixture path")?,
            implementation_source,
        ),
        extract(
            consumer.to_str().ok_or("non-UTF-8 fixture path")?,
            consumer_source,
        ),
    ];
    let sources = [
        (&package, package_source.as_slice()),
        (&implementation, implementation_source.as_slice()),
        (&consumer, consumer_source.as_slice()),
    ]
    .into_iter()
    .map(|(path, source)| {
        Ok((
            path.to_str().ok_or("non-UTF-8 fixture path")?.to_owned(),
            String::from_utf8(source.to_vec())?,
        ))
    })
    .collect::<Result<HashMap<_, _>, Box<dyn std::error::Error>>>()?;

    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert_eq!(resolved.error, None);
    assert!(resolved.nodes.iter().any(|node| {
        node.label() == "tool()" && source_matches(&node.string("source_file"), &implementation)
    }));
    let module_import = resolved
        .edges
        .iter()
        .find(|edge| {
            edge.string("relation") == "imports_from" && edge.string("module") == "#internal/tool"
        })
        .ok_or("missing package imports edge")?;
    assert_eq!(
        module_import.string("resolution_rule"),
        "project-module-binding"
    );
    assert_eq!(
        module_import.string("project_resolution_rule"),
        "package-imports"
    );
    assert_eq!(module_import.string("package_condition"), "types");
    assert!(target_source_matches(
        &resolved,
        &module_import.target,
        &implementation
    ));
    Ok(())
}

#[test]
fn javascript_package_resolution_mode_respects_node10_and_classic()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let package = root.join("packages/mode/package.json");
    let conditional = root.join("packages/mode/conditional.ts");
    let legacy = root.join("packages/mode/legacy.ts");
    let node10_config = root.join("node10/tsconfig.json");
    let node10_consumer = root.join("node10/consumer.ts");
    let classic_config = root.join("classic/tsconfig.json");
    let classic_consumer = root.join("classic/consumer.ts");
    let package_source = br#"{
  "name": "@example/mode",
  "exports": { ".": "./conditional.ts" },
  "main": "./legacy.ts"
}"#;
    let conditional_source = br#"export const selected = "conditional";"#;
    let legacy_source = br#"export const selected = "legacy";"#;
    let node10_config_source = br#"{
  "compilerOptions": { "moduleResolution": "node10" }
}"#;
    let node10_consumer_source = br#"import { selected } from "@example/mode";
export const value = selected;
"#;
    let classic_config_source = br#"{
  "compilerOptions": { "moduleResolution": "classic" }
}"#;
    let classic_consumer_source = br#"import { selected } from "@example/mode";
export const value = selected;
"#;
    for (path, source) in [
        (&package, package_source.as_slice()),
        (&conditional, conditional_source.as_slice()),
        (&legacy, legacy_source.as_slice()),
        (&node10_config, node10_config_source.as_slice()),
        (&node10_consumer, node10_consumer_source.as_slice()),
        (&classic_config, classic_config_source.as_slice()),
        (&classic_consumer, classic_consumer_source.as_slice()),
    ] {
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(path, source)?;
    }
    let extractions = [
        extract(
            package.to_str().ok_or("non-UTF-8 fixture path")?,
            package_source,
        ),
        extract(
            conditional.to_str().ok_or("non-UTF-8 fixture path")?,
            conditional_source,
        ),
        extract(
            legacy.to_str().ok_or("non-UTF-8 fixture path")?,
            legacy_source,
        ),
        extract(
            node10_consumer.to_str().ok_or("non-UTF-8 fixture path")?,
            node10_consumer_source,
        ),
        extract(
            classic_consumer.to_str().ok_or("non-UTF-8 fixture path")?,
            classic_consumer_source,
        ),
    ];
    let sources = [
        (&package, package_source.as_slice()),
        (&conditional, conditional_source.as_slice()),
        (&legacy, legacy_source.as_slice()),
        (&node10_config, node10_config_source.as_slice()),
        (&node10_consumer, node10_consumer_source.as_slice()),
        (&classic_config, classic_config_source.as_slice()),
        (&classic_consumer, classic_consumer_source.as_slice()),
    ]
    .into_iter()
    .map(|(path, source)| {
        Ok((
            path.to_str().ok_or("non-UTF-8 fixture path")?.to_owned(),
            String::from_utf8(source.to_vec())?,
        ))
    })
    .collect::<Result<HashMap<_, _>, Box<dyn std::error::Error>>>()?;

    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert_eq!(resolved.error, None);
    let legacy_id = resolved
        .nodes
        .iter()
        .find(|node| source_matches(&node.string("source_file"), &legacy))
        .map(|node| node.id.clone())
        .ok_or("missing legacy package target")?;
    let conditional_id = resolved
        .nodes
        .iter()
        .find(|node| source_matches(&node.string("source_file"), &conditional))
        .map(|node| node.id.clone())
        .ok_or("missing conditional package target")?;
    let node10_import = resolved
        .edges
        .iter()
        .find(|edge| {
            edge.string("relation") == "imports_from"
                && source_matches(&edge.string("source_file"), &node10_consumer)
        })
        .ok_or("missing Node10 package import")?;
    assert!(target_source_matches(
        &resolved,
        &node10_import.target,
        &legacy
    ));
    assert!(!target_source_matches(
        &resolved,
        &node10_import.target,
        &conditional
    ));
    assert_eq!(
        node10_import.string("resolution_rule"),
        "project-module-binding"
    );
    assert_eq!(
        node10_import.string("project_resolution_rule"),
        "package-legacy"
    );
    assert_eq!(node10_import.string("package_condition"), "main");
    assert_eq!(node10_import.string("module_resolution"), "node10");

    let classic_import = resolved
        .edges
        .iter()
        .find(|edge| {
            edge.string("relation") == "imports_from"
                && source_matches(&edge.string("source_file"), &classic_consumer)
        })
        .ok_or("missing Classic package import")?;
    assert_ne!(classic_import.target, legacy_id);
    assert_ne!(classic_import.target, conditional_id);
    assert!(classic_import.attributes.get("target_file").is_none());
    assert_eq!(
        classic_import.string("resolution_rule"),
        "qualified-external"
    );
    assert_eq!(
        classic_import.string("project_resolution_rule"),
        "package-classic-unresolved"
    );
    assert_eq!(classic_import.string("module_resolution"), "classic");
    Ok(())
}

#[test]
fn javascript_default_object_exports_resolve_exact_imported_members()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/utils.js",
            br#"const isNumber = value => typeof value === 'number';
const isString = value => typeof value === 'string';
export default { isNumber, isString };
"#
            .as_slice(),
        ),
        (
            "lib/spread-default.js",
            br#"import base from './utils.js';
export default { ...base, isString: value => true };
"#
            .as_slice(),
        ),
        (
            "app/consumer.js",
            br#"import utils from '../lib/utils.js';
import { isNumber as named } from '../lib/utils.js';
import spread from '../lib/spread-default.js';
utils.isNumber(1);
utils.isString('value');
named(2);
spread.isString('value');
spread.isNumber(1);
"#
            .as_slice(),
        ),
    ];
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(path, source)?;
    }
    let batches = files
        .iter()
        .map(|(relative, source)| {
            Engine::default()
                .extract_source_universal_candidate_evidence(&root.join(relative), relative, source)
                .map_err(|error| format!("candidate extraction failed for {relative}: {error}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let utils_number = batches[0]
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name == "utils.default.isNumber")
        .ok_or("missing default object isNumber declaration")?;
    let utils_string = batches[0]
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name == "utils.default.isString")
        .ok_or("missing default object isString declaration")?;
    let spread_string = batches[1]
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name == "spread-default.default.isString")
        .ok_or("missing spread-default direct member declaration")?;
    let number_call = batches[2]
        .candidates
        .iter()
        .find(|candidate| {
            candidate.relation == CandidateRelation::Calls
                && candidate.target_spelling == "isNumber"
        })
        .ok_or("missing imported default-object isNumber call")?;
    let string_call = batches[2]
        .candidates
        .iter()
        .find(|candidate| {
            candidate.relation == CandidateRelation::Calls
                && candidate.target_spelling == "isString"
                && candidate
                    .constraints
                    .qualified_name
                    .as_deref()
                    .is_some_and(|qualified| qualified.contains("utils.js::default"))
        })
        .ok_or("missing imported default-object isString call")?;
    let spread_call = batches[2]
        .candidates
        .iter()
        .find(|candidate| {
            candidate.relation == CandidateRelation::Calls
                && candidate.target_spelling == "isString"
                && candidate
                    .constraints
                    .qualified_name
                    .as_deref()
                    .is_some_and(|qualified| qualified.contains("spread-default.js::default"))
        })
        .ok_or("missing spread-default member call")?;
    let spread_number_call = batches[2]
        .candidates
        .iter()
        .find(|candidate| {
            candidate.relation == CandidateRelation::Calls
                && candidate.target_spelling == "isNumber"
                && candidate
                    .constraints
                    .qualified_name
                    .as_deref()
                    .is_some_and(|qualified| qualified.contains("spread-default.js::default"))
        })
        .ok_or("missing spread-default inherited member call")?;
    let named_call = batches[2]
        .candidates
        .iter()
        .find(|candidate| {
            candidate.relation == CandidateRelation::Calls && candidate.target_spelling == "named"
        })
        .ok_or("missing named-import call")?;
    let index = UniversalResolutionIndex::new_with_inventory(
        &batches,
        &[],
        root,
        UniversalResolutionLimits::default(),
    )?;
    assert!(matches!(
        index.resolve(&number_call.id),
        compass_resolve::evidence::ResolutionDecision::Resolved { ref declaration_id, .. }
            if declaration_id == &utils_number.id
    ));
    assert!(matches!(
        index.resolve(&string_call.id),
        compass_resolve::evidence::ResolutionDecision::Resolved { ref declaration_id, .. }
            if declaration_id == &utils_string.id
    ));
    assert!(matches!(
        index.resolve(&spread_call.id),
        compass_resolve::evidence::ResolutionDecision::Resolved { ref declaration_id, .. }
            if declaration_id == &spread_string.id
    ));
    assert!(matches!(
        index.resolve(&spread_number_call.id),
        compass_resolve::evidence::ResolutionDecision::Resolved { ref declaration_id, .. }
            if declaration_id == &utils_number.id
    ));
    assert!(!matches!(
        index.resolve(&named_call.id),
        compass_resolve::evidence::ResolutionDecision::Resolved { .. }
    ));
    Ok(())
}

#[test]
fn javascript_default_object_spread_aliases_preserve_precedence_and_ambiguity()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/left.ts",
            br#"export default { isNumber: value => true };
"#
            .as_slice(),
        ),
        (
            "lib/right.js",
            br#"export default { isNumber: value => false };
"#
            .as_slice(),
        ),
        (
            "lib/ambiguous.js",
            br#"import left from './left.ts';
import right from './right.js';
export default { ...left, ...right, isString: value => true };
"#
            .as_slice(),
        ),
        (
            "lib/override.js",
            br#"import left from './left.ts';
export default { ...left, isNumber: value => false };
"#
            .as_slice(),
        ),
        (
            "lib/cross.js",
            br#"import left from './left.ts';
export default { ...left, isString: value => true };
"#
            .as_slice(),
        ),
        (
            "lib/function.js",
            br#"export default function callable(value) { return value; }
"#
            .as_slice(),
        ),
        (
            "lib/function-spread.js",
            br#"import callable from './function.js';
export default { ...callable, isString: value => true };
"#
            .as_slice(),
        ),
        (
            "app/consumer.js",
            br#"import ambiguous from '../lib/ambiguous.js';
import override from '../lib/override.js';
import cross from '../lib/cross.js';
import functionSpread from '../lib/function-spread.js';
ambiguous.isNumber(1);
override.isNumber(1);
cross.isNumber(1);
functionSpread.callable(1);
"#
            .as_slice(),
        ),
    ];
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(path, source)?;
    }
    let batches = files
        .iter()
        .map(|(relative, source)| {
            Engine::default()
                .extract_source_universal_candidate_evidence(&root.join(relative), relative, source)
                .map_err(|error| format!("candidate extraction failed for {relative}: {error}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let ambiguous_call = batches[7]
        .candidates
        .iter()
        .find(|candidate| {
            candidate.relation == CandidateRelation::Calls
                && candidate.target_spelling == "isNumber"
                && candidate
                    .constraints
                    .qualified_name
                    .as_deref()
                    .is_some_and(|qualified| qualified.contains("ambiguous.js::default"))
        })
        .ok_or("missing ambiguous spread call")?;
    let override_call = batches[7]
        .candidates
        .iter()
        .find(|candidate| {
            candidate.relation == CandidateRelation::Calls
                && candidate.target_spelling == "isNumber"
                && candidate
                    .constraints
                    .qualified_name
                    .as_deref()
                    .is_some_and(|qualified| qualified.contains("override.js::default"))
        })
        .ok_or("missing override spread call")?;
    let function_call = batches[7]
        .candidates
        .iter()
        .find(|candidate| {
            candidate.relation == CandidateRelation::Calls
                && candidate.target_spelling == "callable"
                && candidate
                    .constraints
                    .qualified_name
                    .as_deref()
                    .is_some_and(|qualified| qualified.contains("function-spread.js::default"))
        })
        .ok_or("missing non-object spread call")?;
    let override_member = batches[3]
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name == "override.default.isNumber")
        .ok_or("missing direct override member")?;
    let cross_call = batches[7]
        .candidates
        .iter()
        .find(|candidate| {
            candidate.relation == CandidateRelation::Calls
                && candidate.target_spelling == "isNumber"
                && candidate
                    .constraints
                    .qualified_name
                    .as_deref()
                    .is_some_and(|qualified| qualified.contains("cross.js::default"))
        })
        .ok_or("missing cross-language spread call")?;
    let left_member = batches[0]
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name == "left.default.isNumber")
        .ok_or("missing cross-language source member")?;
    let index = UniversalResolutionIndex::new_with_inventory(
        &batches,
        &[],
        root,
        UniversalResolutionLimits::default(),
    )?;
    assert!(!matches!(
        index.resolve(&ambiguous_call.id),
        compass_resolve::evidence::ResolutionDecision::Resolved { .. }
    ));
    assert!(matches!(
        index.resolve(&override_call.id),
        compass_resolve::evidence::ResolutionDecision::Resolved { ref declaration_id, .. }
            if declaration_id == &override_member.id
    ));
    assert!(matches!(
        index.resolve(&cross_call.id),
        compass_resolve::evidence::ResolutionDecision::Resolved { ref declaration_id, .. }
            if declaration_id == &left_member.id
    ));
    assert!(!matches!(
        index.resolve(&function_call.id),
        compass_resolve::evidence::ResolutionDecision::Resolved { .. }
    ));
    Ok(())
}

#[test]
fn javascript_commonjs_object_exports_resolve_named_require_bindings()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/cjs.js",
            br#"function run() {}
module.exports = {
    run,
    method() { return run(); },
};
"#
            .as_slice(),
        ),
        (
            "app/consumer.js",
            br#"const { run: execute, method: invoke } = require("../lib/cjs");
execute();
invoke();
const api = require("../lib/cjs");
api.run();
"#
            .as_slice(),
        ),
    ];
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(path, source)?;
    }
    let batches = files
        .iter()
        .map(|(relative, source)| {
            let path = root.join(relative);
            Engine::default()
                .extract_source_universal_candidate_evidence(&path, relative, source)
                .map_err(|error| format!("candidate extraction failed for {relative}: {error}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let run = batches[0]
        .declarations
        .iter()
        .find(|declaration| declaration.kind == "function" && declaration.name == "run")
        .ok_or("missing CommonJS run declaration")?;
    let method = batches[0]
        .declarations
        .iter()
        .find(|declaration| declaration.kind == "method" && declaration.name == "method")
        .ok_or("missing CommonJS method declaration")?;
    let calls = batches[1]
        .candidates
        .iter()
        .filter(|candidate| candidate.relation == CandidateRelation::Calls)
        .collect::<Vec<_>>();
    let run_call = calls
        .iter()
        .find(|candidate| candidate.target_spelling == "execute")
        .ok_or("missing aliased required run call")?;
    let method_call = calls
        .iter()
        .find(|candidate| candidate.target_spelling == "invoke")
        .ok_or("missing aliased required method call")?;
    let api_run_call = calls
        .iter()
        .find(|candidate| {
            candidate.target_spelling == "run"
                && candidate
                    .constraints
                    .qualified_name
                    .as_deref()
                    .is_some_and(|qualified| qualified.ends_with("::run"))
        })
        .ok_or("missing namespace require member call")?;
    let index = UniversalResolutionIndex::new_with_inventory(
        &batches,
        &[],
        root,
        UniversalResolutionLimits::default(),
    )?;
    assert!(matches!(
        index.resolve(&run_call.id),
        compass_resolve::evidence::ResolutionDecision::Resolved { ref declaration_id, .. }
            if declaration_id == &run.id
    ));
    assert!(matches!(
        index.resolve(&method_call.id),
        compass_resolve::evidence::ResolutionDecision::Resolved { ref declaration_id, .. }
            if declaration_id == &method.id
    ));
    assert!(matches!(
        index.resolve(&api_run_call.id),
        compass_resolve::evidence::ResolutionDecision::Resolved { ref declaration_id, .. }
            if declaration_id == &run.id
    ));
    Ok(())
}

#[test]
fn javascript_commonjs_object_spreads_resolve_proven_members_and_direct_overrides()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/base.ts",
            br#"export default {
    inherited() { return true; },
    conflict() { return true; },
};
"#
            .as_slice(),
        ),
        (
            "lib/derived.js",
            br#"import base from "./base";
module.exports = {
    ...base,
    direct() { return base.inherited(); },
    conflict() { return false; },
};
"#
            .as_slice(),
        ),
        (
            "app/consumer.js",
            br#"const api = require("../lib/derived");
api.inherited();
api.direct();
api.conflict();
"#
            .as_slice(),
        ),
    ];
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(path, source)?;
    }
    let batches = files
        .iter()
        .map(|(relative, source)| {
            let path = root.join(relative);
            Engine::default()
                .extract_source_universal_candidate_evidence(&path, relative, source)
                .map_err(|error| format!("candidate extraction failed for {relative}: {error}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let base = batches[0]
        .declarations
        .iter()
        .find(|declaration| declaration.kind == "method" && declaration.name == "inherited")
        .ok_or("missing base inherited method")?;
    let derived_direct = batches[1]
        .declarations
        .iter()
        .find(|declaration| declaration.kind == "method" && declaration.name == "direct")
        .ok_or("missing derived direct method")?;
    let derived_conflict = batches[1]
        .declarations
        .iter()
        .find(|declaration| declaration.kind == "method" && declaration.name == "conflict")
        .ok_or("missing derived conflict method")?;
    let calls = batches[2]
        .candidates
        .iter()
        .filter(|candidate| candidate.relation == CandidateRelation::Calls)
        .collect::<Vec<_>>();
    let inherited_call = calls
        .iter()
        .find(|candidate| candidate.target_spelling == "inherited")
        .ok_or("missing inherited call")?;
    let direct_call = calls
        .iter()
        .find(|candidate| candidate.target_spelling == "direct")
        .ok_or("missing direct call")?;
    let conflict_call = calls
        .iter()
        .find(|candidate| candidate.target_spelling == "conflict")
        .ok_or("missing conflict call")?;
    let index = UniversalResolutionIndex::new_with_inventory(
        &batches,
        &[],
        root,
        UniversalResolutionLimits::default(),
    )?;
    assert!(matches!(
        index.resolve(&inherited_call.id),
        compass_resolve::evidence::ResolutionDecision::Resolved { ref declaration_id, .. }
            if declaration_id == &base.id
    ));
    assert!(matches!(
        index.resolve(&direct_call.id),
        compass_resolve::evidence::ResolutionDecision::Resolved { ref declaration_id, .. }
            if declaration_id == &derived_direct.id
    ));
    assert!(matches!(
        index.resolve(&conflict_call.id),
        compass_resolve::evidence::ResolutionDecision::Resolved { ref declaration_id, .. }
            if declaration_id == &derived_conflict.id
    ));
    Ok(())
}

#[test]
fn javascript_commonjs_object_spreads_fail_closed_for_ambiguity_and_non_objects()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/left.ts",
            br#"export default { shared() {} };
"#
            .as_slice(),
        ),
        (
            "lib/right.ts",
            br#"export default { shared() {} };
"#
            .as_slice(),
        ),
        (
            "lib/callable.ts",
            br#"export default function callable() {}
"#
            .as_slice(),
        ),
        (
            "lib/ambiguous.js",
            br#"import left from "./left";
import right from "./right";
module.exports = { ...left, ...right };
"#
            .as_slice(),
        ),
        (
            "lib/non-object.js",
            br#"import callable from "./callable";
module.exports = { ...callable };
"#
            .as_slice(),
        ),
        (
            "app/consumer.js",
            br#"const ambiguous = require("../lib/ambiguous");
ambiguous.shared();
const nonObject = require("../lib/non-object");
nonObject.missing();
"#
            .as_slice(),
        ),
    ];
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(path, source)?;
    }
    let batches = files
        .iter()
        .map(|(relative, source)| {
            let path = root.join(relative);
            Engine::default()
                .extract_source_universal_candidate_evidence(&path, relative, source)
                .map_err(|error| format!("candidate extraction failed for {relative}: {error}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let calls = batches[5]
        .candidates
        .iter()
        .filter(|candidate| candidate.relation == CandidateRelation::Calls)
        .collect::<Vec<_>>();
    let ambiguous_call = calls
        .iter()
        .find(|candidate| candidate.target_spelling == "shared")
        .ok_or("missing ambiguous member call")?;
    let non_object_call = calls
        .iter()
        .find(|candidate| candidate.target_spelling == "missing")
        .ok_or("missing non-object member call")?;
    let index = UniversalResolutionIndex::new_with_inventory(
        &batches,
        &[],
        root,
        UniversalResolutionLimits::default(),
    )?;
    assert!(matches!(
        index.resolve(&ambiguous_call.id),
        compass_resolve::evidence::ResolutionDecision::Ambiguous { .. }
    ));
    assert!(matches!(
        index.resolve(&non_object_call.id),
        compass_resolve::evidence::ResolutionDecision::Unresolved
    ));
    Ok(())
}

#[test]
fn javascript_namespace_and_require_spreads_follow_published_exports_only()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/esm.ts",
            br#"export function esmPublished() {}
function esmPrivate() {}
"#
            .as_slice(),
        ),
        (
            "lib/cjs.cjs",
            br#"function cjsPublished() {}
function cjsPrivate() {}
module.exports = { cjsPublished };
"#
            .as_slice(),
        ),
        (
            "lib/derived.js",
            br#"import * as esm from "./esm";
const cjs = require("./cjs");
module.exports = { ...esm, ...cjs };
"#
            .as_slice(),
        ),
        (
            "app/consumer.js",
            br#"const api = require("../lib/derived");
api.esmPublished();
api.cjsPublished();
api.esmPrivate();
api.cjsPrivate();
"#
            .as_slice(),
        ),
    ];
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(path, source)?;
    }
    let batches = files
        .iter()
        .map(|(relative, source)| {
            let path = root.join(relative);
            Engine::default()
                .extract_source_universal_candidate_evidence(&path, relative, source)
                .map_err(|error| format!("candidate extraction failed for {relative}: {error}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let esm_published = batches[0]
        .declarations
        .iter()
        .find(|declaration| declaration.kind == "function" && declaration.name == "esmPublished")
        .ok_or("missing published ESM function")?;
    let cjs_published = batches[1]
        .declarations
        .iter()
        .find(|declaration| declaration.kind == "function" && declaration.name == "cjsPublished")
        .ok_or("missing published CommonJS function")?;
    let calls = batches[3]
        .candidates
        .iter()
        .filter(|candidate| candidate.relation == CandidateRelation::Calls)
        .collect::<Vec<_>>();
    let find_call = |name: &str| {
        calls
            .iter()
            .find(|candidate| candidate.target_spelling == name)
            .ok_or_else(|| format!("missing call {name}"))
    };
    let esm_call = find_call("esmPublished")?;
    let cjs_call = find_call("cjsPublished")?;
    let esm_private_call = find_call("esmPrivate")?;
    let cjs_private_call = find_call("cjsPrivate")?;
    let index = UniversalResolutionIndex::new_with_inventory(
        &batches,
        &[],
        root,
        UniversalResolutionLimits::default(),
    )?;
    assert!(matches!(
        index.resolve(&esm_call.id),
        compass_resolve::evidence::ResolutionDecision::Resolved { ref declaration_id, .. }
            if declaration_id == &esm_published.id
    ));
    assert!(matches!(
        index.resolve(&cjs_call.id),
        compass_resolve::evidence::ResolutionDecision::Resolved { ref declaration_id, .. }
            if declaration_id == &cjs_published.id
    ));
    assert!(matches!(
        index.resolve(&esm_private_call.id),
        compass_resolve::evidence::ResolutionDecision::Unresolved
    ));
    assert!(matches!(
        index.resolve(&cjs_private_call.id),
        compass_resolve::evidence::ResolutionDecision::Unresolved
    ));
    Ok(())
}

#[test]
fn javascript_commonjs_object_assign_resolves_proven_members_and_mutations()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/base.ts",
            br#"export default {
    inherited() { return true; },
    conflict() { return true; },
};
"#
            .as_slice(),
        ),
        (
            "lib/derived.js",
            br#"import base from "./base";
module.exports = Object.assign({}, base, {
    direct() { return base.inherited(); },
    conflict() { return false; },
});
"#
            .as_slice(),
        ),
        (
            "lib/mutated.js",
            br#"import base from "./base";
Object.assign(exports, base);
"#
            .as_slice(),
        ),
        (
            "lib/unknown.js",
            br#"const source = getSource();
module.exports = Object.assign({}, source, { direct() {} });
"#
            .as_slice(),
        ),
        (
            "app/consumer.js",
            br#"const derived = require("../lib/derived");
derived.inherited();
derived.direct();
derived.conflict();
const mutated = require("../lib/mutated");
mutated.inherited();
const unknown = require("../lib/unknown");
unknown.direct();
"#
            .as_slice(),
        ),
    ];
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(path, source)?;
    }
    let batches = files
        .iter()
        .map(|(relative, source)| {
            let path = root.join(relative);
            Engine::default()
                .extract_source_universal_candidate_evidence(&path, relative, source)
                .map_err(|error| format!("candidate extraction failed for {relative}: {error}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let base_inherited = batches[0]
        .declarations
        .iter()
        .find(|declaration| declaration.kind == "method" && declaration.name == "inherited")
        .ok_or("missing base inherited method")?;
    let derived_direct = batches[1]
        .declarations
        .iter()
        .find(|declaration| declaration.kind == "method" && declaration.name == "direct")
        .ok_or("missing derived direct method")?;
    let derived_conflict = batches[1]
        .declarations
        .iter()
        .find(|declaration| declaration.kind == "method" && declaration.name == "conflict")
        .ok_or("missing derived conflict method")?;
    let calls = batches[4]
        .candidates
        .iter()
        .filter(|candidate| candidate.relation == CandidateRelation::Calls)
        .collect::<Vec<_>>();
    let call = |name: &str| {
        calls
            .iter()
            .find(|candidate| candidate.target_spelling == name)
            .ok_or_else(|| format!("missing call {name}"))
    };
    let inherited = call("inherited")?;
    let direct = call("direct")?;
    let conflict = call("conflict")?;
    let mutated_inherited = calls
        .iter()
        .filter(|candidate| candidate.target_spelling == "inherited")
        .nth(1)
        .ok_or("missing mutated inherited call")?;
    let unknown_direct = calls
        .iter()
        .filter(|candidate| candidate.target_spelling == "direct")
        .nth(1)
        .ok_or("missing unknown direct call")?;
    let index = UniversalResolutionIndex::new_with_inventory(
        &batches,
        &[],
        root,
        UniversalResolutionLimits::default(),
    )?;
    assert!(matches!(
        index.resolve(&inherited.id),
        compass_resolve::evidence::ResolutionDecision::Resolved { ref declaration_id, .. }
            if declaration_id == &base_inherited.id
    ));
    assert!(matches!(
        index.resolve(&direct.id),
        compass_resolve::evidence::ResolutionDecision::Resolved { ref declaration_id, .. }
            if declaration_id == &derived_direct.id
    ));
    assert!(matches!(
        index.resolve(&conflict.id),
        compass_resolve::evidence::ResolutionDecision::Resolved { ref declaration_id, .. }
            if declaration_id == &derived_conflict.id
    ));
    assert!(matches!(
        index.resolve(&mutated_inherited.id),
        compass_resolve::evidence::ResolutionDecision::Resolved { ref declaration_id, .. }
            if declaration_id == &base_inherited.id
    ));
    assert!(matches!(
        index.resolve(&unknown_direct.id),
        compass_resolve::evidence::ResolutionDecision::Unresolved
    ));
    Ok(())
}

#[test]
fn javascript_commonjs_define_property_resolves_value_and_getter_exports()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/values.ts",
            br#"export function imported() {}
"#
            .as_slice(),
        ),
        (
            "lib/define.cjs",
            br#"import { imported } from "./values";
function run() {}
Object.defineProperty(exports, "run", { enumerable: true, value: run });
Object.defineProperty(exports, "imported", {
    enumerable: true,
    get: function () { return imported; },
});
Object.defineProperty(exports, "__esModule", { value: true });
Object.defineProperty(exports, "unknown", { value: getUnknown() });
"#
            .as_slice(),
        ),
        (
            "app/consumer.js",
            br#"const api = require("../lib/define");
api.run();
api.imported();
api.unknown();
"#
            .as_slice(),
        ),
    ];
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(path, source)?;
    }
    let batches = files
        .iter()
        .map(|(relative, source)| {
            let path = root.join(relative);
            Engine::default()
                .extract_source_universal_candidate_evidence(&path, relative, source)
                .map_err(|error| format!("candidate extraction failed for {relative}: {error}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let run = batches[1]
        .declarations
        .iter()
        .find(|declaration| declaration.kind == "function" && declaration.name == "run")
        .ok_or("missing defineProperty run declaration")?;
    let imported = batches[0]
        .declarations
        .iter()
        .find(|declaration| declaration.kind == "function" && declaration.name == "imported")
        .ok_or("missing imported declaration")?;
    let calls = batches[2]
        .candidates
        .iter()
        .filter(|candidate| candidate.relation == CandidateRelation::Calls)
        .collect::<Vec<_>>();
    let run_call = calls
        .iter()
        .find(|candidate| candidate.target_spelling == "run")
        .ok_or("missing run call")?;
    let imported_call = calls
        .iter()
        .find(|candidate| candidate.target_spelling == "imported")
        .ok_or("missing imported call")?;
    let unknown_call = calls
        .iter()
        .find(|candidate| candidate.target_spelling == "unknown")
        .ok_or("missing unknown call")?;
    let index = UniversalResolutionIndex::new_with_inventory(
        &batches,
        &[],
        root,
        UniversalResolutionLimits::default(),
    )?;
    assert!(matches!(
        index.resolve(&run_call.id),
        compass_resolve::evidence::ResolutionDecision::Resolved { ref declaration_id, .. }
            if declaration_id == &run.id
    ));
    assert!(matches!(
        index.resolve(&imported_call.id),
        compass_resolve::evidence::ResolutionDecision::Resolved { ref declaration_id, .. }
            if declaration_id == &imported.id
    ));
    assert!(matches!(
        index.resolve(&unknown_call.id),
        compass_resolve::evidence::ResolutionDecision::Unresolved
    ));
    Ok(())
}

#[test]
fn javascript_commonjs_export_star_resolves_named_and_namespace_members()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/source.js",
            br#"function inherited() {}
exports.inherited = inherited;
function privateThing() {}
"#
            .as_slice(),
        ),
        (
            "lib/barrel.cjs",
            br#"const tslib = require("tslib");
tslib.__exportStar(require("./source"), exports);
"#
            .as_slice(),
        ),
        (
            "app/consumer.js",
            br#"const { inherited, privateThing } = require("../lib/barrel");
const api = require("../lib/barrel");
inherited();
privateThing();
api.inherited();
"#
            .as_slice(),
        ),
    ];
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(path, source)?;
    }
    let batches = files
        .iter()
        .map(|(relative, source)| {
            let path = root.join(relative);
            Engine::default()
                .extract_source_universal_candidate_evidence(&path, relative, source)
                .map_err(|error| format!("candidate extraction failed for {relative}: {error}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let inherited = batches[0]
        .declarations
        .iter()
        .find(|declaration| declaration.kind == "function" && declaration.name == "inherited")
        .ok_or("missing source inherited declaration")?;
    let calls = batches[2]
        .candidates
        .iter()
        .filter(|candidate| candidate.relation == CandidateRelation::Calls)
        .collect::<Vec<_>>();
    let inherited_calls = calls
        .iter()
        .filter(|candidate| candidate.target_spelling == "inherited")
        .collect::<Vec<_>>();
    assert_eq!(inherited_calls.len(), 2);
    let private_call = calls
        .iter()
        .find(|candidate| candidate.target_spelling == "privateThing")
        .ok_or("missing privateThing call")?;
    let index = UniversalResolutionIndex::new_with_inventory(
        &batches,
        &[],
        root,
        UniversalResolutionLimits::default(),
    )?;
    for call in inherited_calls {
        assert!(matches!(
            index.resolve(&call.id),
            compass_resolve::evidence::ResolutionDecision::Resolved { ref declaration_id, .. }
                if declaration_id == &inherited.id
        ));
    }
    assert!(matches!(
        index.resolve(&private_call.id),
        compass_resolve::evidence::ResolutionDecision::Unresolved
    ));
    Ok(())
}

#[test]
fn javascript_commonjs_require_callable_namespace_resolves_default_export()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let provider = root.join("lib/callable.cjs");
    let consumer = root.join("app/consumer.js");
    let provider_source = br#"function run() {}
module.exports = run;
"#;
    let consumer_source = br#"const fn = require("../lib/callable");
fn();
"#;
    for (path, source) in [
        (&provider, provider_source.as_slice()),
        (&consumer, consumer_source.as_slice()),
    ] {
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(path, source)?;
    }
    let batches = [
        Engine::default().extract_source_universal_candidate_evidence(
            &provider,
            "lib/callable.cjs",
            provider_source,
        )?,
        Engine::default().extract_source_universal_candidate_evidence(
            &consumer,
            "app/consumer.js",
            consumer_source,
        )?,
    ];
    let run = batches[0]
        .declarations
        .iter()
        .find(|declaration| declaration.kind == "function" && declaration.name == "run")
        .ok_or("missing CommonJS callable declaration")?;
    let call = batches[1]
        .candidates
        .iter()
        .find(|candidate| {
            candidate.relation == CandidateRelation::Calls && candidate.target_spelling == "fn"
        })
        .ok_or("missing direct CommonJS callable call")?;
    let index = UniversalResolutionIndex::new_with_inventory(
        &batches,
        &[],
        root,
        UniversalResolutionLimits::default(),
    )?;
    assert!(matches!(
        index.resolve(&call.id),
        compass_resolve::evidence::ResolutionDecision::Resolved { ref declaration_id, .. }
            if declaration_id == &run.id
    ));
    Ok(())
}
