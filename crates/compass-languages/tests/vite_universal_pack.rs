use std::fs;
use std::sync::Arc;

use compass_languages::{Engine, ProjectEvidenceIndex, RawFrameworkFact, RawFrameworkOrigin};
use tempfile::tempdir;

#[test]
fn vite_config_uses_ast_literals_and_preserves_ordered_aliases() {
    let directory = tempdir().expect("temporary project");
    let root = directory.path();
    let config_path = root.join("vite.config.ts");
    let source = br#"import { defineConfig as makeConfig } from 'vite';
import react from '@vitejs/plugin-react';
import unrelated from 'my-plugin';
export default makeConfig(({ mode }) => ({
  resolve: { alias: [
    { find: '@app', replacement: './src' },
    { find: /^~(.+)/, replacement: './vendor' },
  ] },
  plugins: [react()],
}));
"#;
    fs::write(
        root.join("package.json"),
        br#"{"dependencies":{"vite":"7.0.0","@vitejs/plugin-react":"5.0.0"}}"#,
    )
    .expect("package manifest");
    fs::write(&config_path, source).expect("vite config");
    let project = ProjectEvidenceIndex::build(root, std::slice::from_ref(&config_path));
    let mut engine = Engine::with_project_evidence(Arc::new(project));
    let extraction = engine
        .extract_source_graph_only(&config_path, "vite.config.ts", source)
        .expect("vite extraction");
    let domain = extraction
        .framework_facts
        .iter()
        .find_map(|fact| match fact {
            RawFrameworkFact::Domain(domain) if domain.framework == "vite" => Some(domain),
            _ => None,
        })
        .expect("vite configuration fact");
    assert_eq!(domain.origin, RawFrameworkOrigin::Config);
    assert_eq!(
        domain
            .detail
            .get("aliases_ordered")
            .and_then(|value| value.as_array())
            .map(Vec::len),
        Some(2)
    );
    assert_eq!(
        domain
            .detail
            .get("plugins")
            .and_then(|value| value.as_array())
            .into_iter()
            .flatten()
            .filter_map(|value| value.as_str())
            .collect::<Vec<_>>(),
        vec!["@vitejs/plugin-react"]
    );
    assert!(extraction.framework_facts.iter().any(|fact| {
        matches!(fact, RawFrameworkFact::Configuration(configuration)
            if configuration.framework == "vite" && configuration.field == "resolve"
                && configuration.detail.contains_key("aliases_ordered"))
    }));
}

#[test]
fn shadowed_define_config_without_vite_import_is_not_a_vite_fact() {
    let directory = tempdir().expect("temporary project");
    let root = directory.path();
    let config_path = root.join("vite.config.ts");
    let source = br#"const defineConfig = () => ({ resolve: { alias: { '@app': './src' } } });
export default defineConfig();
"#;
    fs::write(
        root.join("package.json"),
        br#"{"dependencies":{"vite":"7.0.0"}}"#,
    )
    .expect("package manifest");
    fs::write(&config_path, source).expect("vite config");
    let project = ProjectEvidenceIndex::build(root, std::slice::from_ref(&config_path));
    let mut engine = Engine::with_project_evidence(Arc::new(project));
    let extraction = engine
        .extract_source_graph_only(&config_path, "vite.config.ts", source)
        .expect("vite extraction");
    assert!(
        !extraction
            .framework_facts
            .iter()
            .any(|fact| matches!(fact, RawFrameworkFact::Configuration(_)))
    );
}

#[test]
fn vite_import_meta_glob_publishes_bounded_file_set_evidence() {
    let directory = tempdir().expect("temporary project");
    let root = directory.path();
    let source_path = root.join("src/content.ts");
    let source = br#"const modules = import.meta.glob(['./posts/*.md', '!./posts/draft.md'], {
  eager: true,
  import: 'default',
  query: '?raw',
});
"#;
    fs::write(
        root.join("package.json"),
        br#"{"dependencies":{"vite":"7.0.0"}}"#,
    )
    .expect("package manifest");
    fs::create_dir_all(source_path.parent().expect("source parent")).expect("source directory");
    fs::write(&source_path, source).expect("source file");
    let project = ProjectEvidenceIndex::build(root, std::slice::from_ref(&source_path));
    let mut engine = Engine::with_project_evidence(Arc::new(project));
    let extraction = engine
        .extract_source_graph_only(&source_path, "src/content.ts", source)
        .expect("source extraction");
    let file_set = extraction
        .framework_facts
        .iter()
        .find_map(|fact| match fact {
            RawFrameworkFact::FileSet(file_set) => Some(file_set),
            _ => None,
        })
        .expect("Vite file-set fact");
    assert_eq!(file_set.patterns, vec!["./posts/*.md"]);
    assert_eq!(file_set.negative_patterns, vec!["./posts/draft.md"]);
    assert!(file_set.eager);
    assert!(!file_set.lazy);
    assert!(file_set.import_mode);
    assert!(file_set.query_mode);
    assert_eq!(
        file_set
            .detail
            .get("complete")
            .and_then(|value| value.as_bool()),
        Some(true)
    );
}

