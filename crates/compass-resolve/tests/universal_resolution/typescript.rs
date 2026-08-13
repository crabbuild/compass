#[test]
fn typescript_workspace_package_exports_follow_nodenext_reexports()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let package = directory.path().join("packages/timezone/package.json");
    let barrel = directory.path().join("packages/timezone/src/index.ts");
    let implementation = directory.path().join("packages/timezone/src/date/index.ts");
    let consumer = directory.path().join("packages/app/src/consumer.ts");
    for path in [&package, &barrel, &implementation, &consumer] {
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
    }
    let package_source = br#"{"name":"@example/timezone","exports":{".":"./src/index.ts"}}"#;
    let barrel_source = br#"export * from "./date/index.js";"#;
    let implementation_source = br#"export class ZonedDate {}"#;
    let consumer_source = br#"import { ZonedDate } from "@example/timezone";
export function makeDate() { return new ZonedDate(); }
function consume(value: unknown) { return value; }
export const wrappedDate = consume(ZonedDate);
"#;
    for (path, source) in [
        (&package, package_source.as_slice()),
        (&barrel, barrel_source.as_slice()),
        (&implementation, implementation_source.as_slice()),
        (&consumer, consumer_source.as_slice()),
    ] {
        fs::write(path, source)?;
    }
    let extractions = [
        extract(
            package.to_str().ok_or("non-UTF-8 fixture path")?,
            package_source,
        ),
        extract(
            barrel.to_str().ok_or("non-UTF-8 fixture path")?,
            barrel_source,
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
    assert!(extractions[0].nodes.iter().any(|node| {
        node.string("symbol_kind") == "file"
            && node.string("source_file") == package.to_string_lossy()
    }));
    assert!(
        extractions[3]
            .semantic_evidence
            .as_ref()
            .is_some_and(|batch| {
                batch.candidates.iter().any(|candidate| {
                    matches!(
                        candidate.relation,
                        CandidateRelation::Imports | CandidateRelation::Reexports
                    ) && candidate.constraints.module_or_package.as_deref()
                        == Some("@example/timezone")
                })
            })
    );
    let sources = [
        (&package, package_source.as_slice()),
        (&barrel, barrel_source.as_slice()),
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

    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, directory.path());
    assert_eq!(resolved.error, None);
    let declaration = resolved
        .nodes
        .iter()
        .find(|node| {
            node.label() == "ZonedDate"
                && node.string("symbol_kind") == "class"
                && source_matches(&node.string("source_file"), &implementation)
        })
        .ok_or("missing ZonedDate declaration")?;
    for target in [&barrel, &implementation] {
        assert!(
            resolved
                .nodes
                .iter()
                .any(|node| source_matches(&node.string("source_file"), target))
        );
    }
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "imports_from"
            && source_matches(&edge.string("source_file"), &consumer)
            && target_source_matches(&resolved, &edge.target, &implementation)
    }));
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "re_exports"
            && target_source_matches(&resolved, &edge.source, &barrel)
            && target_source_matches(&resolved, &edge.target, &implementation)
    }));
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "imports_from"
            && edge.target == declaration.id
            && source_matches(&edge.string("source_file"), &consumer)
    }));
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.target == declaration.id
            && source_matches(&edge.string("source_file"), &consumer)
    }));
    Ok(())
}

#[test]
fn typescript_paths_aliases_resolve_extension_substitution_and_named_symbols()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let config = root.join("tsconfig.json");
    let implementation = root.join("src/api.ts");
    let consumer = root.join("app/consumer.ts");
    for path in [&config, &implementation, &consumer] {
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
    }
    let config_source = br#"{
        // JSONC is accepted by TypeScript project configuration.
        "compilerOptions": {
            "baseUrl": ".",
            "paths": { "@/*": ["./src/*",], },
        },
    }"#;
    let implementation_source = br#"export class Widget { run() {} }"#;
    let consumer_source = br#"import { Widget } from "@/api.js";
export function make() { return new Widget(); }
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
    let declaration_id = resolved
        .nodes
        .iter()
        .find(|node| {
            node.label() == "Widget"
                && node.string("symbol_kind") == "class"
                && source_matches(&node.string("source_file"), &implementation)
        })
        .map(|node| node.id.clone())
        .ok_or("missing Widget declaration")?;
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "imports_from"
            && edge.string("module") == "@/api.js"
            && target_source_matches(&resolved, &edge.target, &implementation)
            && edge.string("resolution_rule") == "project-module-binding"
            && edge.string("project_resolution_rule") == "typescript-paths"
    }));
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "imports_from"
            && edge.target == declaration_id
            && edge.string("local_name") == "Widget"
    }));
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.target == declaration_id
            && source_matches(&edge.string("source_file"), &consumer)
    }));
    Ok(())
}

#[test]
fn typescript_paths_choose_nearest_config_and_ordered_fallbacks()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let root_config = root.join("tsconfig.json");
    let nested_config = root.join("app/tsconfig.json");
    let first = root.join("app/src/first.ts");
    let second = root.join("app/src/second.ts");
    let consumer = root.join("app/consumer.ts");
    for path in [&root_config, &nested_config, &first, &second, &consumer] {
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
    }
    let root_config_source = br#"{
        "compilerOptions": { "paths": { "@/*": ["./wrong/*"] } }
    }"#;
    let nested_config_source = br#"{
        "compilerOptions": {
            "baseUrl": ".",
            "paths": { "@/*": ["./missing/*", "./src/*"] }
        }
    }"#;
    let first_source = br#"export const first = true;"#;
    let second_source = br#"export const second = true;"#;
    let consumer_source = br#"import { second } from "@/second.js";
export const value = second;
"#;
    for (path, source) in [
        (&root_config, root_config_source.as_slice()),
        (&nested_config, nested_config_source.as_slice()),
        (&first, first_source.as_slice()),
        (&second, second_source.as_slice()),
        (&consumer, consumer_source.as_slice()),
    ] {
        fs::write(path, source)?;
    }
    let extractions = [
        extract(
            first.to_str().ok_or("non-UTF-8 fixture path")?,
            first_source,
        ),
        extract(
            second.to_str().ok_or("non-UTF-8 fixture path")?,
            second_source,
        ),
        extract(
            consumer.to_str().ok_or("non-UTF-8 fixture path")?,
            consumer_source,
        ),
    ];
    let sources = [
        (&root_config, root_config_source.as_slice()),
        (&nested_config, nested_config_source.as_slice()),
        (&first, first_source.as_slice()),
        (&second, second_source.as_slice()),
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
            .any(|node| source_matches(&node.string("source_file"), &second))
    );
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "imports_from"
            && edge.string("module") == "@/second.js"
            && target_source_matches(&resolved, &edge.target, &second)
            && edge.string("resolution_config") == "app/tsconfig.json"
    }));
    Ok(())
}

#[test]
fn typescript_paths_leave_same_depth_config_ambiguity_unresolved()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let first_config = root.join("app/tsconfig.json");
    let second_config = root.join("app/tsconfig.alt.json");
    let first = root.join("app/one.ts");
    let second = root.join("app/two.ts");
    let consumer = root.join("app/consumer.ts");
    for path in [&first_config, &second_config, &first, &second, &consumer] {
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
    }
    let first_config_source = br#"{"compilerOptions":{"paths":{"@/shared":["./one.ts"]}}}"#;
    let second_config_source = br#"{"compilerOptions":{"paths":{"@/shared":["./two.ts"]}}}"#;
    let first_source = br#"export const one = true;"#;
    let second_source = br#"export const two = true;"#;
    let consumer_source = br#"import { one } from "@/shared";
export const value = one;
"#;
    for (path, source) in [
        (&first_config, first_config_source.as_slice()),
        (&second_config, second_config_source.as_slice()),
        (&first, first_source.as_slice()),
        (&second, second_source.as_slice()),
        (&consumer, consumer_source.as_slice()),
    ] {
        fs::write(path, source)?;
    }
    let extractions = [
        extract(
            first.to_str().ok_or("non-UTF-8 fixture path")?,
            first_source,
        ),
        extract(
            second.to_str().ok_or("non-UTF-8 fixture path")?,
            second_source,
        ),
        extract(
            consumer.to_str().ok_or("non-UTF-8 fixture path")?,
            consumer_source,
        ),
    ];
    let sources = [
        (&first_config, first_config_source.as_slice()),
        (&second_config, second_config_source.as_slice()),
        (&first, first_source.as_slice()),
        (&second, second_source.as_slice()),
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
    let import = resolved
        .edges
        .iter()
        .find(|edge| {
            edge.string("relation") == "imports_from" && edge.string("module") == "@/shared"
        })
        .ok_or("missing ambiguous import")?;
    assert!(!target_source_matches(&resolved, &import.target, &first));
    assert!(!target_source_matches(&resolved, &import.target, &second));
    assert_eq!(import.string("resolution_rule"), "qualified-external");
    Ok(())
}

#[test]
fn typescript_config_extends_inherits_paths_and_project_metadata()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let base = root.join("tsconfig.base.json");
    let config = root.join("tsconfig.json");
    let implementation = root.join("src/api.ts");
    let consumer = root.join("app/consumer.ts");
    for path in [&base, &config, &implementation, &consumer] {
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
    }
    let base_source = br#"{
        "compilerOptions": {
            "baseUrl": "..",
            "paths": { "@/*": ["src/*"] },
            "module": "NodeNext",
            "moduleResolution": "NodeNext"
        }
    }"#;
    let config_source = br#"{
        "extends": "./tsconfig.base.json",
        "compilerOptions": { "allowJs": true },
        "references": [{ "path": "./src" }]
    }"#;
    let implementation_source = br#"export class Widget {}"#;
    let consumer_source = br#"import { Widget } from "@/api.js";
export const value = new Widget();
"#;
    for (path, source) in [
        (&base, base_source.as_slice()),
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
        (&base, base_source.as_slice()),
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
    let import = resolved
        .edges
        .iter()
        .find(|edge| {
            edge.string("relation") == "imports_from"
                && source_matches(&edge.string("source_file"), &consumer)
        })
        .ok_or("missing inherited alias import")?;
    assert!(target_source_matches(
        &resolved,
        &import.target,
        &implementation
    ));
    assert_eq!(import.string("resolution_rule"), "project-module-binding");
    assert_eq!(import.string("project_resolution_rule"), "typescript-paths");
    assert_eq!(import.string("resolution_config"), "tsconfig.json");
    assert_eq!(import.string("module_resolution"), "nodenext");
    assert_eq!(import.string("module_kind"), "nodenext");
    assert_eq!(
        import.attributes.get("resolution_project_references"),
        Some(&serde_json::json!(["src"]))
    );
    Ok(())
}

#[test]
fn typescript_relative_imports_use_module_suffixes_and_root_dirs()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let config = root.join("tsconfig.json");
    let consumer = root.join("src/app/consumer.ts");
    let suffixed = root.join("src/app/feature.ios.ts");
    let generated = root.join("generated/app/runtime.ts");
    let config_source = br#"{
        "compilerOptions": {
            "module": "ESNext",
            "moduleResolution": "Bundler",
            "moduleSuffixes": [".ios", ""],
            "rootDirs": ["src", "generated"]
        }
    }"#;
    let consumer_source = br#"import { feature } from "./feature.js";
import { runtime } from "./runtime.js";
export const value = feature + runtime;
"#;
    let suffixed_source = br#"export const feature = 1;"#;
    let generated_source = br#"export const runtime = 2;"#;
    for (path, source) in [
        (&config, config_source.as_slice()),
        (&consumer, consumer_source.as_slice()),
        (&suffixed, suffixed_source.as_slice()),
        (&generated, generated_source.as_slice()),
    ] {
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(path, source)?;
    }
    let extractions = [
        extract(
            suffixed.to_str().ok_or("non-UTF-8 fixture path")?,
            suffixed_source,
        ),
        extract(
            generated.to_str().ok_or("non-UTF-8 fixture path")?,
            generated_source,
        ),
        extract(
            consumer.to_str().ok_or("non-UTF-8 fixture path")?,
            consumer_source,
        ),
    ];
    let sources = [
        (&config, config_source.as_slice()),
        (&consumer, consumer_source.as_slice()),
        (&suffixed, suffixed_source.as_slice()),
        (&generated, generated_source.as_slice()),
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
    for target in [&suffixed, &generated] {
        assert!(
            resolved
                .nodes
                .iter()
                .any(|node| source_matches(&node.string("source_file"), target))
        );
    }
    let imports = resolved
        .edges
        .iter()
        .filter(|edge| {
            edge.string("relation") == "imports_from"
                && source_matches(&edge.string("source_file"), &consumer)
        })
        .collect::<Vec<_>>();
    assert!(imports.iter().any(|edge| {
        target_source_matches(&resolved, &edge.target, &suffixed)
            && edge.string("resolution_rule") == "project-module-binding"
            && edge.string("project_resolution_rule") == "typescript-relative"
    }));
    assert!(imports.iter().any(|edge| {
        target_source_matches(&resolved, &edge.target, &generated)
            && edge.string("resolution_rule") == "project-module-binding"
            && edge.string("project_resolution_rule") == "typescript-root-dirs"
    }));
    assert!(imports.iter().all(|edge| {
        edge.string("module_resolution") == "bundler"
            && edge.string("module_kind") == "esnext"
            && edge.string("resolution_config") == "tsconfig.json"
    }));
    Ok(())
}

