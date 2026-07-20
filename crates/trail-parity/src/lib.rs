//! Development-only differential verification against the pinned Python baseline.

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::ffi::OsString;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use serde_json::json;
    use tempfile::TempDir;
    use trail_cli::Frontend;
    use trail_files::{
        Cache, CacheKind, DetectOptions, Manifest, ManifestKind, detect, file_hash,
        prompt_fingerprint,
    };
    use trail_languages::Engine;

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

    fn compare_extraction(fixture: &str, extractor: &str) -> Result<(), Box<dyn Error>> {
        let repo = repository_root();
        let source = repo.join("tests/fixtures").join(fixture);
        compare_extraction_path(&source, extractor).map(|_| ())
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