#[test]
fn vite_globs_use_only_one_vite_configs_source_ordered_alias_rules() {
    let directory = tempdir().expect("temporary project");
    let root = directory.path();
    let source_path = root.join("src/content.ts");
    let source = br#"const modules = import.meta.glob('@app/components/*.tsx');"#;
    fs::write(
        root.join("package.json"),
        br#"{"dependencies":{"vite":"7.0.0"},"imports":{"@app":"./wrong-package"}}"#,
    )
    .expect("package manifest");
    fs::write(
        root.join("tsconfig.json"),
        br#"{"compilerOptions":{"paths":{"@app/*":["./wrong-typescript/*"]}}}"#,
    )
    .expect("TypeScript configuration");
    fs::write(
        root.join("vite.config.ts"),
        br#"import { defineConfig } from 'vite';
export default defineConfig({ resolve: { alias: [
  { find: '@app', replacement: './first' },
  { find: '@app/components', replacement: './second' },
  { find: /^~(.+)/, replacement: './vendor' },
] } });"#,
    )
    .expect("Vite configuration");
    fs::create_dir_all(source_path.parent().expect("source parent")).expect("source directory");
    fs::write(&source_path, source).expect("source file");

    let project = ProjectEvidenceIndex::build(root, std::slice::from_ref(&source_path));
    let mut engine = Engine::with_project_evidence(Arc::new(project));
    let extraction = engine
        .extract_source_graph_only(&source_path, "src/content.ts", source)
        .expect("source extraction");
    let detail = extraction
        .framework_facts
        .iter()
        .find_map(|fact| match fact {
            RawFrameworkFact::FileSet(file_set) => Some(&file_set.detail),
            _ => None,
        })
        .expect("Vite file-set detail");
    let aliases = detail
        .get("aliases_ordered")
        .and_then(|value| value.as_array())
        .expect("ordered Vite aliases");
    assert_eq!(aliases.len(), 3);
    assert_eq!(
        aliases[0].get("find").and_then(|value| value.as_str()),
        Some("@app")
    );
    assert_eq!(
        aliases[1].get("find").and_then(|value| value.as_str()),
        Some("@app/components")
    );
    assert_eq!(
        aliases[2].get("kind").and_then(|value| value.as_str()),
        Some("regex")
    );
    assert!(!detail.contains_key("aliases"));
}

#[test]
fn vite_globs_do_not_guess_between_multiple_vite_configurations() {
    let directory = tempdir().expect("temporary project");
    let root = directory.path();
    let source_path = root.join("src/content.ts");
    let source = br#"const modules = import.meta.glob('@app/*.tsx');"#;
    fs::write(
        root.join("package.json"),
        br#"{"dependencies":{"vite":"7.0.0"}}"#,
    )
    .expect("package manifest");
    for (name, replacement) in [
        ("vite.config.js", "./javascript"),
        ("vite.config.ts", "./typescript"),
    ] {
        fs::write(
            root.join(name),
            format!(
                "import {{ defineConfig }} from 'vite'; export default defineConfig({{ resolve: {{ alias: {{ '@app': '{replacement}' }} }} }});"
            ),
        )
        .expect("Vite configuration");
    }
    fs::create_dir_all(source_path.parent().expect("source parent")).expect("source directory");
    fs::write(&source_path, source).expect("source file");

    let project = ProjectEvidenceIndex::build(root, std::slice::from_ref(&source_path));
    let mut engine = Engine::with_project_evidence(Arc::new(project));
    let extraction = engine
        .extract_source_graph_only(&source_path, "src/content.ts", source)
        .expect("source extraction");
    let detail = extraction
        .framework_facts
        .iter()
        .find_map(|fact| match fact {
            RawFrameworkFact::FileSet(file_set) => Some(&file_set.detail),
            _ => None,
        })
        .expect("Vite file-set detail");
    assert!(!detail.contains_key("aliases_ordered"));
}