#[test]
fn typescript_package_types_versions_selects_the_admitted_declaration_target()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let package = root.join("packages/typed/package.json");
    let declaration = root.join("packages/typed/types/index.ts");
    let consumer = root.join("app/consumer.ts");
    let package_source = br#"{
  "name": "@example/typed",
  "types": "./src/index.d.ts",
  "typesVersions": { "*": { "*": ["types/*"] } }
}"#;
    let declaration_source = br#"export function helper(): string { return "ok"; }"#;
    let consumer_source = br#"import { helper } from "@example/typed";
export const value = helper();
"#;
    for (path, source) in [
        (&package, package_source.as_slice()),
        (&declaration, declaration_source.as_slice()),
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
            declaration.to_str().ok_or("non-UTF-8 fixture path")?,
            declaration_source,
        ),
        extract(
            consumer.to_str().ok_or("non-UTF-8 fixture path")?,
            consumer_source,
        ),
    ];
    let sources = [
        (&package, package_source.as_slice()),
        (&declaration, declaration_source.as_slice()),
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
    let module_import = resolved
        .edges
        .iter()
        .find(|edge| {
            edge.string("relation") == "imports_from" && edge.string("module") == "@example/typed"
        })
        .ok_or("missing package import")?;
    assert_eq!(
        module_import.string("resolution_rule"),
        "project-module-binding"
    );
    assert_eq!(
        module_import.string("project_resolution_rule"),
        "typesVersions"
    );
    let helper = resolved
        .nodes
        .iter()
        .find(|node| {
            matches!(node.label(), "helper()" | "helper")
                && node.string("symbol_kind") == "function"
                && source_matches(&node.string("source_file"), &declaration)
        })
        .ok_or("missing typesVersions declaration")?;
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "imports_from"
            && edge.target == helper.id
            && edge.string("local_name") == "helper"
    }));
    Ok(())
}

#[test]
fn typescript_include_exclude_ownership_blocks_out_of_project_alias_targets()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let config = root.join("tsconfig.json");
    let allowed = root.join("src/allowed.ts");
    let excluded = root.join("src/excluded.ts");
    let consumer = root.join("src/consumer.ts");
    let config_source = br#"{
  "compilerOptions": { "baseUrl": ".", "paths": { "@/*": ["src/*"] } },
  "include": ["src/**/*.ts"],
  "exclude": ["src/excluded.ts"]
}"#;
    let allowed_source = br#"export const allowed = true;"#;
    let excluded_source = br#"export const excluded = true;"#;
    let consumer_source = br#"import { allowed } from "@/allowed";
import { excluded } from "@/excluded";
export const value = allowed && excluded;
"#;
    for (path, source) in [
        (&config, config_source.as_slice()),
        (&allowed, allowed_source.as_slice()),
        (&excluded, excluded_source.as_slice()),
        (&consumer, consumer_source.as_slice()),
    ] {
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(path, source)?;
    }
    let extractions = [
        extract(
            allowed.to_str().ok_or("non-UTF-8 fixture path")?,
            allowed_source,
        ),
        extract(
            excluded.to_str().ok_or("non-UTF-8 fixture path")?,
            excluded_source,
        ),
        extract(
            consumer.to_str().ok_or("non-UTF-8 fixture path")?,
            consumer_source,
        ),
    ];
    let sources = [
        (&config, config_source.as_slice()),
        (&allowed, allowed_source.as_slice()),
        (&excluded, excluded_source.as_slice()),
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
    let excluded_id = resolved
        .nodes
        .iter()
        .find(|node| source_matches(&node.string("source_file"), &excluded))
        .map(|node| node.id.clone())
        .ok_or("missing excluded target")?;
    let imports = resolved
        .edges
        .iter()
        .filter(|edge| {
            edge.string("relation") == "imports_from"
                && source_matches(&edge.string("source_file"), &consumer)
        })
        .collect::<Vec<_>>();
    assert!(imports.iter().any(|edge| {
        edge.string("module") == "@/allowed"
            && target_source_matches(&resolved, &edge.target, &allowed)
            && edge.string("resolution_rule") == "project-module-binding"
            && edge.string("project_resolution_rule") == "typescript-paths"
    }));
    let excluded_import = imports
        .iter()
        .find(|edge| edge.string("module") == "@/excluded")
        .ok_or("missing excluded import")?;
    assert_ne!(excluded_import.target, excluded_id);
    assert_eq!(
        excluded_import.string("resolution_rule"),
        "qualified-external"
    );
    Ok(())
}

#[test]
fn typescript_type_roots_resolve_admitted_declaration_packages()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let config = root.join("tsconfig.json");
    let declaration = root.join("types/ambient/index.d.ts");
    let consumer = root.join("src/consumer.ts");
    let config_source = br#"{
  "compilerOptions": { "typeRoots": ["types"] }
}"#;
    let declaration_source = br#"export const ambient = true;"#;
    let consumer_source = br#"import { ambient } from "ambient";
export const value = ambient;
"#;
    for (path, source) in [
        (&config, config_source.as_slice()),
        (&declaration, declaration_source.as_slice()),
        (&consumer, consumer_source.as_slice()),
    ] {
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(path, source)?;
    }
    let extractions = [
        extract(
            declaration.to_str().ok_or("non-UTF-8 fixture path")?,
            declaration_source,
        ),
        extract(
            consumer.to_str().ok_or("non-UTF-8 fixture path")?,
            consumer_source,
        ),
    ];
    let sources = [
        (&config, config_source.as_slice()),
        (&declaration, declaration_source.as_slice()),
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
            .any(|node| source_matches(&node.string("source_file"), &declaration))
    );
    let import = resolved
        .edges
        .iter()
        .find(|edge| {
            edge.string("relation") == "imports_from" && edge.string("module") == "ambient"
        })
        .ok_or("missing typeRoots import")?;
    assert!(target_source_matches(
        &resolved,
        &import.target,
        &declaration
    ));
    assert_eq!(import.string("resolution_rule"), "project-module-binding");
    assert_eq!(
        import.string("project_resolution_rule"),
        "typescript-type-roots"
    );
    Ok(())
}

#[test]
fn typescript_custom_conditions_are_selected_before_default_package_exports()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let config = root.join("tsconfig.json");
    let package = root.join("packages/conditional/package.json");
    let browser = root.join("packages/conditional/browser.ts");
    let fallback = root.join("packages/conditional/fallback.ts");
    let consumer = root.join("src/consumer.ts");
    let config_source = br#"{
  "compilerOptions": { "customConditions": ["browser"] }
}"#;
    let package_source = br#"{
  "name": "@example/conditional",
  "exports": { ".": { "browser": "./browser.ts", "default": "./fallback.ts" } }
}"#;
    let browser_source = br#"export const selected = "browser";"#;
    let fallback_source = br#"export const selected = "fallback";"#;
    let consumer_source = br#"import { selected } from "@example/conditional";
export const value = selected;
"#;
    for (path, source) in [
        (&config, config_source.as_slice()),
        (&package, package_source.as_slice()),
        (&browser, browser_source.as_slice()),
        (&fallback, fallback_source.as_slice()),
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
            browser.to_str().ok_or("non-UTF-8 fixture path")?,
            browser_source,
        ),
        extract(
            fallback.to_str().ok_or("non-UTF-8 fixture path")?,
            fallback_source,
        ),
        extract(
            consumer.to_str().ok_or("non-UTF-8 fixture path")?,
            consumer_source,
        ),
    ];
    let sources = [
        (&config, config_source.as_slice()),
        (&package, package_source.as_slice()),
        (&browser, browser_source.as_slice()),
        (&fallback, fallback_source.as_slice()),
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
            .any(|node| source_matches(&node.string("source_file"), &browser))
    );
    let import = resolved
        .edges
        .iter()
        .find(|edge| {
            edge.string("relation") == "imports_from"
                && edge.string("module") == "@example/conditional"
        })
        .ok_or("missing custom-condition import")?;
    assert!(target_source_matches(&resolved, &import.target, &browser));
    assert_eq!(import.string("package_condition"), "browser");
    Ok(())
}

#[test]
fn typescript_config_extends_cycles_fail_closed_with_diagnostic()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let first = root.join("tsconfig.json");
    let second = root.join("tsconfig.other.json");
    let implementation = root.join("src/api.ts");
    let consumer = root.join("src/consumer.ts");
    let first_source = br#"{
        "extends": "./tsconfig.other.json",
        "compilerOptions": { "paths": { "@/*": ["src/*"] } }
    }"#;
    let second_source = br#"{
        "extends": "./tsconfig.json",
        "compilerOptions": { "baseUrl": "." }
    }"#;
    let implementation_source = br#"export const api = true;"#;
    let consumer_source = br#"import { api } from "@/api";
export const value = api;
"#;
    for (path, source) in [
        (&first, first_source.as_slice()),
        (&second, second_source.as_slice()),
        (&implementation, implementation_source.as_slice()),
        (&consumer, consumer_source.as_slice()),
    ] {
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
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
        (&first, first_source.as_slice()),
        (&second, second_source.as_slice()),
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
    let error = resolved.error.as_deref().unwrap_or_default();
    assert!(error.contains("extends cycle"), "error={error:?}");
    let import = resolved
        .edges
        .iter()
        .find(|edge| edge.string("relation") == "imports_from" && edge.string("module") == "@/api")
        .ok_or("missing cyclic import")?;
    assert_eq!(import.string("resolution_rule"), "qualified-external");
    Ok(())
}

#[test]
fn typescript_candidate_preserves_dynamic_member_as_unresolved() {
    let source = br#"class Known { run() {} }
const value = getValue();
value.run();
new Known().run();
function exact(value: string) {}
exact();
"#;
    let batch = Engine::default()
        .extract_source_universal_candidate_evidence(
            Path::new("src/dynamic.ts"),
            "src/dynamic.ts",
            source,
        )
        .expect("candidate evidence");
    let dynamic_id = batch
        .candidates
        .iter()
        .find(|candidate| {
            candidate.relation == CandidateRelation::Calls
                && candidate.target_spelling == "run"
                && candidate.constraints.exact_target_declaration_id.is_none()
                && candidate.constraints.qualified_name.is_none()
        })
        .map(|candidate| candidate.id.clone())
        .expect("dynamic call candidate");
    let known_id = batch
        .candidates
        .iter()
        .find(|candidate| {
            candidate.relation == CandidateRelation::Calls
                && candidate.target_spelling == "run"
                && candidate.constraints.exact_target_declaration_id.is_some()
        })
        .map(|candidate| candidate.id.clone())
        .expect("known nominal call candidate");
    let arity_mismatch_id = batch
        .candidates
        .iter()
        .find(|candidate| {
            candidate.relation == CandidateRelation::Calls
                && candidate.target_spelling == "exact"
                && candidate.constraints.argument_count == Some(0)
                && candidate.constraints.exact_target_declaration_id.is_none()
        })
        .map(|candidate| candidate.id.clone())
        .expect("arity-mismatch candidate");
    let index = UniversalResolutionIndex::new(&[batch], UniversalResolutionLimits::default())
        .expect("candidate resolution index");
    assert_eq!(
        index.resolve(&dynamic_id),
        compass_resolve::evidence::ResolutionDecision::Unresolved
    );
    assert!(matches!(
        index.resolve(&known_id),
        compass_resolve::evidence::ResolutionDecision::Resolved { .. }
    ));
    assert_eq!(
        index.resolve(&arity_mismatch_id),
        compass_resolve::evidence::ResolutionDecision::Unresolved
    );
}

#[test]
fn typescript_candidate_resolves_typeof_import_query_as_module_dependency()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let items_path = root.join("lib/items.ts");
    let consumer_path = root.join("src/types.ts");
    fs::create_dir_all(items_path.parent().ok_or("items parent")?)?;
    fs::create_dir_all(consumer_path.parent().ok_or("consumer parent")?)?;
    let items_source = br#"export interface Item { value: string }
