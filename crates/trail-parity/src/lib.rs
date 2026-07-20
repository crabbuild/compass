//! Development-only differential verification against the pinned Python baseline.

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::ffi::OsString;
    use std::fs;
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use serde_json::{Value, json};
    use tempfile::TempDir;
    use trail_cli::Frontend;
    use trail_files::{
        Cache, CacheKind, DetectOptions, Manifest, ManifestKind, detect, file_hash,
        prompt_fingerprint,
    };
    use trail_graph::{
        ClusterOptions, build_from_extraction, cluster, community_member_signatures,
        deduplicate_entities, find_import_cycles, god_nodes, graph_diff, label_communities_by_hub,
        score_communities, suggest_questions, surprising_connections,
    };
    use trail_languages::Engine;
    use trail_output::{JsonExportOptions, cypher_document, write_json};
    use trail_resolve::resolve;

    #[test]
    fn read_commands_match_python_cli() -> Result<(), Box<dyn Error>> {
        let fixture = Fixture::new()?;
        for arguments in [
            vec![
                "query",
                "who calls extract",
                "--context",
                "call",
                "--graph",
                fixture.graph_string(),
            ],
            vec![
                "path",
                "createPatchHandler",
                "validateSanitySession",
                "--graph",
                fixture.graph_string(),
            ],
            vec![
                "path",
                "validateSanitySession",
                "createPatchHandler",
                "--graph",
                fixture.graph_string(),
            ],
            vec![
                "explain",
                "validateSanitySession",
                "--graph",
                fixture.graph_string(),
            ],
            vec![
                "affected",
                "validateSanitySession",
                "--relation",
                "calls",
                "--graph",
                fixture.graph_string(),
            ],
        ] {
            compare(&arguments)?;
        }
        Ok(())
    }

    #[test]
    fn deterministic_files_match_python() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        fs::create_dir(root.join("src"))?;
        fs::create_dir_all(root.join("vendor/nested"))?;
        fs::create_dir(root.join("secrets"))?;
        fs::write(root.join("src/main.py"), "def main():\n    return 42\n")?;
        fs::write(
            root.join("src/tool"),
            "#!/usr/bin/env python3\nprint('ok')\n",
        )?;
        fs::write(root.join("README.md"), "# Project\n\nA small project.\n")?;
        fs::write(
            root.join("paper.md"),
            "# Abstract\nWe propose a method in this arXiv preprint. See [1].\\cite{x}\n",
        )?;
        fs::write(root.join("diagram.svg"), "<svg></svg>")?;
        fs::write(root.join("secrets/db.json"), "{\"token\": \"redacted\"}")?;
        fs::write(root.join("vendor/ignored.py"), "x = 1\n")?;
        fs::write(root.join("vendor/nested/keep.py"), "x = 2\n")?;
        fs::write(root.join("vendor/nested/debug.log"), "noise\n")?;
        fs::write(root.join("unknown.bin"), [0_u8, 1, 2])?;
        fs::write(root.join(".graphifyignore"), "vendor/ignored.py\n")?;
        fs::write(root.join("vendor/nested/.gitignore"), "*.log\n")?;

        let rust = serde_json::to_value(detect(root, &DetectOptions::default())?)?;
        let repo = repository_root();
        let output = Command::new(python_executable(&repo))
            .args([
                "-c",
                "import json,sys; from pathlib import Path; from graphify.detect import detect; print(json.dumps(detect(Path(sys.argv[1]))))",
            ])
            .arg(root)
            .current_dir(&repo)
            .env("PYTHONPATH", &repo)
            .output()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let python: serde_json::Value = serde_json::from_slice(&output.stdout)?;
        assert_eq!(rust, python);

        let source = root.join("README.md");
        let rust_hash = file_hash(&source, root)?;
        let output = Command::new(python_executable(&repo))
            .args([
                "-c",
                "import sys; from pathlib import Path; from graphify.cache import file_hash; print(file_hash(Path(sys.argv[1]), Path(sys.argv[2])))",
            ])
            .arg(&source)
            .arg(root)
            .current_dir(&repo)
            .env("PYTHONPATH", &repo)
            .output()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(rust_hash, String::from_utf8(output.stdout)?.trim());
        assert_eq!(prompt_fingerprint("hello  \r\nworld\r\n"), "26c60a61d01d");
        Ok(())
    }

    #[test]
    fn persisted_file_artifacts_cross_read_with_python() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let root = fs::canonicalize(directory.path())?;
        let source = root.join("src.py");
        fs::write(&source, "def trail():\n    return 1\n")?;
        let manifest_path = root.join("graphify-out/manifest.json");
        let source_string = source.to_string_lossy().into_owned();
        let mut buckets = std::collections::BTreeMap::new();
        buckets.insert("code".to_owned(), vec![source_string.clone()]);

        let mut manifest = Manifest::default();
        manifest.save(
            &buckets,
            &manifest_path,
            ManifestKind::Both,
            Some(&root),
            None,
            None,
        )?;

        let repo = repository_root();
        let output = Command::new(python_executable(&repo))
            .args([
                "-c",
                "import json,sys; from pathlib import Path; from graphify.detect import load_manifest; print(json.dumps(load_manifest(sys.argv[1], root=Path(sys.argv[2])), sort_keys=True))",
            ])
            .arg(&manifest_path)
            .arg(&root)
            .current_dir(&repo)
            .env("PYTHONPATH", &repo)
            .output()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let python_manifest: serde_json::Value = serde_json::from_slice(&output.stdout)?;
        let python_entry = python_manifest
            .get(&source_string)
            .ok_or("Python did not load the Rust manifest entry")?;
        let rust_entry = manifest
            .entries()
            .get(&source_string)
            .ok_or("Rust manifest entry missing")?;
        assert_eq!(
            python_entry.get("ast_hash"),
            Some(&json!(rust_entry.ast_hash))
        );
        assert_eq!(
            python_entry.get("semantic_hash"),
            Some(&json!(rust_entry.semantic_hash))
        );

        let cached = json!({
            "nodes": [{"id": "trail", "source_file": source_string}],
            "edges": []
        });
        let mut cache = Cache::new(&root, None)?;
        cache.save(&source, &cached, &CacheKind::Ast, None)?;
        cache.flush()?;
        let output = Command::new(python_executable(&repo))
            .args([
                "-c",
                "import json,sys; from pathlib import Path; from graphify.cache import load_cached; print(json.dumps(load_cached(Path(sys.argv[1]), root=Path(sys.argv[2]), kind='ast'), sort_keys=True))",
            ])
            .arg(&source)
            .arg(&root)
            .current_dir(&repo)
            .env("PYTHONPATH", &repo)
            .output()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let python_cache: serde_json::Value = serde_json::from_slice(&output.stdout)?;
        assert_eq!(python_cache, cached);
        Ok(())
    }

    #[test]
    fn python_ast_extraction_matches_exactly() -> Result<(), Box<dyn Error>> {
        compare_extraction("sample.py", "extract_python")
    }

    #[test]
    fn typescript_ast_extraction_matches_exactly() -> Result<(), Box<dyn Error>> {
        compare_extraction("sample.ts", "extract_js")
    }

    #[test]
    fn java_ast_extraction_matches_exactly() -> Result<(), Box<dyn Error>> {
        compare_extraction("sample.java", "extract_java")
    }

    #[test]
    fn go_ast_extraction_matches_exactly() -> Result<(), Box<dyn Error>> {
        compare_extraction("sample.go", "extract_go")
    }

    #[test]
    fn rust_ast_extraction_matches_exactly() -> Result<(), Box<dyn Error>> {
        compare_extraction("sample.rs", "extract_rust")
    }

    #[test]
    fn c_ast_extraction_matches_exactly() -> Result<(), Box<dyn Error>> {
        compare_extraction("sample.c", "extract_c")
    }

    #[test]
    fn ruby_ast_extraction_matches_exactly() -> Result<(), Box<dyn Error>> {
        compare_extraction("sample.rb", "extract_ruby")
    }

    #[test]
    fn kotlin_ast_extraction_matches_exactly() -> Result<(), Box<dyn Error>> {
        compare_extraction("sample.kt", "extract_kotlin")
    }

    #[test]
    fn scala_ast_extraction_matches_exactly() -> Result<(), Box<dyn Error>> {
        compare_extraction("sample.scala", "extract_scala")
    }

    #[test]
    fn lua_ast_extraction_matches_exactly() -> Result<(), Box<dyn Error>> {
        compare_extraction("sample.luau", "extract_lua")
    }

    #[test]
    fn bash_ast_extraction_matches_exactly() -> Result<(), Box<dyn Error>> {
        compare_extraction("sample.sh", "extract_bash")
    }

    #[test]
    fn markdown_extraction_matches_exactly() -> Result<(), Box<dyn Error>> {
        for fixture in ["sample.md", "deploy_guide.md"] {
            compare_extraction(fixture, "extract_markdown")?;
        }
        Ok(())
    }

    #[test]
    fn json_config_extraction_matches_exactly() -> Result<(), Box<dyn Error>> {
        for fixture in ["sample.json", "sample_tsconfig.json", "extraction.json"] {
            compare_extraction(fixture, "extract_json")?;
        }
        Ok(())
    }

    #[test]
    fn mcp_config_extraction_matches_exactly_without_leaking_values() -> Result<(), Box<dyn Error>>
    {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join(".mcp.json");
        fs::copy(
            repository_root().join("tests/fixtures/sample.mcp.json"),
            &source,
        )?;
        let extraction = compare_extraction_path(&source, "extract_mcp_config")?;
        let serialized = serde_json::to_string(&extraction)?;
        assert!(!serialized.contains("ghp_PLACEHOLDER_NOT_A_REAL_TOKEN"));
        assert!(!serialized.contains("/tmp/workspace"));
        Ok(())
    }

    #[test]
    fn package_manifest_extraction_matches_exactly() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let fixtures = [
            (
                "apm.yml",
                "name: trail-apm\nversion: 1.2.3\ndependencies:\n  - alpha\n  - beta\n",
            ),
            (
                "pyproject.toml",
                "[project]\nname = \"trail-python\"\nversion = \"2.0.0\"\ndependencies = [\"requests>=2\", \"rich[pretty]==13\"]\n",
            ),
            (
                "go.mod",
                "module example.com/trail\n\nrequire (\n example.com/alpha v1.2.3\n example.com/beta v0.4.0 // indirect\n)\n",
            ),
            (
                "pom.xml",
                "<project><groupId>dev.trail</groupId><artifactId>trail-maven</artifactId><version>3.0</version><dependencies><dependency><groupId>org.example</groupId><artifactId>alpha</artifactId></dependency></dependencies></project>",
            ),
        ];
        for (name, contents) in fixtures {
            let source = directory.path().join(name);
            fs::write(&source, contents)?;
            compare_extraction_path(&source, "extract_package_manifest")?;
        }
        compare_extraction_path(
            &repository_root().join("pyproject.toml"),
            "extract_package_manifest",
        )?;
        Ok(())
    }

    #[test]
    fn terraform_extraction_matches_exactly() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("main.tf");
        fs::write(
            &source,
            r#"variable "region" { default = "us-west-2" }
locals { image = data.aws_ami.base.id }
data "aws_ami" "base" { most_recent = true }
resource "aws_instance" "web" {
  ami = local.image
  depends_on = [data.aws_ami.base]
}
output "instance_id" { value = aws_instance.web.id }
"#,
        )?;
        compare_extraction_path(&source, "extract_terraform")?;
        Ok(())
    }

    #[test]
    fn pascal_form_extraction_matches_exactly() -> Result<(), Box<dyn Error>> {
        for (fixture, extractor) in [
            ("sample.dfm", "extract_delphi_form"),
            ("sample.lfm", "extract_lazarus_form"),
            ("sample.lpk", "extract_lazarus_package"),
        ] {
            compare_extraction(fixture, extractor)?;
        }
        Ok(())
    }

    #[test]
    fn dreammaker_asset_extraction_matches_exactly() -> Result<(), Box<dyn Error>> {
        for (fixture, extractor) in [
            ("sample.dmi", "extract_dmi"),
            ("sample.dmm", "extract_dmm"),
            ("sample.dmf", "extract_dmf"),
        ] {
            compare_extraction(fixture, extractor)?;
        }
        Ok(())
    }

    #[test]
    fn dreammaker_source_extraction_matches_exactly() -> Result<(), Box<dyn Error>> {
        compare_extraction("sample.dm", "extract_dm")
    }

    #[test]
    fn dotnet_project_extraction_matches_exactly() -> Result<(), Box<dyn Error>> {
        for (fixture, extractor) in [
            ("sample.sln", "extract_sln"),
            ("sample.slnx", "extract_slnx"),
            ("sample.csproj", "extract_csproj"),
        ] {
            compare_extraction(fixture, extractor)?;
        }
        Ok(())
    }

    #[test]
    fn csharp_ast_extraction_matches_exactly() -> Result<(), Box<dyn Error>> {
        for fixture in ["sample.cs", "sample.xaml.cs"] {
            compare_extraction(fixture, "extract_csharp")?;
        }
        Ok(())
    }

    #[test]
    fn xaml_extraction_matches_exactly() -> Result<(), Box<dyn Error>> {
        for fixture in ["sample.xaml", "bindings.xaml"] {
            compare_extraction(fixture, "extract_xaml")?;
        }
        Ok(())
    }

    #[test]
    fn cpp_ast_extraction_matches_exactly() -> Result<(), Box<dyn Error>> {
        for fixture in ["sample.cpp", "sample.cu", "sample.metal"] {
            compare_extraction(fixture, "extract_cpp")?;
        }
        let repo = repository_root();
        for fixture in [
            "cpp_paired/Foo.cpp",
            "cpp_paired/Foo.h",
            "cpp_paired/Main.cpp",
            "cpp_samedir/Alpha.h",
            "cpp_samedir/Beta.h",
            "cpp_logger/a/Logger.cpp",
            "cpp_logger/a/Logger.h",
            "cpp_logger/b/Logger.cpp",
            "cpp_logger/b/Logger.h",
        ] {
            compare_extraction_path(&repo.join("tests/fixtures").join(fixture), "extract_cpp")?;
        }
        Ok(())
    }

    #[test]
    fn php_ast_extraction_matches_exactly() -> Result<(), Box<dyn Error>> {
        for fixture in [
            "sample.php",
            "sample_php_config.php",
            "sample_php_container.php",
            "sample_php_listen.php",
            "sample_php_static_prop.php",
        ] {
            compare_extraction(fixture, "extract_php")?;
        }
        Ok(())
    }

    #[test]
    fn groovy_ast_extraction_matches_exactly() -> Result<(), Box<dyn Error>> {
        for fixture in ["sample.groovy", "sample_spock.groovy"] {
            compare_extraction(fixture, "extract_groovy")?;
        }
        Ok(())
    }

    #[test]
    fn swift_ast_extraction_matches_exactly() -> Result<(), Box<dyn Error>> {
        compare_extraction("sample.swift", "extract_swift")?;
        let repo = repository_root();
        for fixture in [
            "swift_cross_file/Foo.swift",
            "swift_cross_file/Foo+Ext.swift",
        ] {
            compare_extraction_path(&repo.join("tests/fixtures").join(fixture), "extract_swift")?;
        }
        Ok(())
    }

    #[test]
    fn objc_ast_extraction_matches_exactly() -> Result<(), Box<dyn Error>> {
        compare_extraction("sample.m", "extract_objc")?;
        let repo = repository_root();
        for fixture in [
            "objc_mixed/Bridging-Header.h",
            "objc_mixed/Widget.h",
            "objc_mixed/Widget.m",
        ] {
            compare_extraction_path(&repo.join("tests/fixtures").join(fixture), "extract_objc")?;
        }
        Ok(())
    }

    #[test]
    fn powershell_extraction_matches_exactly() -> Result<(), Box<dyn Error>> {
        for fixture in ["sample.ps1", "sample_import.ps1"] {
            compare_extraction(fixture, "extract_powershell")?;
        }
        compare_extraction("sample.psd1", "extract_powershell_manifest")?;
        Ok(())
    }

    #[test]
    fn elixir_extraction_matches_exactly() -> Result<(), Box<dyn Error>> {
        compare_extraction("sample.ex", "extract_elixir")
    }

    #[test]
    fn julia_extraction_matches_exactly() -> Result<(), Box<dyn Error>> {
        compare_extraction("sample.jl", "extract_julia")
    }

    #[test]
    fn fortran_extraction_matches_exactly() -> Result<(), Box<dyn Error>> {
        for fixture in ["sample.f90", "sample_preprocessed.F90"] {
            compare_extraction(fixture, "extract_fortran")?;
        }
        Ok(())
    }

    #[test]
    fn zig_extraction_matches_exactly() -> Result<(), Box<dyn Error>> {
        compare_extraction("sample.zig", "extract_zig")
    }

    #[test]
    fn verilog_extraction_matches_exactly() -> Result<(), Box<dyn Error>> {
        compare_extraction("sample.sv", "extract_verilog")
    }

    #[test]
    fn sql_extraction_matches_exactly() -> Result<(), Box<dyn Error>> {
        for fixture in [
            "sample.sql",
            "sample_alter_fk.sql",
            "sample_plpgsql.sql",
            "sample_schema_qualified.sql",
        ] {
            compare_extraction(fixture, "extract_sql")?;
        }
        Ok(())
    }

    #[test]
    fn pascal_source_extraction_matches_exactly() -> Result<(), Box<dyn Error>> {
        compare_extraction("sample.pas", "extract_pascal")?;
        compare_extraction("sample_scoped_calls.pas", "extract_pascal")?;
        let repo = repository_root();
        for fixture in [
            "pascal_cross_file/BaseGadget.pas",
            "pascal_cross_file/DerivedGadget.pas",
            "pascal_cross_file/OtherGadget.pas",
        ] {
            compare_extraction_path(&repo.join("tests/fixtures").join(fixture), "extract_pascal")?;
        }
        Ok(())
    }

    #[test]
    fn apex_extraction_matches_exactly() -> Result<(), Box<dyn Error>> {
        for fixture in ["sample.cls", "sample.trigger"] {
            compare_extraction(fixture, "extract_apex")?;
        }
        Ok(())
    }

    #[test]
    fn dart_extraction_matches_exactly() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("test_app_bloc.dart");
        fs::write(
            &source,
            r#"
import 'package:flutter/material.dart';
import 'package:flutter_bloc/flutter_bloc.dart';
export 'package:flutter_bloc/flutter_bloc.dart';

@injectable
@HiveType(typeId: 10)
class UserBloc extends Bloc<UserEvent, UserState> with MyMixin implements Disposable {
  UserBloc() : super(InitialState()) {
    on<AuthLogin>((event, emit) { emit(AuthLoading()); });
  }
}

@jsonSerializable
enum UserRole { admin, user }

extension StringExtensions on String {
  bool get isEmail => contains('@');
}

final authServiceProvider = Provider<AuthService>((ref) => AuthService());
final myData = 42;

void checkDependencies(BuildContext context) {
  final custom = context.dependOnInheritedWidgetOfExactType<CustomService>();
  final auth = context.read<AuthService>();
  final bloc = BlocProvider.of<UserBloc>(context);
  final getItService = GetIt.I<DatabaseService>();
  final locatorService = locator<api.NetworkFactory>();
  context.read<AuthBloc>().add(AuthLogin());
  context.go('/home?id=123&type=auth');
  Navigator.pushNamed(context, Routes.login);
  context.router.push(ProfileRoute());
}
"#,
        )?;
        compare_extraction_path(&source, "extract_dart")?;

        let namespaces = directory.path().join("test_namespaces.dart");
        fs::write(
            &namespaces,
            r#"
class MyWidget extends foo.Bar<Map<String, int>> implements ui.Widget, db.Model {}
final Map<String, int> myVar = 10;
const List<Map<String, int>> myList = [];
late final auth.AuthService authService;
Map<String, Map<String, int>> myMethod(String a) {}
auth.AuthService init() {}
"#,
        )?;
        compare_extraction_path(&namespaces, "extract_dart")?;

        let advanced = directory.path().join("test_advanced.dart");
        fs::write(
            &advanced,
            r#"
import 'package:riverpod/riverpod.dart';
abstract base class MyBaseClass {}
abstract interface class MyInterface {}
mixin class MyMixinClass {}
@riverpod
class MyNotifier extends _$MyNotifier {
  @override
  String build() { ref.watch(anotherProvider); return "hello"; }
}
@riverpod
String myValue(MyValueRef ref) { return "world"; }
class MyModel {
  late final String lateField;
  final int noInitField;
  final String initField = "init";
}
final (int, String) typedRecord = (1, "one");
var (recA, recB) = (10, 20);
(double, double) getCoordinates() {
  var localVal = switch (typedRecord) { _ => (0.0, 0.0) };
  return localVal;
}
"#,
        )?;
        compare_extraction_path(&advanced, "extract_dart")?;

        let specifics = directory.path().join("test_specifics.dart");
        fs::write(
            &specifics,
            r#"
mixin AuthMixin on BaseWidget {}
typedef JsonMap = ApiJsonMap;
extension type UserId(int value) implements Object {}
class MyService {
  final AuthService api;
  MyService(this.api);
  factory MyService.fromJson() {}
  void navigate(BuildContext context) {
    context.go('/home');
    Navigator.pushNamed(context, Routes.login);
    context.router.push(ProfileRoute());
  }
}
"#,
        )?;
        compare_extraction_path(&specifics, "extract_dart")?;

        let parent = directory.path().join("parent_lib.dart");
        fs::write(&parent, "library parent_lib;\npart 'child_part.dart';\n")?;
        let child = directory.path().join("child_part.dart");
        fs::write(
            &child,
            r#"
part of 'parent_lib.dart';
class ChildClass extends Bloc<Pair<UserEvent, MyState>, State> {}
var User(name: myVar, age: myAge) = user;
void runDI(BuildContext context) {
  final repo = locator<Repository<User>>();
  context.go('/home?id=123&type=auth');
}
"#,
        )?;
        compare_extraction_path(&child, "extract_dart")?;
        Ok(())
    }

    #[test]
    fn razor_and_blade_extraction_match_exactly() -> Result<(), Box<dyn Error>> {
        compare_extraction("sample.razor", "extract_razor")?;

        let directory = tempfile::tempdir()?;
        let source = directory.path().join("dashboard.blade.php");
        fs::write(
            &source,
            r#"
@include('layouts.header')
@include("shared.alert")
<livewire:user.profile />
<livewire:admin-panel>
<button wire:click="save">Save</button>
<button wire:click='delete(42)'>Delete</button>
"#,
        )?;
        compare_extraction_path(&source, "extract_blade")?;
        Ok(())
    }

    #[test]
    fn web_template_extraction_matches_exactly() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        fs::create_dir_all(directory.path().join("components"))?;
        fs::write(
            directory.path().join("helper.ts"),
            "export function helper() {}\n",
        )?;
        fs::write(
            directory.path().join("Lazy.vue"),
            "<template><div/></template>\n",
        )?;
        let vue = directory.path().join("Host.vue");
        fs::write(
            &vue,
            r#"<template><Lazy /></template>
<script setup lang="ts" generic="T extends Record<string, unknown>">
import { helper } from './helper'
const count = 1
function onClick(): void { helper() }
const Lazy = defineAsyncComponent(() => import('./Lazy.vue'))
</script>
"#,
        )?;
        compare_extraction_path(&vue, "extract_vue")?;

        fs::write(directory.path().join("Card.svelte"), "<div>card</div>\n")?;
        fs::write(
            directory.path().join("Heavy.svelte.ts"),
            "export const heavy = true\n",
        )?;
        let svelte = directory.path().join("Page.svelte");
        fs::write(
            &svelte,
            r#"<script lang="ts">
import Card from './Card.svelte'
const lazy = () => import('./Heavy.svelte')
</script>
{#await import('./Card.svelte')}<p>loading</p>{/await}
"#,
        )?;
        compare_extraction_path(&svelte, "extract_svelte")?;

        fs::write(
            directory.path().join("components/Hero.astro"),
            "---\n---\n<h1>hero</h1>\n",
        )?;
        fs::write(
            directory.path().join("client.ts"),
            "export function hydrate() {}\n",
        )?;
        let astro = directory.path().join("Page.astro");
        fs::write(
            &astro,
            r#"---
import Hero from './components/Hero.astro';
const Mod = await import('./components/Hero.astro');
---
<Hero />
<script>
import { hydrate } from './client.ts';
hydrate();
</script>
"#,
        )?;
        compare_extraction_path(&astro, "extract_astro")?;
        Ok(())
    }

    #[test]
    fn extensionless_shebang_extraction_matches_exactly() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let python = directory.path().join("pytool");
        fs::write(
            &python,
            "#!/usr/bin/env -S python3 -u\ndef main():\n    print('ok')\n",
        )?;
        compare_extraction_path(&python, "extract_python")?;

        let shell = directory.path().join("devctl");
        fs::write(
            &shell,
            "#!/usr/bin/env -i DEBUG=1 bash\nrun() { echo ok; }\nrun\n",
        )?;
        compare_extraction_path(&shell, "extract_bash")?;

        let node = directory.path().join("serve");
        fs::write(
            &node,
            "#!/usr/bin/node\nfunction serve() { console.log('ok') }\nserve()\n",
        )?;
        compare_extraction_path(&node, "extract_js")?;
        Ok(())
    }

    #[test]
    fn graph_construction_matches_python() -> Result<(), Box<dyn Error>> {
        let repo = repository_root();
        compare_graph_build(&repo.join("tests/fixtures/extraction.json"))?;

        let directory = tempfile::tempdir()?;
        let source = directory.path().join("edge-cases.json");
        fs::write(
            &source,
            serde_json::to_vec(&json!({
                "nodes": [
                    {"id": "a", "label": "A", "source": "src\\a.py"},
                    {"id": "b", "label": "B", "file_type": "tool", "source_file": "src/b.py"},
                    {"id": "a", "label": "A2", "file_type": "code", "source_file": "src/a.py"}
                ],
                "edges": [
                    {"source": "A", "target": "b", "relation": "calls", "weight": null, "confidence_score": "bad"},
                    {"source": "a", "target": "external", "relation": "imports"},
                    {"source": "b", "target": "a", "relation": "calls", "weight": 2.5}
                ],
                "hyperedges": [
                    {"id": "flow", "members": ["a", "a", "b", "missing"], "source_file": "src\\a.py"}
                ]
            }))?,
        )?;
        compare_graph_build(&source)?;
        Ok(())
    }

    #[test]
    fn shared_cross_file_call_resolution_matches_python() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let files = [
            (
                "a.py",
                "from b import target\ndef caller():\n    return target()\n",
            ),
            ("b.py", "def target():\n    return 1\n"),
            ("c.py", "def inferred():\n    return target()\n"),
            ("helper.js", "export function jsTarget() { return 1 }\n"),
            (
                "caller.js",
                "export function blocked() { return jsTarget() }\n",
            ),
            ("dep.js", "export function importedTarget() { return 1 }\n"),
            (
                "imported.js",
                "import { importedTarget } from './dep.js';\nexport function accepted() { return importedTarget() }\n",
            ),
        ];
        let mut paths = Vec::new();
        let mut engine = Engine::default();
        let mut extractions = Vec::new();
        let mut sources = std::collections::HashMap::new();
        for (name, contents) in files {
            let path = directory.path().join(name);
            fs::write(&path, contents)?;
            paths.push(path.clone());
            extractions.push(engine.extract(&path)?);
            sources.insert(path.to_string_lossy().into_owned(), contents.to_owned());
        }
        let resolved = resolve(&extractions, &sources);
        let labels = resolved
            .nodes
            .iter()
            .map(|node| (node.id.as_str(), node.label()))
            .collect::<std::collections::HashMap<_, _>>();
        let mut rust = resolved
            .edges
            .iter()
            .filter(|edge| edge.string("relation") == "calls")
            .filter_map(|edge| {
                Some(json!({
                    "source": labels.get(edge.source.as_str())?,
                    "target": labels.get(edge.target.as_str())?,
                    "confidence": edge.string("confidence"),
                    "score": edge.attributes.get("confidence_score").cloned().unwrap_or(Value::Null),
                }))
            })
            .collect::<Vec<_>>();
        rust.sort_by_key(ToString::to_string);

        let repo = repository_root();
        let output = Command::new(python_executable(&repo))
            .args([
                "-c",
                "import json,sys; from pathlib import Path; from graphify.extract import extract; ps=[Path(p) for p in sys.argv[2:]]; x=extract(ps, cache_root=Path(sys.argv[1])); labels={n['id']:n.get('label',n['id']) for n in x['nodes']}; rows=[{'source':labels[e['source']],'target':labels[e['target']],'confidence':e.get('confidence',''),'score':e.get('confidence_score')} for e in x['edges'] if e.get('relation')=='calls' and e['source'] in labels and e['target'] in labels]; print(json.dumps(sorted(rows,key=lambda r:json.dumps(r,sort_keys=True)),ensure_ascii=False))",
            ])
            .arg(directory.path())
            .args(&paths)
            .current_dir(&repo)
            .env("PYTHONPATH", &repo)
            .output()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let python: Vec<Value> = serde_json::from_slice(&output.stdout)?;
        assert_eq!(rust, python);
        Ok(())
    }

    #[test]
    fn deterministic_entity_dedup_matches_python() -> Result<(), Box<dyn Error>> {
        let fixture = json!({
            "nodes": [
                {"id":"u1","label":"User Service","file_type":"concept","source_file":"svc.md"},
                {"id":"u2_c1","label":"user_service","file_type":"concept","source_file":"svc.md"},
                {"id":"g1_long","label":"GraphExtractor","file_type":"concept","source_file":"a.md"},
                {"id":"g2","label":"Graph Extractor","file_type":"concept","source_file":"b.md"},
                {"id":"sku1","label":"ASR1603","file_type":"concept","source_file":"models.md"},
                {"id":"sku2","label":"ASR1605","file_type":"concept","source_file":"models.md"},
                {"id":"code_a","label":"render","file_type":"code","source_file":"a.rs"},
                {"id":"code_b","label":"render","file_type":"code","source_file":"b.rs"},
                {"id":"r1","label":"Django app config for cards. No business logic here. Domain services live in services.py.","file_type":"rationale","source_file":"cards/apps.py"},
                {"id":"r2","label":"Django app config for cores. No business logic here. Domain services live in services.py.","file_type":"rationale","source_file":"cores/apps.py"},
                {"id":"n1","label":"Pipeline placement 4 call sites ADR 0013 D4","file_type":"concept","source_file":"a.md"},
                {"id":"n2","label":"Pipeline placement 4 call sites ADR 0011 D5","file_type":"concept","source_file":"b.md"},
                {"id":"agents_make_batch_fixtures_make_batch_fixtures","label":"make-batch-fixtures","file_type":"concept","source_file":"available/diagnose-issue/SKILL.md"},
                {"id":"agents_make_batch_fixtures_make_batch_fixtures","label":"make-batch-fixtures agent","file_type":"concept","source_file":"agents/make-batch-fixtures.md"}
            ],
            "edges": [
                {"source":"u2_c1","target":"g1_long","relation":"uses"},
                {"source":"g1_long","target":"g2","relation":"same"},
                {"source":"code_a","target":"code_b","relation":"calls"}
            ]
        });
        let nodes: Vec<trail_model::NodeRecord> = serde_json::from_value(fixture["nodes"].clone())?;
        let edges: Vec<trail_model::EdgeRecord> = serde_json::from_value(fixture["edges"].clone())?;
        let result = deduplicate_entities(&nodes, &edges, &std::collections::HashMap::new())?;
        let rust = json!({"nodes": result.nodes, "edges": result.edges});

        let repo = repository_root();
        let mut child = Command::new(python_executable(&repo))
            .args([
                "-c",
                "import contextlib,io,json,sys; from graphify.dedup import deduplicate_entities; x=json.load(sys.stdin); out=io.StringIO(); err=io.StringIO();\nwith contextlib.redirect_stdout(out), contextlib.redirect_stderr(err): n,e=deduplicate_entities(x['nodes'],x['edges'],communities={})\nprint(json.dumps({'nodes':n,'edges':e},ensure_ascii=False))",
            ])
            .current_dir(&repo)
            .env("PYTHONPATH", &repo)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;
        child
            .stdin
            .as_mut()
            .ok_or("Python dedup oracle stdin unavailable")?
            .write_all(&serde_json::to_vec(&fixture)?)?;
        let output = child.wait_with_output()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let python: Value = serde_json::from_slice(&output.stdout)?;
        assert_eq!(rust, python);
        Ok(())
    }

    #[test]
    fn deterministic_clustering_matches_python() -> Result<(), Box<dyn Error>> {
        let nodes = ["a", "b", "c", "d", "e", "f", "hub", "isolate"]
            .into_iter()
            .map(|id| json!({"id":id,"label":format!("{id}()") }))
            .collect::<Vec<_>>();
        let links = [
            ("a", "b", 1.0),
            ("a", "c", 1.0),
            ("b", "c", 1.0),
            ("d", "e", 1.0),
            ("d", "f", 1.0),
            ("e", "f", 1.0),
            ("c", "d", 0.25),
            ("hub", "a", 1.0),
            ("hub", "b", 1.0),
            ("hub", "d", 1.0),
            ("hub", "e", 1.0),
        ]
        .into_iter()
        .map(|(source, target, weight)| json!({"source":source,"target":target,"weight":weight}))
        .collect::<Vec<_>>();
        let fixture = json!({
            "directed": false,
            "multigraph": false,
            "graph": {},
            "nodes": nodes,
            "links": links,
        });
        let graph: trail_model::GraphDocument = serde_json::from_value(fixture.clone())?;
        for percentile in [None, Some(75.0)] {
            let communities = cluster(
                &graph,
                ClusterOptions {
                    resolution: 1.0,
                    exclude_hubs_percentile: percentile,
                },
            );
            let rust = json!({
                "communities": communities,
                "labels": label_communities_by_hub(&graph, &communities),
                "signatures": community_member_signatures(&communities),
                "cohesion": score_communities(&graph, &communities),
            });
            let repo = repository_root();
            let mut child = Command::new(python_executable(&repo))
                .args([
                    "-c",
                    "import json,sys,networkx as nx; from graphify.cluster import cluster,label_communities_by_hub,community_member_sigs,score_all; x=json.load(sys.stdin); p=x.pop('percentile'); G=nx.node_link_graph(x,edges='links'); c=cluster(G,exclude_hubs_percentile=p); print(json.dumps({'communities':c,'labels':label_communities_by_hub(G,c),'signatures':community_member_sigs(c),'cohesion':score_all(G,c)},sort_keys=True))",
                ])
                .current_dir(&repo)
                .env("PYTHONPATH", &repo)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()?;
            let mut input = fixture.clone();
            input["percentile"] = serde_json::to_value(percentile)?;
            child
                .stdin
                .as_mut()
                .ok_or("Python cluster oracle stdin unavailable")?
                .write_all(&serde_json::to_vec(&input)?)?;
            let output = child.wait_with_output()?;
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let python: Value = serde_json::from_slice(&output.stdout)?;
            assert_eq!(rust, python, "hub percentile {percentile:?}");
        }
        Ok(())
    }

    #[test]
    fn deterministic_analysis_matches_python() -> Result<(), Box<dyn Error>> {
        let fixture = json!({
            "directed": false,
            "multigraph": false,
            "graph": {},
            "nodes": [
                {"id":"a","label":"AuthService","source_file":"src/a.py","file_type":"code"},
                {"id":"b","label":"BillingService","source_file":"src/b.py","file_type":"code"},
                {"id":"c","label":"Cache","source_file":"lib/c.ts","file_type":"code"},
                {"id":"d","label":"Dispatcher","source_file":"lib/d.ts","file_type":"code"},
                {"id":"weak","label":"WeakLink","source_file":"src/weak.py","file_type":"code"}
            ],
            "links": [
                {"source":"a","target":"b","relation":"calls","confidence":"AMBIGUOUS","source_file":"src/a.py","_src":"a","_tgt":"b"},
                {"source":"a","target":"c","relation":"uses","confidence":"INFERRED","source_file":"src/a.py","_src":"a","_tgt":"c"},
                {"source":"b","target":"c","relation":"calls","confidence":"EXTRACTED","source_file":"src/b.py","_src":"b","_tgt":"c"},
                {"source":"c","target":"d","relation":"imports_from","confidence":"EXTRACTED","source_file":"lib/c.ts","_src":"c","_tgt":"d"},
                {"source":"d","target":"c","relation":"imports_from","confidence":"EXTRACTED","source_file":"lib/d.ts","_src":"d","_tgt":"c"}
            ]
        });
        let graph: trail_model::GraphDocument = serde_json::from_value(fixture.clone())?;
        let communities = std::collections::BTreeMap::from([
            (0, vec!["a".to_owned(), "b".to_owned(), "weak".to_owned()]),
            (1, vec!["c".to_owned(), "d".to_owned()]),
        ]);
        let labels =
            std::collections::BTreeMap::from([(0, "Auth".to_owned()), (1, "Runtime".to_owned())]);
        let mut newer = graph.clone();
        newer.nodes.push(serde_json::from_value(json!({
            "id":"new","label":"NewNode","source_file":"src/new.py","file_type":"code"
        }))?);
        newer.links.push(serde_json::from_value(json!({
            "source":"weak","target":"new","relation":"uses","confidence":"INFERRED"
        }))?);
        let rust = json!({
            "gods": god_nodes(&graph, 10),
            "surprises": surprising_connections(&graph, &communities, 5),
            "questions": suggest_questions(&graph, &communities, &labels, 10),
            "diff": graph_diff(&graph, &newer),
            "cycles": find_import_cycles(&graph, 5, 20),
        });

        let input = json!({
            "graph": fixture,
            "communities": communities,
            "labels": labels,
            "new_graph": serde_json::to_value(&newer)?,
        });
        let repo = repository_root();
        let mut child = Command::new(python_executable(&repo))
            .args([
                "-c",
                "import json,sys,networkx as nx; from graphify.analyze import god_nodes,surprising_connections,suggest_questions,graph_diff,find_import_cycles; x=json.load(sys.stdin); G=nx.node_link_graph(x['graph'],edges='links'); N=nx.node_link_graph(x['new_graph'],edges='links'); c={int(k):v for k,v in x['communities'].items()}; l={int(k):v for k,v in x['labels'].items()}; print(json.dumps({'gods':god_nodes(G,10),'surprises':surprising_connections(G,c,5),'questions':suggest_questions(G,c,l,10),'diff':graph_diff(G,N),'cycles':find_import_cycles(G,5,20)},sort_keys=True))",
            ])
            .current_dir(&repo)
            .env("PYTHONPATH", &repo)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;
        child
            .stdin
            .as_mut()
            .ok_or("Python analysis oracle stdin unavailable")?
            .write_all(&serde_json::to_vec(&input)?)?;
        let output = child.wait_with_output()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let python: Value = serde_json::from_slice(&output.stdout)?;
        assert_eq!(rust, python);
        Ok(())
    }

    #[test]
    fn import_cycle_analysis_matches_python() -> Result<(), Box<dyn Error>> {
        let fixture = json!({
            "directed": true,
            "multigraph": false,
            "graph": {},
            "nodes": [
                {"id":"a","label":"a.ts","source_file":"src/a.ts"},
                {"id":"b","label":"b.ts","source_file":"src/b.ts"},
                {"id":"c","label":"c.ts","source_file":"src/c.ts"},
                {"id":"d","label":"d.ts","source_file":"src/d.ts"},
                {"id":"external","label":"react"}
            ],
            "links": [
                {"source":"a","target":"b","relation":"imports_from","source_file":"src/a.ts"},
                {"source":"b","target":"a","relation":"imports_from","source_file":"src/b.ts"},
                {"source":"b","target":"c","relation":"imports_from","source_file":"src/b.ts"},
                {"source":"c","target":"d","relation":"imports_from","source_file":"src/c.ts"},
                {"source":"d","target":"b","relation":"re_exports","source_file":"src/d.ts"},
                {"source":"c","target":"c","relation":"imports_from","source_file":"src/c.ts"},
                {"source":"a","target":"external","relation":"imports_from","source_file":"src/a.ts"},
                {"source":"d","target":"a","relation":"imports_from","source_file":"src/d.ts","deferred":true}
            ]
        });
        let graph: trail_model::GraphDocument = serde_json::from_value(fixture.clone())?;
        let rust = serde_json::to_value(find_import_cycles(&graph, 5, 20))?;
        let repo = repository_root();
        let output = Command::new(python_executable(&repo))
            .args([
                "-c",
                "import json,sys,networkx as nx; from graphify.analyze import find_import_cycles; G=nx.node_link_graph(json.load(sys.stdin),edges='links'); print(json.dumps(find_import_cycles(G,5,20)))",
            ])
            .current_dir(&repo)
            .env("PYTHONPATH", &repo)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;
        let mut child = output;
        child
            .stdin
            .as_mut()
            .ok_or("Python cycle oracle stdin unavailable")?
            .write_all(&serde_json::to_vec(&fixture)?)?;
        let output = child.wait_with_output()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let python: Value = serde_json::from_slice(&output.stdout)?;
        assert_eq!(rust, python);
        Ok(())
    }

    #[test]
    fn json_and_cypher_exports_match_python() -> Result<(), Box<dyn Error>> {
        let repo = repository_root();
        let fixture_path = repo.join("tests/fixtures/extraction.json");
        let extraction: trail_languages::Extraction =
            serde_json::from_slice(&fs::read(&fixture_path)?)?;
        let graph = build_from_extraction(&extraction, false, None);
        let communities = cluster(&graph, ClusterOptions::default());
        let labels = label_communities_by_hub(&graph, &communities);
        let directory = tempfile::tempdir()?;
        let rust_json = directory.path().join("rust.json");
        write_json(
            &graph,
            &communities,
            &rust_json,
            &JsonExportOptions {
                force: true,
                built_at_commit: Some("0123456789abcdef"),
                community_labels: Some(&labels),
            },
        )?;
        let python_json = directory.path().join("python.json");
        let labels_json = serde_json::to_string(&labels)?;
        let output = Command::new(python_executable(&repo))
            .args([
                "-c",
                "import json,sys; from graphify.build import build_from_json; from graphify.cluster import cluster,label_communities_by_hub; from graphify.export import to_json,to_cypher; x=json.load(open(sys.argv[1])); G=build_from_json(x); c=cluster(G); labels=label_communities_by_hub(G,c); assert labels=={int(k):v for k,v in json.loads(sys.argv[4]).items()}; to_json(G,c,sys.argv[2],force=True,built_at_commit='0123456789abcdef',community_labels=labels); to_cypher(G,sys.argv[3])",
            ])
            .arg(&fixture_path)
            .arg(&python_json)
            .arg(directory.path().join("python.cypher"))
            .arg(labels_json)
            .current_dir(&repo)
            .env("PYTHONPATH", &repo)
            .output()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let rust_value: Value = serde_json::from_slice(&fs::read(&rust_json)?)?;
        let python_value: Value = serde_json::from_slice(&fs::read(&python_json)?)?;
        assert_eq!(rust_value, python_value);
        assert_eq!(fs::read(&rust_json)?, fs::read(&python_json)?);
        assert_eq!(
            cypher_document(&graph),
            fs::read_to_string(directory.path().join("python.cypher"))?
        );
        Ok(())
    }

    fn compare_extraction(fixture: &str, extractor: &str) -> Result<(), Box<dyn Error>> {
        let repo = repository_root();
        let source = repo.join("tests/fixtures").join(fixture);
        compare_extraction_path(&source, extractor).map(|_| ())
    }

    fn compare_graph_build(source: &Path) -> Result<(), Box<dyn Error>> {
        let repo = repository_root();
        let extraction: trail_languages::Extraction = serde_json::from_slice(&fs::read(source)?)?;
        let rust = serde_json::to_value(build_from_extraction(&extraction, false, None))?;
        let output = Command::new(python_executable(&repo))
            .args([
                "-c",
                "import json,sys; from pathlib import Path; from networkx.readwrite import json_graph; from graphify.build import build_from_json; x=json.loads(Path(sys.argv[1]).read_text()); g=build_from_json(x, directed=False); print(json.dumps(json_graph.node_link_data(g, edges='links'), ensure_ascii=False, allow_nan=False))",
            ])
            .arg(source)
            .current_dir(&repo)
            .env("PYTHONPATH", &repo)
            .output()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let python: serde_json::Value = serde_json::from_slice(&output.stdout)?;
        assert_eq!(rust, python, "graph build: {}", source.display());
        Ok(())
    }

    fn compare_extraction_path(
        source: &Path,
        extractor: &str,
    ) -> Result<serde_json::Value, Box<dyn Error>> {
        let repo = repository_root();
        let rust = serde_json::to_value(Engine::default().extract(source)?)?;
        let output = Command::new(python_executable(&repo))
            .args([
                "-c",
                "import json,sys; from pathlib import Path; import graphify.extract as e; print(json.dumps(getattr(e, sys.argv[1])(Path(sys.argv[2])), ensure_ascii=False))",
                extractor,
            ])
            .arg(source)
            .current_dir(&repo)
            .env("PYTHONPATH", &repo)
            .output()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let python: serde_json::Value = serde_json::from_slice(&output.stdout)?;
        assert_eq!(rust, python, "fixture: {}", source.display());
        Ok(rust)
    }

    fn compare(arguments: &[&str]) -> Result<(), Box<dyn Error>> {
        let rust = trail_cli::run(
            Frontend::Graphify,
            arguments.iter().map(|argument| OsString::from(*argument)),
        );
        let repo = repository_root();
        let python = python_executable(&repo);
        let output = Command::new(&python)
            .arg("-m")
            .arg("graphify")
            .args(arguments)
            .current_dir(&repo)
            .env("PYTHONPATH", &repo)
            .env("PYTHONHASHSEED", "0")
            .env("GRAPHIFY_QUERY_LOG_DISABLE", "1")
            .output()?;
        assert_eq!(
            rust.code,
            output.status.code().unwrap_or(1) as u8,
            "{arguments:?}"
        );
        assert_eq!(
            with_newline(&rust.stdout),
            String::from_utf8(output.stdout)?,
            "stdout mismatch for {arguments:?}"
        );
        assert_eq!(
            with_newline(&rust.stderr),
            String::from_utf8(output.stderr)?,
            "stderr mismatch for {arguments:?}"
        );
        Ok(())
    }

    fn with_newline(value: &str) -> String {
        if value.is_empty() {
            String::new()
        } else {
            format!("{value}\n")
        }
    }

    fn repository_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(3)
            .map_or_else(|| PathBuf::from("../.."), Path::to_path_buf)
    }

    fn python_executable(repo: &Path) -> PathBuf {
        if let Ok(value) = std::env::var("GRAPHIFY_PYTHON") {
            return PathBuf::from(value);
        }
        if cfg!(windows) {
            repo.join(".venv/Scripts/python.exe")
        } else {
            repo.join(".venv/bin/python")
        }
    }

    struct Fixture {
        _directory: TempDir,
        graph: PathBuf,
    }

    impl Fixture {
        fn new() -> Result<Self, Box<dyn Error>> {
            let directory = tempfile::tempdir()?;
            let graph = directory.path().join("graph.json");
            let document = json!({
                "directed": false,
                "multigraph": false,
                "graph": {},
                "nodes": [
                    {"id": "extract", "label": "extract", "source_file": "extract.py", "source_location": "L10", "community": 0},
                    {"id": "cluster", "label": "cluster", "source_file": "cluster.py", "source_location": "L5", "community": 0},
                    {"id": "build", "label": "build", "source_file": "build.py", "source_location": "L1", "community": 1},
                    {"id": "create", "label": "createPatchHandler()", "source_file": "create.ts", "source_location": "L2", "community": 2},
                    {"id": "validate", "label": "validateSanitySession()", "source_file": "validate.ts", "source_location": "L4", "community": 2}
                ],
                "links": [
                    {"source": "extract", "target": "cluster", "relation": "calls", "confidence": "EXTRACTED", "context": "call"},
                    {"source": "cluster", "target": "build", "relation": "imports", "confidence": "EXTRACTED", "context": "import"},
                    {"source": "create", "target": "validate", "relation": "calls", "confidence": "EXTRACTED", "context": "call"}
                ]
            });
            fs::write(&graph, serde_json::to_vec(&document)?)?;
            Ok(Self {
                _directory: directory,
                graph,
            })
        }

        fn graph_string(&self) -> &str {
            self.graph.to_str().unwrap_or_default()
        }
    }
}