"#;
    let consumer_source = br#"type Item = (typeof import("../lib/items").Item)["value"];
type Plain = import("../lib/items").Item;
"#;
    fs::write(&items_path, items_source)?;
    fs::write(&consumer_path, consumer_source)?;
    let items_batch = Engine::default().extract_source_universal_candidate_evidence(
        &items_path,
        "lib/items.ts",
        items_source,
    )?;
    let consumer_batch = Engine::default().extract_source_universal_candidate_evidence(
        &consumer_path,
        "src/types.ts",
        consumer_source,
    )?;
    let module = items_batch
        .declarations
        .iter()
        .find(|declaration| declaration.kind == "module")
        .ok_or("missing imported module declaration")?;
    let module_id = module.id.clone();
    let import_candidate_ids = consumer_batch
        .candidates
        .iter()
        .filter(|candidate| {
            candidate.relation == CandidateRelation::Imports
                && candidate
                    .occurrence_id
                    .as_deref()
                    .and_then(|id| {
                        consumer_batch
                            .occurrences
                            .iter()
                            .find(|occurrence| occurrence.id == id)
                    })
                    .is_some_and(|occurrence| occurrence.context.as_deref() == Some("import_type"))
        })
        .map(|candidate| candidate.id.clone())
        .collect::<Vec<_>>();
    assert_eq!(import_candidate_ids.len(), 2);
    let index = UniversalResolutionIndex::new_with_inventory(
        &[items_batch, consumer_batch],
        &[],
        root,
        UniversalResolutionLimits::default(),
    )?;
    for import_candidate_id in import_candidate_ids {
        assert!(matches!(
            index.resolve(&import_candidate_id),
            compass_resolve::evidence::ResolutionDecision::Resolved { ref declaration_id, .. }
                if declaration_id == &module_id
        ));
    }
    Ok(())
}

#[test]
fn typescript_candidate_resolves_relative_and_default_imports_across_files()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/api.ts",
            br#"export class Widget { run() {} }
"#
            .as_slice(),
        ),
        (
            "lib/default.ts",
            br#"class DefaultWidget { run() {} }
export default DefaultWidget;
"#
            .as_slice(),
        ),
        (
            "app/consumer.ts",
            br#"import { Widget } from "../lib/api.js";
import DefaultWidget from "../lib/default";
new Widget();
new Widget().run();
new DefaultWidget();
new DefaultWidget().run();
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
    let widget = batches[0]
        .declarations
        .iter()
        .find(|declaration| declaration.name == "Widget" && declaration.kind == "class")
        .ok_or("missing Widget declaration")?;
    let default_widget = batches[1]
        .declarations
        .iter()
        .find(|declaration| declaration.name == "DefaultWidget" && declaration.kind == "class")
        .ok_or("missing default declaration")?;
    let widget_construct = batches[2]
        .candidates
        .iter()
        .find(|candidate| {
            candidate.relation == CandidateRelation::Constructs
                && candidate.target_spelling == "Widget"
        })
        .ok_or("missing Widget construction candidate")?;
    let default_construct = batches[2]
        .candidates
        .iter()
        .find(|candidate| {
            candidate.relation == CandidateRelation::Constructs
                && candidate.target_spelling == "DefaultWidget"
        })
        .ok_or("missing default construction candidate")?;
    let widget_member_call = batches[2]
        .candidates
        .iter()
        .find(|candidate| {
            candidate.relation == CandidateRelation::Calls
                && candidate.target_spelling == "run"
                && candidate
                    .constraints
                    .qualified_name
                    .as_deref()
                    .is_some_and(|qualified| qualified.ends_with("::Widget.run"))
        })
        .ok_or("missing imported member call candidate")?;
    let default_member_call = batches[2]
        .candidates
        .iter()
        .find(|candidate| {
            candidate.relation == CandidateRelation::Calls
                && candidate.target_spelling == "run"
                && candidate
                    .constraints
                    .qualified_name
                    .as_deref()
                    .is_some_and(|qualified| qualified.contains("default"))
        })
        .ok_or("missing default imported member call candidate")?;
    let widget_run = batches[0]
        .declarations
        .iter()
        .find(|declaration| declaration.name == "run")
        .ok_or("missing Widget.run declaration")?;
    let default_run = batches[1]
        .declarations
        .iter()
        .find(|declaration| declaration.name == "run")
        .ok_or("missing DefaultWidget.run declaration")?;
    let index = UniversalResolutionIndex::new_with_inventory(
        &batches,
        &[],
        root,
        UniversalResolutionLimits::default(),
    )?;
    assert!(matches!(
        index.resolve(&widget_construct.id),
        compass_resolve::evidence::ResolutionDecision::Resolved { ref declaration_id, .. }
            if declaration_id == &widget.id
    ));
    assert!(matches!(
        index.resolve(&default_construct.id),
        compass_resolve::evidence::ResolutionDecision::Resolved { ref declaration_id, .. }
            if declaration_id == &default_widget.id
    ));
    let widget_member_decision = index.resolve(&widget_member_call.id);
    assert!(matches!(
        widget_member_decision,
        compass_resolve::evidence::ResolutionDecision::Resolved { ref declaration_id, .. }
            if declaration_id == &widget_run.id
    ));
    assert!(matches!(
        index.resolve(&default_member_call.id),
        compass_resolve::evidence::ResolutionDecision::Resolved { ref declaration_id, .. }
            if declaration_id == &default_run.id
    ));
    Ok(())
}

#[test]
fn typescript_wildcard_barrels_resolve_transitively_and_fail_closed()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/values.js",
            br#"export function run(value) { return value; }
"#
            .as_slice(),
        ),
        (
            "lib/middle.js",
            br#"export * from "./values.js";
"#
            .as_slice(),
        ),
        (
            "lib/outer.js",
            br#"export * from "./middle.js";
export * as values from "./values.js";
"#
            .as_slice(),
        ),
        (
            "lib/left.js",
            br#"export function same() {}
"#
            .as_slice(),
        ),
        (
            "lib/right.js",
            br#"export function same() {}
"#
            .as_slice(),
        ),
        (
            "lib/ambiguous.js",
            br#"export * from "./left.js";
export * from "./right.js";
"#
            .as_slice(),
        ),
        (
            "lib/cycle-a.js",
            br#"export * from "./cycle-b.js";
"#
            .as_slice(),
        ),
        (
            "lib/cycle-b.js",
            br#"export * from "./cycle-a.js";
"#
            .as_slice(),
        ),
        (
            "app/consumer.js",
            br#"import { run } from '../lib/outer.js';
import { values } from '../lib/outer.js';
import { same } from '../lib/ambiguous.js';
import { cycleValue } from '../lib/cycle-a.js';
run(1);
values.run(2);
same();
cycleValue();
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
        .find(|declaration| declaration.name == "run" && declaration.kind == "function")
        .ok_or("missing wildcard provider declaration")?;
    let calls = batches[8]
        .candidates
        .iter()
        .filter(|candidate| candidate.relation == CandidateRelation::Calls)
        .collect::<Vec<_>>();
    let run_call = calls
        .iter()
        .find(|candidate| {
            candidate.target_spelling == "run"
                && candidate
                    .constraints
                    .qualified_name
                    .as_deref()
                    .is_some_and(|qualified| !qualified.contains("values"))
        })
        .ok_or("missing transitive wildcard call")?;
    let namespace_call = calls
        .iter()
        .find(|candidate| {
            candidate.target_spelling == "run"
                && candidate
                    .constraints
                    .qualified_name
                    .as_deref()
                    .is_some_and(|qualified| qualified.contains("values"))
        })
        .ok_or("missing namespace reexport member call")?;
    let same_call = calls
        .iter()
        .find(|candidate| candidate.target_spelling == "same")
        .ok_or("missing ambiguous wildcard call")?;
    let cycle_call = calls
        .iter()
        .find(|candidate| candidate.target_spelling == "cycleValue")
        .ok_or("missing cyclic wildcard call")?;
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
        index.resolve(&namespace_call.id),
        compass_resolve::evidence::ResolutionDecision::Resolved { ref declaration_id, .. }
            if declaration_id == &run.id
    ));
    assert!(!matches!(
        index.resolve(&same_call.id),
        compass_resolve::evidence::ResolutionDecision::Resolved { .. }
    ));
    assert!(!matches!(
        index.resolve(&cycle_call.id),
        compass_resolve::evidence::ResolutionDecision::Resolved { .. }
    ));
    Ok(())
}

#[test]
fn typescript_export_assignment_resolves_import_equals_and_default_imports()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/api.ts",
            br#"export function run(value: number) { return value; }
export = run;
"#
            .as_slice(),
        ),
        (
            "lib/object-api.ts",
            br#"export = { run: (value: number) => value };
"#
            .as_slice(),
        ),
        (
            "app/consumer.ts",
            br#"import api = require("../lib/api");
import run from "../lib/api";
import object = require("../lib/object-api");
api(1);
run(2);
object.run(3);
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
        .find(|declaration| declaration.name == "run" && declaration.kind == "function")
        .ok_or("missing export-assignment declaration")?;
    let object_run = batches[1]
        .declarations
        .iter()
        .find(|declaration| declaration.qualified_name == "object-api.default.run")
        .ok_or("missing object export-assignment member")?;
    let calls = batches[2]
        .candidates
        .iter()
        .filter(|candidate| candidate.relation == CandidateRelation::Calls)
        .collect::<Vec<_>>();
    let import_equals_call = calls
        .iter()
        .find(|candidate| candidate.target_spelling == "api")
        .ok_or("missing import-equals call")?;
    let default_import_call = calls
        .iter()
        .find(|candidate| {
            candidate.target_spelling == "run"
                && candidate
                    .constraints
                    .qualified_name
                    .as_deref()
                    .is_some_and(|qualified| qualified.contains("../lib/api"))
        })
        .ok_or("missing default-import call")?;
    let object_member_call = calls
        .iter()
        .find(|candidate| {
            candidate.target_spelling == "run"
                && candidate
                    .constraints
                    .qualified_name
                    .as_deref()
                    .is_some_and(|qualified| qualified.contains("../lib/object-api"))
        })
        .ok_or("missing object import-equals member call")?;
    let index = UniversalResolutionIndex::new_with_inventory(
        &batches,
        &[],
        root,
        UniversalResolutionLimits::default(),
    )?;
    for (call, declaration) in [
        (import_equals_call, &run.id),
        (default_import_call, &run.id),
        (object_member_call, &object_run.id),
    ] {
        assert!(matches!(
            index.resolve(&call.id),
            compass_resolve::evidence::ResolutionDecision::Resolved { ref declaration_id, .. }
                if declaration_id == declaration
        ));
    }
    Ok(())
}

#[test]
fn typescript_tagged_template_calls_resolve_imported_tags_with_exact_shape()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/tags.ts",
            br#"export function sql(strings: TemplateStringsArray, value: number) {}
"#
            .as_slice(),
        ),
        (
            "app/consumer.ts",
            br#"import { sql } from "../lib/tags";
import * as tags from "../lib/tags";
sql`select ${42}`;
tags.sql`select ${42}`;
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
    let sql = batches[0]
        .declarations
        .iter()
        .find(|declaration| declaration.kind == "function" && declaration.name == "sql")
        .ok_or("missing sql declaration")?;
    let sql_calls = batches[1]
        .candidates
        .iter()
        .filter(|candidate| {
            candidate.relation == CandidateRelation::Calls && candidate.target_spelling == "sql"
        })
        .collect::<Vec<_>>();
    assert_eq!(sql_calls.len(), 2);
    for sql_call in &sql_calls {
        assert_eq!(sql_call.constraints.argument_count, Some(2));
        assert_eq!(
            sql_call.constraints.argument_types,
            [None, Some("number".to_owned())]
        );
        assert!(matches!(
            sql_call
                .occurrence_id
                .as_ref()
                .and_then(|id| {
                    batches[1]
                        .occurrences
                        .iter()
                        .find(|occurrence| occurrence.id == *id)
                })
                .and_then(|occurrence| occurrence.context.as_deref()),
            Some("tagged_template" | "tagged_member")
        ));
    }
    let index = UniversalResolutionIndex::new_with_inventory(
        &batches,
        &[],
        root,
        UniversalResolutionLimits::default(),
    )?;
    for sql_call in sql_calls {
        assert!(matches!(
            index.resolve(&sql_call.id),
            compass_resolve::evidence::ResolutionDecision::Resolved { ref declaration_id, .. }
                if declaration_id == &sql.id
        ));
    }
    Ok(())
}

#[test]
fn typescript_decorator_factories_resolve_direct_and_namespace_imports()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/decorators.ts",
            br#"export function Controller(options: any) {}
"#
            .as_slice(),
        ),
        (
            "app/service.ts",
            br#"import { Controller } from "../lib/decorators";
import * as decorators from "../lib/decorators";
@Controller({ path: "/users" })
class Users {}
@decorators.Controller({ path: "/admin" })
class Admin {}
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
    let controller = batches[0]
        .declarations
        .iter()
        .find(|declaration| declaration.kind == "function" && declaration.name == "Controller")
        .ok_or("missing Controller declaration")?;
    let decorations = batches[1]
        .candidates
        .iter()
        .filter(|candidate| {
            candidate.relation == CandidateRelation::Decorates
                && candidate.target_spelling == "Controller"
        })
        .collect::<Vec<_>>();
    assert_eq!(decorations.len(), 2);
    for decoration in &decorations {
        assert_eq!(decoration.constraints.argument_count, Some(1));
        assert_eq!(
            decoration.constraints.argument_types,
            [Some("object".to_owned())]
        );
        assert!(matches!(
            decoration
                .occurrence_id
                .as_ref()
                .and_then(|id| {
                    batches[1]
                        .occurrences
                        .iter()
                        .find(|occurrence| occurrence.id == *id)
                })
                .and_then(|occurrence| occurrence.context.as_deref()),
            Some("decorator" | "decorator_member")
        ));
    }
    let index = UniversalResolutionIndex::new_with_inventory(
        &batches,
        &[],
        root,
        UniversalResolutionLimits::default(),
    )?;
    for decoration in decorations {
        assert!(matches!(
            index.resolve(&decoration.id),
            compass_resolve::evidence::ResolutionDecision::Resolved { ref declaration_id, .. }
                if declaration_id == &controller.id
        ));
    }
    Ok(())
}

#[test]
fn typescript_candidate_does_not_use_terminal_name_for_relative_imports()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "app/api.ts",
            br#"export class Widget {}
"#
            .as_slice(),
        ),
        (
            "lib/api.ts",
            br#"export class Widget {}
"#
            .as_slice(),
        ),
        (
            "app/consumer.ts",
            br#"import { Widget } from "./api";
new Widget();
"#
            .as_slice(),
        ),
    ];
    let mut batches = Vec::new();
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(&path, source)?;
        batches.push(
            Engine::default()
                .extract_source_universal_candidate_evidence(&path, relative, source)?,
        );
    }
    let app_widget = batches[0]
        .declarations
        .iter()
        .find(|declaration| declaration.name == "Widget" && declaration.kind == "class")
        .ok_or("missing app Widget")?;
    let lib_widget = batches[1]
        .declarations
        .iter()
        .find(|declaration| declaration.name == "Widget" && declaration.kind == "class")
        .ok_or("missing lib Widget")?;
    let construct = batches[2]
        .candidates
        .iter()
        .find(|candidate| candidate.relation == CandidateRelation::Constructs)
        .ok_or("missing construction candidate")?;
    let index = UniversalResolutionIndex::new_with_inventory(
        &batches,
        &[],
        root,
        UniversalResolutionLimits::default(),
    )?;
    assert!(matches!(
        index.resolve(&construct.id),
        compass_resolve::evidence::ResolutionDecision::Resolved { ref declaration_id, .. }
            if declaration_id == &app_widget.id
    ));
    assert_ne!(app_widget.id, lib_widget.id);
    Ok(())
}

#[test]
fn typescript_candidate_resolves_exact_javascript_interop() -> Result<(), Box<dyn std::error::Error>>
{
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/runtime.js",
            br#"export class Runtime { run() {} }
"#
            .as_slice(),
        ),
        (
            "app/consumer.ts",
            br#"import { Runtime } from "../lib/runtime.js";
new Runtime().run();
"#
            .as_slice(),
        ),
    ];
    let mut batches = Vec::new();
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(&path, source)?;
        batches.push(
            Engine::default()
                .extract_source_universal_candidate_evidence(&path, relative, source)?,
        );
    }
    let runtime = batches[0]
        .declarations
        .iter()
        .find(|declaration| declaration.name == "Runtime" && declaration.kind == "class")
        .ok_or("missing JavaScript Runtime declaration")?;
    assert_eq!(runtime.language, "javascript");
    let run = batches[0]
        .declarations
        .iter()
        .find(|declaration| declaration.name == "run")
        .ok_or("missing JavaScript Runtime.run declaration")?;
    let construct = batches[1]
        .candidates
        .iter()
        .find(|candidate| candidate.relation == CandidateRelation::Constructs)
        .ok_or("missing interop construction candidate")?;
    let call = batches[1]
        .candidates
        .iter()
        .find(|candidate| {
            candidate.relation == CandidateRelation::Calls && candidate.target_spelling == "run"
        })
        .ok_or("missing interop member call candidate")?;
    let index = UniversalResolutionIndex::new_with_inventory(
        &batches,
        &[],
        root,
        UniversalResolutionLimits::default(),
    )?;
    assert!(matches!(
        index.resolve(&construct.id),
        compass_resolve::evidence::ResolutionDecision::Resolved { ref declaration_id, .. }
            if declaration_id == &runtime.id
    ));
    assert!(matches!(
        index.resolve(&call.id),
        compass_resolve::evidence::ResolutionDecision::Resolved { ref declaration_id, .. }
            if declaration_id == &run.id
    ));
    Ok(())
}

#[test]
fn typescript_candidate_follows_cross_file_reexport_aliases()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/api.ts",
            br#"export class Widget { run() {} }
"#
            .as_slice(),
        ),
        (
            "lib/barrel.ts",
            br#"export { Widget as PublicWidget } from "./api";
"#
            .as_slice(),
        ),
        (
            "app/consumer.ts",
            br#"import { PublicWidget } from "../lib/barrel";
new PublicWidget().run();
"#
            .as_slice(),
        ),
    ];
    let mut batches = Vec::new();
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(&path, source)?;
        batches.push(
            Engine::default()
                .extract_source_universal_candidate_evidence(&path, relative, source)?,
        );
    }
    let widget = batches[0]
        .declarations
        .iter()
        .find(|declaration| declaration.name == "Widget" && declaration.kind == "class")
        .ok_or("missing re-exported Widget declaration")?;
    let run = batches[0]
        .declarations
        .iter()
        .find(|declaration| declaration.name == "run")
        .ok_or("missing re-exported Widget.run declaration")?;
    let construct = batches[2]
        .candidates
        .iter()
        .find(|candidate| candidate.relation == CandidateRelation::Constructs)
        .ok_or("missing re-exported construction candidate")?;
    let call = batches[2]
        .candidates
        .iter()
        .find(|candidate| {
            candidate.relation == CandidateRelation::Calls && candidate.target_spelling == "run"
        })
        .ok_or("missing re-exported member call candidate")?;
    let index = UniversalResolutionIndex::new_with_inventory(
        &batches,
        &[],
        root,
        UniversalResolutionLimits::default(),
    )?;
    assert!(matches!(
        index.resolve(&construct.id),
        compass_resolve::evidence::ResolutionDecision::Resolved { ref declaration_id, .. }
            if declaration_id == &widget.id
    ));
    assert!(matches!(
        index.resolve(&call.id),
        compass_resolve::evidence::ResolutionDecision::Resolved { ref declaration_id, .. }
            if declaration_id == &run.id
    ));
    Ok(())
}

#[test]
fn typescript_candidate_keeps_duplicate_module_realizations_ambiguous()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "app/api.ts",
            br#"export class Widget {}
"#
            .as_slice(),
        ),
        (
            "app/api.js",
            br#"export class Widget {}
"#
            .as_slice(),
        ),
        (
            "app/consumer.ts",
            br#"import { Widget } from "./api";
new Widget();
"#
            .as_slice(),
        ),
    ];
    let mut batches = Vec::new();
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(&path, source)?;
        batches.push(
            Engine::default()
                .extract_source_universal_candidate_evidence(&path, relative, source)?,
        );
    }
    let construct = batches[2]
        .candidates
        .iter()
        .find(|candidate| candidate.relation == CandidateRelation::Constructs)
        .ok_or("missing construction candidate")?;
    let index = UniversalResolutionIndex::new_with_inventory(
        &batches,
        &[],
        root,
        UniversalResolutionLimits::default(),
    )?;
    assert!(matches!(
        index.resolve(&construct.id),
        compass_resolve::evidence::ResolutionDecision::Ambiguous { candidate_count } if candidate_count == 2
    ));
    Ok(())
}

#[test]
fn typescript_candidate_consumes_project_path_targets_in_shared_resolution()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "tsconfig.json",
            br#"{
  "compilerOptions": {
    "baseUrl": ".",
    "paths": { "@/*": ["src/*"] }
  }
}
"#
            .as_slice(),
        ),
        (
            "src/api.ts",
            br#"export class Widget { run() {} }
"#
            .as_slice(),
        ),
        (
            "app/consumer.ts",
            br#"import { Widget } from "@/api";
new Widget().run();
"#
            .as_slice(),
        ),
    ];
    let mut extractions = Vec::new();
    let mut sources = HashMap::new();
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(&path, source)?;
        sources.insert(relative.to_owned(), String::from_utf8(source.to_vec())?);
        let mut extraction = extract(relative, source);
        if relative.ends_with(".ts") {
            extraction.semantic_evidence = Some(
                Engine::default().extract_source_universal_candidate_evidence(
                    Path::new(relative),
                    relative,
                    source,
                )?,
            );
        }
        extractions.push(extraction);
    }
    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    let owned = compass_resolve::resolve_owned_with_root(extractions, &sources, root);
    for resolved in [&resolved, &owned] {
        assert!(
            resolved.error.is_none(),
            "resolver error: {:?}",
            resolved.error
        );
        let calls = resolved
            .edges
            .iter()
            .filter(|edge| {
                edge.string("source_file") == "app/consumer.ts"
                    && edge.string("relation") == "calls"
            })
            .collect::<Vec<_>>();
        assert!(
            calls.iter().any(|edge| {
                edge.string("resolution_rule") == "project-module-binding"
                    && resolved.nodes.iter().any(|node| {
                        node.id == edge.target
                            && node.string("source_file") == "src/api.ts"
                            && node.label() == "Widget"
                    })
            }),
            "project construction target missing: {calls:#?}"
        );
        assert!(
            calls.iter().any(|edge| {
                edge.string("resolution_rule") == "member-binding"
                    && resolved.nodes.iter().any(|node| {
                        node.id == edge.target
                            && node.string("source_file") == "src/api.ts"
                            && node.label() == ".run()"
                    })
            }),
            "project member target missing: {calls:#?}"
        );
    }
    Ok(())
}

#[test]
fn typescript_candidate_merges_imported_interface_members_across_declarations()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/types.ts",
            br#"export interface Config { run(): void }
export interface Config { inspect(): void }
"#
            .as_slice(),
        ),
        (
            "app/consumer.ts",
            br#"import type { Config } from "../lib/types";
export function use(config: Config) { config.inspect(); }
"#
            .as_slice(),
        ),
    ];
    let mut extractions = Vec::new();
    let mut sources = HashMap::new();
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(&path, source)?;
        sources.insert(relative.to_owned(), String::from_utf8(source.to_vec())?);
        let mut extraction = extract(relative, source);
        extraction.semantic_evidence = Some(
            Engine::default().extract_source_universal_candidate_evidence(
                Path::new(relative),
                relative,
                source,
            )?,
        );
        extractions.push(extraction);
    }
    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    let owned = compass_resolve::resolve_owned_with_root(extractions, &sources, root);
    for resolved in [&resolved, &owned] {
        assert!(
            resolved.error.is_none(),
            "resolver error: {:?}",
            resolved.error
        );
        let inspect = resolved
            .nodes
            .iter()
            .find(|node| {
                node.string("source_file") == "lib/types.ts" && node.label() == ".inspect()"
            })
            .ok_or("missing merged interface member")?;
        assert!(resolved.edges.iter().any(|edge| {
            edge.string("relation") == "calls"
                && edge.string("source_file") == "app/consumer.ts"
                && edge.target == inspect.id
                && edge.string("resolution_rule") == "member-binding"
        }));
    }
    Ok(())
}

#[test]
fn typescript_candidate_leaves_duplicate_merged_interface_members_ambiguous()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/types.ts",
            br#"export interface Config { inspect(): void }
export interface Config { inspect(): void }
"#
            .as_slice(),
        ),
        (
            "app/consumer.ts",
            br#"import type { Config } from "../lib/types";
export function use(config: Config) { config.inspect(); }
"#
            .as_slice(),
        ),
    ];
    let mut extractions = Vec::new();
    let mut sources = HashMap::new();
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(&path, source)?;
        sources.insert(relative.to_owned(), String::from_utf8(source.to_vec())?);
        let mut extraction = extract(relative, source);
        extraction.semantic_evidence = Some(
            Engine::default().extract_source_universal_candidate_evidence(
                Path::new(relative),
                relative,
                source,
            )?,
        );
        extractions.push(extraction);
    }
    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert!(
        resolved.error.is_none(),
        "resolver error: {:?}",
        resolved.error
    );
    assert!(!resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls" && edge.string("source_file") == "app/consumer.ts"
    }));
    Ok(())
}

#[test]
fn typescript_candidate_resolves_imported_generic_member_chain()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/item.ts",
            br#"export interface Item { inspect(): void }
"#
            .as_slice(),
        ),
        (
            "lib/types.ts",
            br#"export interface Box<T> { item: T }
"#
            .as_slice(),
        ),
        (
            "app/consumer.ts",
            br#"import type { Box } from "../lib/types";
import type { Item } from "../lib/item";
export function use(box: Box<Item>) { box.item.inspect(); }
"#
            .as_slice(),
        ),
    ];
    let mut extractions = Vec::new();
    let mut sources = HashMap::new();
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(&path, source)?;
        sources.insert(relative.to_owned(), String::from_utf8(source.to_vec())?);
        let mut extraction = extract(relative, source);
        extraction.semantic_evidence = Some(
            Engine::default().extract_source_universal_candidate_evidence(
                Path::new(relative),
                relative,
                source,
            )?,
        );
        extractions.push(extraction);
    }
    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert!(
        resolved.error.is_none(),
        "resolver error: {:?}",
        resolved.error
    );
    let inspect = resolved
        .nodes
        .iter()
        .find(|node| node.string("source_file") == "lib/item.ts" && node.label() == ".inspect()")
        .ok_or("missing imported generic member")?;
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.string("source_file") == "app/consumer.ts"
            && edge.target == inspect.id
            && edge.string("resolution_rule") == "member-binding"
    }));
    Ok(())
}

#[test]
fn typescript_candidate_resolves_nested_imported_generic_member_chain()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/item.ts",
            br#"export interface Item { inspect(): void }
"#
            .as_slice(),
        ),
        (
            "lib/types.ts",
            br#"export interface Wrapper<U> { value: U }
export interface Box<T> { item: T }
"#
            .as_slice(),
        ),
        (
            "app/consumer.ts",
            br#"import type { Box, Wrapper } from "../lib/types";
import type { Item } from "../lib/item";
export function use(box: Box<Wrapper<Item>>) { box.item.value.inspect(); }
"#
            .as_slice(),
        ),
    ];
    let mut extractions = Vec::new();
    let mut sources = HashMap::new();
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(&path, source)?;
        sources.insert(relative.to_owned(), String::from_utf8(source.to_vec())?);
        let mut extraction = extract(relative, source);
        extraction.semantic_evidence = Some(
            Engine::default().extract_source_universal_candidate_evidence(
                Path::new(relative),
                relative,
                source,
            )?,
        );
        extractions.push(extraction);
    }
    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert!(
        resolved.error.is_none(),
        "resolver error: {:?}",
        resolved.error
    );
    let inspect = resolved
        .nodes
        .iter()
        .find(|node| node.string("source_file") == "lib/item.ts" && node.label() == ".inspect()")
        .ok_or("missing nested imported generic member")?;
    let consumer_evidence = extractions
        .iter()
        .filter_map(|extraction| extraction.semantic_evidence.as_ref())
        .find(|evidence| {
            evidence
                .declarations
                .iter()
                .any(|declaration| declaration.range.source_file == "app/consumer.ts")
        })
        .ok_or("missing nested consumer evidence")?;
    let nested_call = consumer_evidence
        .candidates
        .iter()
        .find(|candidate| candidate.relation == CandidateRelation::Calls)
        .ok_or("missing nested generic call candidate")?;
    assert_eq!(
        nested_call.constraints.qualified_name.as_deref(),
        Some("../lib/types::Box<../lib/types::Wrapper<../lib/item::Item>>.item.value.inspect")
    );
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.string("source_file") == "app/consumer.ts"
            && edge.target == inspect.id
            && edge.string("resolution_rule") == "member-binding"
    }));
    Ok(())
}

#[test]
fn typescript_candidate_keeps_nested_generic_member_ambiguity_unresolved()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/item.ts",
            br#"export interface Item { inspect(): void }
export interface Item { inspect(): void }
"#
            .as_slice(),
        ),
        (
            "lib/types.ts",
            br#"export interface Wrapper<U> { value: U }
export interface Box<T> { item: T }
"#
            .as_slice(),
        ),
        (
            "app/consumer.ts",
            br#"import type { Box, Wrapper } from "../lib/types";
import type { Item } from "../lib/item";
export function use(box: Box<Wrapper<Item>>) { box.item.value.inspect(); }
"#
            .as_slice(),
        ),
    ];
    let mut extractions = Vec::new();
    let mut sources = HashMap::new();
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(&path, source)?;
        sources.insert(relative.to_owned(), String::from_utf8(source.to_vec())?);
        let mut extraction = extract(relative, source);
        extraction.semantic_evidence = Some(
            Engine::default().extract_source_universal_candidate_evidence(
                Path::new(relative),
                relative,
                source,
            )?,
        );
        extractions.push(extraction);
    }
    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert!(
        resolved.error.is_none(),
        "resolver error: {:?}",
        resolved.error
    );
    assert!(!resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls" && edge.string("source_file") == "app/consumer.ts"
    }));
    Ok(())
}

#[test]
fn typescript_candidate_does_not_invent_nested_generic_primitive_members()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/types.ts",
            br#"export interface Box<T> { item: T }
"#
            .as_slice(),
        ),
        (
            "app/consumer.ts",
            br#"import type { Box } from "../lib/types";
export function use(box: Box<string>) { box.item.inspect(); }
"#
            .as_slice(),
        ),
    ];
    let mut extractions = Vec::new();
    let mut sources = HashMap::new();
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(&path, source)?;
        sources.insert(relative.to_owned(), String::from_utf8(source.to_vec())?);
        let mut extraction = extract(relative, source);
        extraction.semantic_evidence = Some(
            Engine::default().extract_source_universal_candidate_evidence(
                Path::new(relative),
                relative,
                source,
            )?,
        );
        extractions.push(extraction);
    }
    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert!(
        resolved.error.is_none(),
        "resolver error: {:?}",
        resolved.error
    );
    assert!(!resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.string("source_file") == "app/consumer.ts"
            && edge.string("resolution_rule") == "member-binding"
    }));
    Ok(())
}

#[test]
fn typescript_candidate_resolves_imported_generic_object_type_alias_members()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/item.ts",
            br#"export interface Item { inspect(): void }
"#
            .as_slice(),
        ),
        (
            "lib/types.ts",
            br#"export type Boxed<T> = { value: T };
"#
            .as_slice(),
        ),
        (
            "app/consumer.ts",
            br#"import type { Boxed } from "../lib/types";
import type { Item } from "../lib/item";
export function use(box: Boxed<Item>) { box.value.inspect(); }
"#
            .as_slice(),
        ),
    ];
    let mut extractions = Vec::new();
    let mut sources = HashMap::new();
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(&path, source)?;
        sources.insert(relative.to_owned(), String::from_utf8(source.to_vec())?);
        let mut extraction = extract(relative, source);
        extraction.semantic_evidence = Some(
            Engine::default().extract_source_universal_candidate_evidence(
                Path::new(relative),
                relative,
                source,
            )?,
        );
        extractions.push(extraction);
    }
    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert!(
        resolved.error.is_none(),
        "resolver error: {:?}",
        resolved.error
    );
    let inspect = resolved
        .nodes
        .iter()
        .find(|node| node.string("source_file") == "lib/item.ts" && node.label() == ".inspect()")
        .ok_or("missing imported alias member")?;
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.string("source_file") == "app/consumer.ts"
            && edge.target == inspect.id
            && edge.string("resolution_rule") == "member-binding"
    }));
    Ok(())
}

#[test]
fn typescript_candidate_resolves_imported_nominal_generic_type_alias_members()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/item.ts",
            br#"export interface Item { inspect(): void }
"#
            .as_slice(),
        ),
        (
            "lib/types.ts",
            br#"import type { Box } from "./box";
export type Alias<T> = Box<T>;
"#
            .as_slice(),
        ),
        (
            "lib/box.ts",
            br#"export interface Box<T> { value: T }
"#
            .as_slice(),
        ),
        (
            "app/consumer.ts",
            br#"import type { Alias } from "../lib/types";
import type { Item } from "../lib/item";
export function use(box: Alias<Item>) { box.value.inspect(); }
"#
            .as_slice(),
        ),
    ];
    let mut extractions = Vec::new();
    let mut sources = HashMap::new();
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(&path, source)?;
        sources.insert(relative.to_owned(), String::from_utf8(source.to_vec())?);
        let mut extraction = extract(relative, source);
        extraction.semantic_evidence = Some(
            Engine::default().extract_source_universal_candidate_evidence(
                Path::new(relative),
                relative,
                source,
            )?,
        );
        extractions.push(extraction);
    }
    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert!(
        resolved.error.is_none(),
        "resolver error: {:?}",
        resolved.error
    );
    let inspect = resolved
        .nodes
        .iter()
        .find(|node| node.string("source_file") == "lib/item.ts" && node.label() == ".inspect()")
        .ok_or("missing nominal alias member")?;
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.string("source_file") == "app/consumer.ts"
            && edge.target == inspect.id
            && edge.string("resolution_rule") == "member-binding"
    }));
    Ok(())
}

#[test]
fn typescript_candidate_resolves_imported_homomorphic_mapped_alias_members()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/item.ts",
            br#"export interface Item { inspect(): void }
"#
            .as_slice(),
        ),
        (
            "lib/types.ts",
            br#"export type Copy<T> = { [K in keyof T]: T[K] };
"#
            .as_slice(),
        ),
        (
            "app/consumer.ts",
            br#"import type { Copy } from "../lib/types";
import type { Item } from "../lib/item";
export function use(value: Copy<Item>) { value.inspect(); }
"#
            .as_slice(),
        ),
    ];
    let mut extractions = Vec::new();
    let mut sources = HashMap::new();
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(&path, source)?;
        sources.insert(relative.to_owned(), String::from_utf8(source.to_vec())?);
        let mut extraction = extract(relative, source);
        extraction.semantic_evidence = Some(
            Engine::default().extract_source_universal_candidate_evidence(
                Path::new(relative),
                relative,
                source,
            )?,
        );
        extractions.push(extraction);
    }
    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert!(
        resolved.error.is_none(),
        "resolver error: {:?}",
        resolved.error
    );
    let inspect = resolved
        .nodes
        .iter()
        .find(|node| node.string("source_file") == "lib/item.ts" && node.label() == ".inspect()")
        .ok_or("missing imported mapped member")?;
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.string("source_file") == "app/consumer.ts"
            && edge.target == inspect.id
            && edge.string("resolution_rule") == "member-binding"
    }));
    Ok(())
}

#[test]
fn typescript_candidate_publishes_local_conditional_branch_members()
-> Result<(), Box<dyn std::error::Error>> {
    let source = br#"class Item { inspect(): void {} }
class Other { other(): void {} }
type Choose<T> = T extends Item ? Item : Other;
type ChooseObject<T> = T extends object ? T : never;
function selected(value: Choose<Item>) { value.inspect(); }
function rejected(value: Choose<Other>) { value.inspect(); }
function union(value: Choose<Item | Other>) { value.inspect(); }
function direct(value: Item extends Item ? Item : Other) { value.inspect(); }
function object(value: ChooseObject<Item>) { value.inspect(); }
"#;
    let mut extraction = extract("src/conditional.ts", source);
    extraction.semantic_evidence = Some(
        Engine::default().extract_source_universal_candidate_evidence(
            Path::new("src/conditional.ts"),
            "src/conditional.ts",
            source,
        )?,
    );
    let resolved = compass_resolve::resolve(
        &[extraction],
        &HashMap::from([(
            "src/conditional.ts".to_owned(),
            String::from_utf8(source.to_vec())?,
        )]),
    );
    assert!(
        resolved.error.is_none(),
        "resolver error: {:?}",
        resolved.error
    );
    let inspect = resolved
        .nodes
        .iter()
        .find(|node| {
            node.string("source_file") == "src/conditional.ts" && node.label() == ".inspect()"
        })
        .ok_or("missing conditional Item.inspect member")?;
    let calls = resolved
        .edges
        .iter()
        .filter(|edge| edge.string("relation") == "calls" && edge.target == inspect.id)
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 3);
    assert!(
        calls
            .iter()
            .all(|edge| { edge.string("source_file") == "src/conditional.ts" })
    );
    Ok(())
}

#[test]
fn typescript_candidate_publishes_literal_indexed_alias_member_calls()
-> Result<(), Box<dyn std::error::Error>> {
    let source = br#"interface Nested { inspect(): void }
interface Item { nested: Nested }
type NestedAlias = Item["nested"];
export function use(value: NestedAlias) { value.inspect(); }
"#;
    let relative = "src/indexed-alias.ts";
    let mut extraction = extract(relative, source);
    extraction.semantic_evidence = Some(
        Engine::default().extract_source_universal_candidate_evidence(
            Path::new(relative),
            relative,
            source,
        )?,
    );
    let resolved = compass_resolve::resolve(
        &[extraction],
        &HashMap::from([(relative.to_owned(), String::from_utf8(source.to_vec())?)]),
    );
    assert!(
        resolved.error.is_none(),
        "resolver error: {:?}",
        resolved.error
    );
    let inspect = resolved
        .nodes
        .iter()
        .find(|node| node.string("source_file") == relative && node.label() == ".inspect()")
        .ok_or("missing indexed alias Nested.inspect member")?;
    let calls = resolved
        .edges
        .iter()
        .filter(|edge| {
            edge.string("relation") == "calls"
                && edge.string("source_file") == relative
                && edge.target == inspect.id
        })
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].string("resolution_rule"),
        "exact-source-declaration"
    );
    Ok(())
}

#[test]
fn typescript_candidate_resolves_imported_generic_indexed_alias_member_calls()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/item.ts",
            br#"export interface Nested { inspect(): void }
export interface Item { nested: Nested }
"#
            .as_slice(),
        ),
        (
            "lib/types.ts",
            br#"export type NestedOf<T> = T["nested"];
"#
            .as_slice(),
        ),
        (
            "app/consumer.ts",
            br#"import type { NestedOf } from "../lib/types";
import type { Item } from "../lib/item";
export function use(value: NestedOf<Item>) { value.inspect(); }
"#
            .as_slice(),
        ),
    ];
    let mut extractions = Vec::new();
    let mut sources = HashMap::new();
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(&path, source)?;
        sources.insert(relative.to_owned(), String::from_utf8(source.to_vec())?);
        let mut extraction = extract(relative, source);
        extraction.semantic_evidence = Some(
            Engine::default().extract_source_universal_candidate_evidence(
                Path::new(relative),
                relative,
                source,
            )?,
        );
        extractions.push(extraction);
    }
    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert!(
        resolved.error.is_none(),
        "resolver error: {:?}",
        resolved.error
    );
    let inspect = resolved
        .nodes
        .iter()
        .find(|node| node.string("source_file") == "lib/item.ts" && node.label() == ".inspect()")
        .ok_or("missing imported indexed alias Nested.inspect member")?;
    let calls = resolved
        .edges
        .iter()
        .filter(|edge| {
            edge.string("relation") == "calls"
                && edge.string("source_file") == "app/consumer.ts"
                && edge.target == inspect.id
        })
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 1);
    Ok(())
}

#[test]
fn typescript_candidate_resolves_imported_keyof_identity_alias_members()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/item.ts",
            br#"export interface Item { inspect(): void }
"#
            .as_slice(),
        ),
        (
            "lib/types.ts",
            br#"export type Copy<T> = Pick<T, keyof T>;
export type Empty<T> = Omit<T, keyof T>;
"#
            .as_slice(),
        ),
        (
            "app/consumer.ts",
            br#"import type { Copy, Empty } from "../lib/types";
import type { Item } from "../lib/item";
export function use(value: Copy<Item>) { value.inspect(); }
export function rejected(value: Empty<Item>) { value.inspect(); }
"#
            .as_slice(),
        ),
    ];
    let mut extractions = Vec::new();
    let mut sources = HashMap::new();
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(&path, source)?;
        sources.insert(relative.to_owned(), String::from_utf8(source.to_vec())?);
        let mut extraction = extract(relative, source);
        extraction.semantic_evidence = Some(
            Engine::default().extract_source_universal_candidate_evidence(
                Path::new(relative),
                relative,
                source,
            )?,
        );
        extractions.push(extraction);
    }
    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert!(
        resolved.error.is_none(),
        "resolver error: {:?}",
        resolved.error
    );
    let inspect = resolved
        .nodes
        .iter()
        .find(|node| node.string("source_file") == "lib/item.ts" && node.label() == ".inspect()")
        .ok_or("missing imported keyof identity Item.inspect member")?;
    let calls = resolved
        .edges
        .iter()
        .filter(|edge| {
            edge.string("relation") == "calls"
                && edge.string("source_file") == "app/consumer.ts"
                && edge.target == inspect.id
        })
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 1);
    Ok(())
}

#[test]
fn typescript_candidate_resolves_imported_literal_utility_projection_members()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/item.ts",
            br#"export interface Item {
    enabled(): void;
    debug(): void;
}
"#
            .as_slice(),
        ),
        (
            "lib/types.ts",
            br#"import type { Item } from "./item";
export type Picked = Pick<Item, "enabled">;
export type Omitted = Omit<Item, "debug">;
"#
            .as_slice(),
        ),
        (
            "app/consumer.ts",
            br#"import type { Picked, Omitted } from "../lib/types";
export function use(picked: Picked, omitted: Omitted) {
    picked.enabled();
    picked.debug();
    omitted.enabled();
    omitted.debug();
}
"#
            .as_slice(),
        ),
    ];
    let mut extractions = Vec::new();
    let mut sources = HashMap::new();
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(&path, source)?;
        sources.insert(relative.to_owned(), String::from_utf8(source.to_vec())?);
        let mut extraction = extract(relative, source);
        extraction.semantic_evidence = Some(
            Engine::default().extract_source_universal_candidate_evidence(
                Path::new(relative),
                relative,
                source,
            )?,
        );
        extractions.push(extraction);
    }
    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert!(
        resolved.error.is_none(),
        "resolver error: {:?}",
        resolved.error
    );
    let enabled = resolved
        .nodes
        .iter()
        .find(|node| node.string("source_file") == "lib/item.ts" && node.label() == ".enabled()")
        .ok_or("missing imported Pick enabled member")?;
    let debug = resolved
        .nodes
        .iter()
        .find(|node| node.string("source_file") == "lib/item.ts" && node.label() == ".debug()")
        .ok_or("missing imported Omit debug member")?;
    let enabled_calls = resolved
        .edges
        .iter()
        .filter(|edge| {
            edge.string("relation") == "calls"
                && edge.string("source_file") == "app/consumer.ts"
                && edge.target == enabled.id
        })
        .collect::<Vec<_>>();
    let debug_calls = resolved
        .edges
        .iter()
        .filter(|edge| {
            edge.string("relation") == "calls"
                && edge.string("source_file") == "app/consumer.ts"
                && edge.target == debug.id
        })
        .collect::<Vec<_>>();
    assert_eq!(enabled_calls.len(), 2);
    assert!(debug_calls.is_empty());
    Ok(())
}

#[test]
fn typescript_candidate_resolves_imported_array_and_tuple_member_chains()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/item.ts",
            br#"export interface Item { inspect(): void }
"#
            .as_slice(),
        ),
        (
            "lib/types.ts",
            br#"export interface Box<T> {
    values: T[];
    pair: [T, string];
    nullable: NonNullable<T | undefined>;
    awaited: Awaited<Promise<T>>;
    readonlyValue: Readonly<T>;
}
"#
            .as_slice(),
        ),
        (
            "app/consumer.ts",
            br#"import type { Item } from "../lib/item";
import type { Box } from "../lib/types";
export function use(values: Item[], box: Box<Item>) {
    values[0].inspect();
    box.values[0].inspect();
    box.pair[0].inspect();
    box.nullable.inspect();
    box.awaited.inspect();
    box.readonlyValue.inspect();
}
"#
            .as_slice(),
        ),
    ];
    let mut extractions = Vec::new();
    let mut sources = HashMap::new();
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(&path, source)?;
        sources.insert(relative.to_owned(), String::from_utf8(source.to_vec())?);
        let mut extraction = extract(relative, source);
        extraction.semantic_evidence = Some(
            Engine::default().extract_source_universal_candidate_evidence(
                Path::new(relative),
                relative,
                source,
            )?,
        );
        extractions.push(extraction);
    }
    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert!(
        resolved.error.is_none(),
        "resolver error: {:?}",
        resolved.error
    );
    let inspect = resolved
        .nodes
        .iter()
        .find(|node| node.string("source_file") == "lib/item.ts" && node.label() == ".inspect()")
        .ok_or("missing imported array element member")?;
    let calls = resolved
        .edges
        .iter()
        .filter(|edge| {
            edge.string("relation") == "calls" && edge.string("source_file") == "app/consumer.ts"
        })
        .collect::<Vec<_>>();
    assert_eq!(
        calls
            .iter()
            .filter(|edge| edge.target == inspect.id)
            .count(),
        6
    );
    assert!(
        calls
            .iter()
            .all(|edge| edge.string("resolution_rule") == "member-binding")
    );
    Ok(())
}

#[test]
fn typescript_candidate_resolves_generic_callable_return_member_chain()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let relative = "src/generic-return.ts";
    let source = br#"class Item { inspect(): void {} }
function identity<T>(value: T): T { return value; }
export function use() { identity(new Item()).inspect(); }
"#;
    let path = root.join(relative);
    fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
    fs::write(&path, source)?;
    let mut extraction = extract(relative, source);
    extraction.semantic_evidence = Some(
        Engine::default().extract_source_universal_candidate_evidence(
            Path::new(relative),
            relative,
            source,
        )?,
    );
    let mut sources = HashMap::new();
    sources.insert(relative.to_owned(), String::from_utf8(source.to_vec())?);
    let resolved = compass_resolve::resolve_with_root(&[extraction], &sources, root);
    assert!(
        resolved.error.is_none(),
        "resolver error: {:?}",
        resolved.error
    );
    let inspect = resolved
        .nodes
        .iter()
        .find(|node| node.string("source_file") == relative && node.label() == ".inspect()")
        .ok_or("missing generic return member")?;
    let calls = resolved
        .edges
        .iter()
        .filter(|edge| edge.string("relation") == "calls" && edge.string("source_file") == relative)
        .collect::<Vec<_>>();
    let inspect_calls = calls
        .iter()
        .filter(|edge| edge.target == inspect.id)
        .collect::<Vec<_>>();
    assert_eq!(inspect_calls.len(), 1);
    assert_eq!(
        inspect_calls[0].string("resolution_rule"),
        "exact-source-declaration"
    );
    Ok(())
}

#[test]
fn typescript_candidate_resolves_imported_callable_return_member_chains()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/item.ts",
            br#"export interface Item { inspect(): void }
"#
            .as_slice(),
        ),
        (
            "lib/factory.ts",
            br#"import type { Item } from "./item";
export function make(value: Item): Item { return value; }
export const makeArrow = (value: Item): Item => value;
export function identity<T>(value: T): T { return value; }
export interface Box<T> { value: T }
export function box<T>(value: T): Box<T> { return { value }; }
"#
            .as_slice(),
        ),
        (
            "app/consumer.ts",
            br#"import { make, makeArrow, identity, box } from "../lib/factory";
import type { Item } from "../lib/item";
export function use(value: Item) {
    make(value).inspect();
    makeArrow(value).inspect();
    identity(value).inspect();
    box(value).value.inspect();
}
"#
            .as_slice(),
        ),
    ];
    let mut extractions = Vec::new();
    let mut sources = HashMap::new();
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(&path, source)?;
        sources.insert(relative.to_owned(), String::from_utf8(source.to_vec())?);
        let mut extraction = extract(relative, source);
        extraction.semantic_evidence = Some(
            Engine::default().extract_source_universal_candidate_evidence(
                Path::new(relative),
                relative,
                source,
            )?,
        );
        extractions.push(extraction);
    }
    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert!(
        resolved.error.is_none(),
        "resolver error: {:?}",
        resolved.error
    );
    let inspect = resolved
        .nodes
        .iter()
        .find(|node| node.string("source_file") == "lib/item.ts" && node.label() == ".inspect()")
        .ok_or("missing imported callable return member")?;
    let calls = resolved
        .edges
        .iter()
        .filter(|edge| {
            edge.string("relation") == "calls"
                && edge.string("source_file") == "app/consumer.ts"
                && edge.target == inspect.id
        })
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 4);
    assert!(
        calls
            .iter()
            .all(|edge| edge.string("resolution_rule") == "member-binding")
    );
    Ok(())
}

#[test]
fn typescript_candidate_keeps_imported_callable_return_ambiguity_unresolved()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/item.ts",
            br#"export interface Item { inspect(): void }
export interface Item { inspect(): void }
"#
            .as_slice(),
        ),
        (
            "lib/factory.ts",
            br#"import type { Item } from "./item";
export function make(value: Item): Item { return value; }
"#
            .as_slice(),
        ),
        (
            "app/consumer.ts",
            br#"import { make } from "../lib/factory";
import type { Item } from "../lib/item";
export function use(value: Item) { make(value).inspect(); }
"#
            .as_slice(),
        ),
    ];
    let mut extractions = Vec::new();
    let mut sources = HashMap::new();
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(&path, source)?;
        sources.insert(relative.to_owned(), String::from_utf8(source.to_vec())?);
        let mut extraction = extract(relative, source);
        extraction.semantic_evidence = Some(
            Engine::default().extract_source_universal_candidate_evidence(
                Path::new(relative),
                relative,
                source,
            )?,
        );
        extractions.push(extraction);
    }
    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert!(
        resolved.error.is_none(),
        "resolver error: {:?}",
        resolved.error
    );
    assert!(!resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.string("source_file") == "app/consumer.ts"
            && edge.string("resolution_rule") == "member-binding"
    }));
    Ok(())
}

#[test]
fn typescript_candidate_resolves_imported_callable_member_returns_and_explicit_generics()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/item.ts",
            br#"export interface Item { inspect(): void }
"#
            .as_slice(),
        ),
        (
            "lib/factory.ts",
            br#"import type { Item } from "./item";
export class Factory {
    static make(value: Item): Item { return value; }
    static identity<T>(value: T): T { return value; }
}
"#
            .as_slice(),
        ),
        (
            "app/consumer.ts",
            br#"import { Factory } from "../lib/factory";
import type { Item } from "../lib/item";
export function use(value: Item) {
    Factory.make(value).inspect();
    Factory.identity(value).inspect();
    Factory.identity<Item>(value).inspect();
    Factory.identity<Item>(unknownValue).inspect();
}
"#
            .as_slice(),
        ),
    ];
    let mut extractions = Vec::new();
    let mut sources = HashMap::new();
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(&path, source)?;
        sources.insert(relative.to_owned(), String::from_utf8(source.to_vec())?);
        let mut extraction = extract(relative, source);
        extraction.semantic_evidence = Some(
            Engine::default().extract_source_universal_candidate_evidence(
                Path::new(relative),
                relative,
                source,
            )?,
        );
        extractions.push(extraction);
    }
    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert!(
        resolved.error.is_none(),
        "resolver error: {:?}",
        resolved.error
    );
    let inspect = resolved
        .nodes
        .iter()
        .find(|node| node.string("source_file") == "lib/item.ts" && node.label() == ".inspect()")
        .ok_or("missing imported callable member return")?;
    let calls = resolved
        .edges
        .iter()
        .filter(|edge| {
            edge.string("relation") == "calls"
                && edge.string("source_file") == "app/consumer.ts"
                && edge.target == inspect.id
        })
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 4);
    assert!(
        calls
            .iter()
            .all(|edge| edge.string("resolution_rule") == "member-binding")
    );
    Ok(())
}

#[test]
fn typescript_candidate_keeps_ambiguous_imported_callable_member_returns_unresolved()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/item.ts",
            br#"export interface Item { inspect(): void }
"#
            .as_slice(),
        ),
        (
            "lib/factory.ts",
            br#"import type { Item } from "./item";
export class Factory {
    static make(value: Item): Item { return value; }
}
export class Factory {
    static make(value: Item): Item { return value; }
}
"#
            .as_slice(),
        ),
        (
            "app/consumer.ts",
            br#"import { Factory } from "../lib/factory";
import type { Item } from "../lib/item";
export function use(value: Item) { Factory.make(value).inspect(); }
"#
            .as_slice(),
        ),
    ];
    let mut extractions = Vec::new();
    let mut sources = HashMap::new();
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(&path, source)?;
        sources.insert(relative.to_owned(), String::from_utf8(source.to_vec())?);
        let mut extraction = extract(relative, source);
        extraction.semantic_evidence = Some(
            Engine::default().extract_source_universal_candidate_evidence(
                Path::new(relative),
                relative,
                source,
            )?,
        );
        extractions.push(extraction);
    }
    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert!(
        resolved.error.is_none(),
        "resolver error: {:?}",
        resolved.error
    );
    assert!(!resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.string("source_file") == "app/consumer.ts"
            && edge.string("resolution_rule") == "member-binding"
    }));
    Ok(())
}

#[test]
fn typescript_candidate_resolves_imported_callable_properties_and_typed_objects()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/item.ts",
            br#"export interface Item { inspect(): void }
"#
            .as_slice(),
        ),
        (
            "lib/api.ts",
            br#"import type { Item } from "./item";
interface TypedApi { make: (value: Item) => Item }
export declare const typed: TypedApi;
export const api = {
    make: (value: Item): Item => value,
    identity: <T>(value: T): T => value,
};
"#
            .as_slice(),
        ),
        (
            "app/consumer.ts",
            br#"import { api, typed } from "../lib/api";
import type { Item } from "../lib/item";
export function use(value: Item) {
    api.make(value).inspect();
    api.identity<Item>(value).inspect();
    typed.make(value).inspect();
}
"#
            .as_slice(),
        ),
    ];
    let mut extractions = Vec::new();
    let mut sources = HashMap::new();
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(&path, source)?;
        sources.insert(relative.to_owned(), String::from_utf8(source.to_vec())?);
        let mut extraction = extract(relative, source);
        extraction.semantic_evidence = Some(
            Engine::default().extract_source_universal_candidate_evidence(
                Path::new(relative),
                relative,
                source,
            )?,
        );
        extractions.push(extraction);
    }
    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert!(
        resolved.error.is_none(),
        "resolver error: {:?}",
        resolved.error
    );
    let inspect = resolved
        .nodes
        .iter()
        .find(|node| node.string("source_file") == "lib/item.ts" && node.label() == ".inspect()")
        .ok_or("missing imported callable property return")?;
    let calls = resolved
        .edges
        .iter()
        .filter(|edge| {
            edge.string("relation") == "calls"
                && edge.string("source_file") == "app/consumer.ts"
                && edge.target == inspect.id
        })
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 3);
    assert!(
        calls
            .iter()
            .all(|edge| edge.string("resolution_rule") == "member-binding")
    );
    let consumer_calls = resolved
        .edges
        .iter()
        .filter(|edge| {
            edge.string("relation") == "calls" && edge.string("source_file") == "app/consumer.ts"
        })
        .collect::<Vec<_>>();
    assert_eq!(consumer_calls.len(), 6);
    assert!(
        consumer_calls
            .iter()
            .all(|edge| edge.string("resolution_rule") == "member-binding")
    );
    Ok(())
}

#[test]
fn typescript_candidate_keeps_duplicate_imported_callable_properties_unresolved()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/item.ts",
            br#"export interface Item { inspect(): void }
"#
            .as_slice(),
        ),
        (
            "lib/api.ts",
            br#"import type { Item } from "./item";
export const api = {
    make: (value: Item): Item => value,
    make: (value: Item): Item => value,
};
"#
            .as_slice(),
        ),
        (
            "app/consumer.ts",
            br#"import { api } from "../lib/api";
import type { Item } from "../lib/item";
export function use(value: Item) { api.make(value).inspect(); }
"#
            .as_slice(),
        ),
    ];
    let mut extractions = Vec::new();
    let mut sources = HashMap::new();
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(&path, source)?;
        sources.insert(relative.to_owned(), String::from_utf8(source.to_vec())?);
        let mut extraction = extract(relative, source);
        extraction.semantic_evidence = Some(
            Engine::default().extract_source_universal_candidate_evidence(
                Path::new(relative),
                relative,
                source,
            )?,
        );
        extractions.push(extraction);
    }
    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert!(
        resolved.error.is_none(),
        "resolver error: {:?}",
        resolved.error
    );
    assert!(!resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.string("source_file") == "app/consumer.ts"
            && edge.string("resolution_rule") == "member-binding"
    }));
    Ok(())
}

#[test]
fn typescript_candidate_selects_unique_imported_overload_by_argument_type()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/item.ts",
            br#"export interface Item { inspect(): void }
"#
            .as_slice(),
        ),
        (
            "lib/other.ts",
            br#"export interface Other { inspect(): void }
"#
            .as_slice(),
        ),
        (
            "lib/factory.ts",
            br#"import type { Item } from "./item";
import type { Other } from "./other";
export function make(value: Item): Item { return value; }
export function make(value: Other): Other { return value; }
export class Factory {
    static create(value: Item): Item { return value; }
    static create(value: Other): Other { return value; }
}
"#
            .as_slice(),
        ),
        (
            "app/consumer.ts",
            br#"import { Factory, make } from "../lib/factory";
import type { Item } from "../lib/item";
export function use(value: Item) {
    make(value).inspect();
    Factory.create(value).inspect();
    const current = make(value);
    const alias = current;
    alias.inspect();
}
"#
            .as_slice(),
        ),
    ];
    let mut extractions = Vec::new();
    let mut sources = HashMap::new();
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(&path, source)?;
        sources.insert(relative.to_owned(), String::from_utf8(source.to_vec())?);
        let mut extraction = extract(relative, source);
        extraction.semantic_evidence = Some(
            Engine::default().extract_source_universal_candidate_evidence(
                Path::new(relative),
                relative,
                source,
            )?,
        );
        extractions.push(extraction);
    }
    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert!(
        resolved.error.is_none(),
        "resolver error: {:?}",
        resolved.error
    );
    let inspect = resolved
        .nodes
        .iter()
        .find(|node| node.string("source_file") == "lib/item.ts" && node.label() == ".inspect()")
        .ok_or("missing selected overload return member")?;
    let other_inspect = resolved
        .nodes
        .iter()
        .find(|node| node.string("source_file") == "lib/other.ts" && node.label() == ".inspect()")
        .ok_or("missing alternate overload return member")?;
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.string("source_file") == "app/consumer.ts"
            && edge.target == inspect.id
            && edge.string("resolution_rule") == "member-binding"
    }));
    assert_eq!(
        resolved
            .edges
            .iter()
            .filter(|edge| {
                edge.string("relation") == "calls"
                    && edge.string("source_file") == "app/consumer.ts"
                    && edge.target == inspect.id
                    && edge.string("resolution_rule") == "member-binding"
            })
            .count(),
        3
    );
    assert!(!resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.string("source_file") == "app/consumer.ts"
            && edge.target == other_inspect.id
    }));
    assert!(!resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.string("source_file") == "app/consumer.ts"
            && edge.string("resolution_rule") == "deferred-receiver"
    }));
    Ok(())
}

#[test]
fn typescript_candidate_keeps_imported_overload_ambiguity_and_mismatch_unresolved()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/item.ts",
            br#"export interface Item { inspect(): void }
"#
            .as_slice(),
        ),
        (
            "lib/other.ts",
            br#"export interface Other { inspect(): void }
export interface Third { inspect(): void }
"#
            .as_slice(),
        ),
        (
            "lib/factory.ts",
            br#"import type { Item } from "./item";
import type { Other, Third } from "./other";
export function make(value: Item): Item { return value; }
export function make(value: Item): Item { return value; }
export function other(value: Other): Other { return value; }
export function other(value: Third): Third { return value; }
"#
            .as_slice(),
        ),
        (
            "app/consumer.ts",
            br#"import { make, other } from "../lib/factory";
import type { Item } from "../lib/item";
export function use(value: Item) {
    make(value).inspect();
    other(value).inspect();
    make(unknownValue).inspect();
}
"#
            .as_slice(),
        ),
    ];
    let mut extractions = Vec::new();
    let mut sources = HashMap::new();
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(&path, source)?;
        sources.insert(relative.to_owned(), String::from_utf8(source.to_vec())?);
        let mut extraction = extract(relative, source);
        extraction.semantic_evidence = Some(
            Engine::default().extract_source_universal_candidate_evidence(
                Path::new(relative),
                relative,
                source,
            )?,
        );
        extractions.push(extraction);
    }
    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert!(
        resolved.error.is_none(),
        "resolver error: {:?}",
        resolved.error
    );
    assert!(!resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.string("source_file") == "app/consumer.ts"
            && edge.string("resolution_rule") == "member-binding"
    }));
    Ok(())
}

#[test]
fn typescript_candidate_resolves_imported_index_signature_member_chain()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/item.ts",
            br#"export interface Item { inspect(): void }
"#
            .as_slice(),
        ),
        (
            "lib/types.ts",
            br#"import type { Item } from "./item";
export interface Shape { [key: string]: Item }
"#
            .as_slice(),
        ),
        (
            "app/consumer.ts",
            br#"import type { Shape } from "../lib/types";
export function use(shape: Shape, key: string) { shape[key].inspect(); }
"#
            .as_slice(),
        ),
    ];
    let mut extractions = Vec::new();
    let mut sources = HashMap::new();
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(&path, source)?;
        sources.insert(relative.to_owned(), String::from_utf8(source.to_vec())?);
        let mut extraction = extract(relative, source);
        extraction.semantic_evidence = Some(
            Engine::default().extract_source_universal_candidate_evidence(
                Path::new(relative),
                relative,
                source,
            )?,
        );
        extractions.push(extraction);
    }
    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert!(
        resolved.error.is_none(),
        "resolver error: {:?}",
        resolved.error
    );
    let inspect = resolved
        .nodes
        .iter()
        .find(|node| node.string("source_file") == "lib/item.ts" && node.label() == ".inspect()")
        .ok_or("missing imported index member")?;
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.string("source_file") == "app/consumer.ts"
            && edge.target == inspect.id
            && edge.string("resolution_rule") == "member-binding"
    }));
    Ok(())
}

#[test]
fn typescript_candidate_resolves_imported_structural_index_signature_alias_member_chain()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/item.ts",
            br#"export interface Item { inspect(): void }
"#
            .as_slice(),
        ),
        (
            "lib/types.ts",
            br#"import type { Item } from "./item";
export type Shape = { [key: string]: Item };
"#
            .as_slice(),
        ),
        (
            "app/consumer.ts",
            br#"import type { Shape } from "../lib/types";
export function use(shape: Shape, key: string) { shape[key].inspect(); }
"#
            .as_slice(),
        ),
    ];
    let mut extractions = Vec::new();
    let mut sources = HashMap::new();
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(&path, source)?;
        sources.insert(relative.to_owned(), String::from_utf8(source.to_vec())?);
        let mut extraction = extract(relative, source);
        extraction.semantic_evidence = Some(
            Engine::default().extract_source_universal_candidate_evidence(
                Path::new(relative),
                relative,
                source,
            )?,
        );
        extractions.push(extraction);
    }
    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert!(
        resolved.error.is_none(),
        "resolver error: {:?}",
        resolved.error
    );
    let inspect = resolved
        .nodes
        .iter()
        .find(|node| node.string("source_file") == "lib/item.ts" && node.label() == ".inspect()")
        .ok_or("missing imported structural index member")?;
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.string("source_file") == "app/consumer.ts"
            && edge.target == inspect.id
            && edge.string("resolution_rule") == "member-binding"
    }));
    Ok(())
}

#[test]
fn typescript_candidate_resolves_inline_structural_index_signature_member_chain()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/item.ts",
            br#"export interface Item { inspect(): void }
"#
            .as_slice(),
        ),
        (
            "app/consumer.ts",
            br#"import type { Item } from "../lib/item";
export function use(shape: { [key: string]: Item }, key: string) {
    shape[key].inspect();
    const local: { [key: string]: Item } = shape;
    local[key].inspect();
}
export class Holder {
    values: { [key: string]: Item };
    use(key: string) { this.values[key].inspect(); }
}
export function rejected(shape: { [key: string]: string }, key: string) { shape[key].inspect(); }
export function rejectedMapped<T extends string>(shape: { [key in T]: Item }, key: T) { shape[key].inspect(); }
export function rejectedAmbiguous(shape: { [key: string]: Item | string }, key: string) { shape[key].inspect(); }
"#
            .as_slice(),
        ),
    ];
    let mut extractions = Vec::new();
    let mut sources = HashMap::new();
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(&path, source)?;
        sources.insert(relative.to_owned(), String::from_utf8(source.to_vec())?);
        let mut extraction = extract(relative, source);
        extraction.semantic_evidence = Some(
            Engine::default().extract_source_universal_candidate_evidence(
                Path::new(relative),
                relative,
                source,
            )?,
        );
        extractions.push(extraction);
    }
    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert!(
        resolved.error.is_none(),
        "resolver error: {:?}",
        resolved.error
    );
    let inspect = resolved
        .nodes
        .iter()
        .find(|node| node.string("source_file") == "lib/item.ts" && node.label() == ".inspect()")
        .ok_or("missing imported inline structural index member")?;
    let matching_edges = resolved.edges.iter().filter(|edge| {
        edge.string("relation") == "calls"
            && edge.string("source_file") == "app/consumer.ts"
            && edge.target == inspect.id
            && edge.string("resolution_rule") == "member-binding"
    });
    assert_eq!(matching_edges.count(), 3);
    Ok(())
}

#[test]
fn typescript_candidate_resolves_imported_generic_index_signature_member_chain()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/item.ts",
            br#"export interface Item { inspect(): void }
"#
            .as_slice(),
        ),
        (
            "lib/types.ts",
            br#"export interface Shape<T> { [key: string]: T }
"#
            .as_slice(),
        ),
        (
            "app/consumer.ts",
            br#"import type { Shape } from "../lib/types";
import type { Item } from "../lib/item";
export function use(shape: Shape<Item>, key: string) { shape[key].inspect(); }
"#
            .as_slice(),
        ),
    ];
    let mut extractions = Vec::new();
    let mut sources = HashMap::new();
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(&path, source)?;
        sources.insert(relative.to_owned(), String::from_utf8(source.to_vec())?);
        let mut extraction = extract(relative, source);
        extraction.semantic_evidence = Some(
            Engine::default().extract_source_universal_candidate_evidence(
                Path::new(relative),
                relative,
                source,
            )?,
        );
        extractions.push(extraction);
    }
    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert!(
        resolved.error.is_none(),
        "resolver error: {:?}",
        resolved.error
    );
    let inspect = resolved
        .nodes
        .iter()
        .find(|node| node.string("source_file") == "lib/item.ts" && node.label() == ".inspect()")
        .ok_or("missing imported generic index member")?;
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.string("source_file") == "app/consumer.ts"
            && edge.target == inspect.id
            && edge.string("resolution_rule") == "member-binding"
    }));
    Ok(())
}

#[test]
fn typescript_candidate_keeps_imported_index_signature_ambiguity_unresolved()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/item.ts",
            br#"export interface Item { inspect(): void }
export interface Item { inspect(): void }
"#
            .as_slice(),
        ),
        (
            "lib/types.ts",
            br#"import type { Item } from "./item";
export interface Shape { [key: string]: Item }
"#
            .as_slice(),
        ),
        (
            "app/consumer.ts",
            br#"import type { Shape } from "../lib/types";
export function use(shape: Shape, key: string) { shape[key].inspect(); }
"#
            .as_slice(),
        ),
    ];
    let mut extractions = Vec::new();
    let mut sources = HashMap::new();
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(&path, source)?;
        sources.insert(relative.to_owned(), String::from_utf8(source.to_vec())?);
        let mut extraction = extract(relative, source);
        extraction.semantic_evidence = Some(
            Engine::default().extract_source_universal_candidate_evidence(
                Path::new(relative),
                relative,
                source,
            )?,
        );
        extractions.push(extraction);
    }
    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert!(
        resolved.error.is_none(),
        "resolver error: {:?}",
        resolved.error
    );
    assert!(!resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.string("source_file") == "app/consumer.ts"
            && edge.string("resolution_rule") == "member-binding"
    }));
    Ok(())
}

#[test]
fn typescript_candidate_does_not_invent_imported_index_signature_primitive_members()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let files = [
        (
            "lib/types.ts",
            br#"export interface Shape { [key: string]: string }
"#
            .as_slice(),
        ),
        (
            "app/consumer.ts",
            br#"import type { Shape } from "../lib/types";
export function use(shape: Shape, key: string) { shape[key].inspect(); }
"#
            .as_slice(),
        ),
    ];
    let mut extractions = Vec::new();
    let mut sources = HashMap::new();
    for (relative, source) in files {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
        fs::write(&path, source)?;
        sources.insert(relative.to_owned(), String::from_utf8(source.to_vec())?);
        let mut extraction = extract(relative, source);
        extraction.semantic_evidence = Some(
            Engine::default().extract_source_universal_candidate_evidence(
                Path::new(relative),
                relative,
                source,
            )?,
        );
        extractions.push(extraction);
    }
    let resolved = compass_resolve::resolve_with_root(&extractions, &sources, root);
    assert!(
        resolved.error.is_none(),
        "resolver error: {:?}",
        resolved.error
    );
    assert!(!resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.string("source_file") == "app/consumer.ts"
            && edge.string("resolution_rule") == "member-binding"
    }));
    Ok(())
}

#[test]
fn typescript_candidate_resolves_straight_line_reassignment_to_latest_member()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let relative = "src/reassignment.ts";
    let source = br#"class First { run() {} }
class Second { run() {} }
export function use() {
    let current = new First();
    current = new Second();
    current.run();
}
"#;
    let path = root.join(relative);
    fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
    fs::write(&path, source)?;
    let mut sources = HashMap::new();
    sources.insert(relative.to_owned(), String::from_utf8(source.to_vec())?);
    let mut extraction = extract(relative, source);
    extraction.semantic_evidence = Some(
        Engine::default().extract_source_universal_candidate_evidence(
            Path::new(relative),
            relative,
            source,
        )?,
    );
    let resolved = compass_resolve::resolve_with_root(&[extraction], &sources, root);
    assert!(
        resolved.error.is_none(),
        "resolver error: {:?}",
        resolved.error
    );
    let first_run = resolved
        .nodes
        .iter()
        .find(|node| {
            node.string("source_file") == relative
                && node.label() == ".run()"
                && node.string("qualified_name").ends_with("First.run")
        })
        .ok_or("missing First.run member")?;
    let second_run = resolved
        .nodes
        .iter()
        .find(|node| {
            node.string("source_file") == relative
                && node.label() == ".run()"
                && node.string("qualified_name").ends_with("Second.run")
        })
        .ok_or("missing Second.run member")?;
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.string("source_file") == relative
            && edge.target == second_run.id
            && edge.string("resolution_rule") == "exact-source-declaration"
    }));
    assert!(!resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.string("source_file") == relative
            && edge.target == first_run.id
            && edge.string("resolution_rule") == "exact-source-declaration"
    }));
    Ok(())
}

#[test]
fn typescript_callable_values_materialize_as_references_not_indirect_calls()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let relative = "src/callback-values.ts";
    let source = br#"function onValue(value: string) {}
const alias = onValue;
const alias2 = alias;
const handlers = [onValue, alias2];
consume(onValue);
consume(alias2);
consume(handlers[0]);
"#;
    let path = root.join(relative);
    fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
    fs::write(&path, source)?;
    let mut sources = HashMap::new();
    sources.insert(relative.to_owned(), String::from_utf8(source.to_vec())?);
    let mut extraction = extract(relative, source);
    extraction.semantic_evidence = Some(
        Engine::default().extract_source_universal_candidate_evidence(
            Path::new(relative),
            relative,
            source,
        )?,
    );
    let resolved = compass_resolve::resolve_with_root(&[extraction], &sources, root);
    assert!(
        resolved.error.is_none(),
        "resolver error: {:?}",
        resolved.error
    );
    let on_value = resolved
        .nodes
        .iter()
        .find(|node| {
            node.string("source_file") == relative
                && node.string("qualified_name").ends_with(".onValue")
                && node.string("symbol_kind") == "function"
        })
        .ok_or("missing callable declaration")?;
    let alias2 = resolved
        .nodes
        .iter()
        .find(|node| {
            node.string("source_file") == relative
                && node.string("qualified_name").ends_with(".alias2")
                && node.string("symbol_kind") == "variable"
        })
        .ok_or("missing callable alias declaration")?;
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "references"
            && edge.string("source_file") == relative
            && edge.target == on_value.id
            && edge.string("resolution_rule") == "exact-source-declaration"
    }));
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "references"
            && edge.string("source_file") == relative
            && edge.target == alias2.id
            && edge.string("resolution_rule") == "exact-source-declaration"
    }));
    assert!(!resolved.edges.iter().any(|edge| {
        edge.string("relation") == "indirect_call" && edge.string("source_file") == relative
    }));
    Ok(())
}
