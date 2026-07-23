use std::error::Error;
use std::fs;

use compass_languages::{Engine, ExtractError, make_id};

#[test]
fn caller_supplied_source_matches_file_based_generic_extraction() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("source.rs");
    let source = b"pub struct Service;\nimpl Service { pub fn run(&self) {} }\n";
    fs::write(&path, source)?;

    let from_file = Engine::default().extract(&path)?;
    let from_memory = Engine::default().extract_source(&path, source)?;
    assert_eq!(from_memory, from_file);
    Ok(())
}

#[test]
fn python_indirect_rationale_types_and_binding_shapes_are_extracted() -> Result<(), Box<dyn Error>>
{
    let directory = tempfile::tempdir()?;
    let source = directory.path().join("advanced.py");
    fs::write(
        &source,
        r#"""A module rationale long enough to become a rationale node."""
from package import imported as alias
from typing import Annotated, Callable, Generic, Optional, TypeVar, Union

T = TypeVar("T")
external_map = {"handler": external_handler}
external_list = [first_handler, second_handler]

class Base(Generic[T]):
    """A class rationale long enough to be indexed safely."""

class Service(Base[ExternalType]):
    def execute(
        self,
        callback: Callable[[InputType], OutputType],
        value: Annotated[Optional[Union[InputType, None]], "meta"],
    ) -> tuple[OutputType, ...]:
        """A function rationale long enough to be indexed safely."""
        # WHY: retain this adapter for compatibility with old callers
        local = callback
        assigned = (external_factory, local)
        consume(external_argument, named=external_keyword)
        mapping = {"one": dictionary_handler, "bound": local}
        handlers = {set_handler, local}
        alias()
        with open_resource() as resource:
            resource.use()
        for item in iterator_factory():
            item.run()
        try:
            risky()
        except ErrorType as error:
            error.handle()
        return external_result

def top_level(arg: InputType) -> OutputType:
    alias()
    return external_top
"#,
    )?;
    let mut engine = Engine::default();
    let extraction = engine.extract(&source)?;
    assert!(
        extraction
            .nodes
            .iter()
            .any(|node| node.string("file_type") == "rationale")
    );
    assert!(
        extraction
            .edges
            .iter()
            .any(|edge| edge.string("relation") == "rationale_for")
    );
    let raw = extraction.raw_calls.as_deref().unwrap_or_default();
    for callee in ["set_handler", "external_result", "external_top"] {
        assert!(
            raw.iter().any(|call| call.callee == callee),
            "missing {callee}; calls={raw:?}"
        );
    }
    assert!(extraction.nodes.len() >= 2);
    Ok(())
}

#[test]
fn generated_python_javascript_exports_and_static_type_families_cover_rare_ast_shapes()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let fixtures = [
        (
            "migration.py",
            r#"""This generated module rationale must be suppressed by migration detection."""
revision = "abc"
down_revision = "def"
def upgrade():
    """Nested rationale remains discoverable for the upgrade function."""
    pass
"#,
        ),
        (
            "module.ts",
            r#"export const handler = (value: Input): Output => factory(value);
export const mapping = { first: handler };
class Box<T extends Base> extends Parent implements Contract {
  field: Array<Item>;
  run(arg: Promise<Input>): Result<Output> { return helper(arg); }
}
"#,
        ),
        (
            "Types.kt",
            r#"enum class Mode { FAST, SAFE }
class Box<T : Base>(val item: Item) : Parent(), Contract {
    val values: List<External> = listOf()
    fun <R : Result> run(input: Input): Map<String, R> = helper(input)
}
"#,
        ),
        (
            "Types.scala",
            r#"trait Contract
class Box[T <: Base](value: Input) extends Parent with Contract {
  val field: Either[Failure, T] = ???
  def run[R](input: Option[Input]): (R, Output) = helper(input)
}
"#,
        ),
        (
            "Types.java",
            r#"enum Mode { FAST(1), SAFE(2); Mode(int n) {} }
class Box<T extends Base> extends Parent implements Contract {
  java.util.List<Item> field;
  <R extends Result> R run(Input input) { return helper(input); }
}
"#,
        ),
        (
            "types.c",
            r#"typedef struct Payload Payload;
struct Box { Payload *payload; };
Result run(const Input *input, Output **output) { return helper(input); }
"#,
        ),
    ];
    let mut engine = Engine::default();
    for (name, text) in fixtures {
        let path = directory.path().join(name);
        fs::write(&path, text)?;
        let extraction = engine.extract(&path)?;
        assert!(!extraction.nodes.is_empty(), "{name}");
    }
    let migration = engine.extract(&directory.path().join("migration.py"))?;
    assert!(
        !migration
            .nodes
            .iter()
            .any(|node| node.label().contains("generated module"))
    );
    assert!(!migration.nodes.is_empty());

    let missing = directory.path().join("missing.py");
    assert!(matches!(
        engine.extract(&missing),
        Err(ExtractError::File(_))
    ));
    let unsupported = directory.path().join("unsupported.unknown");
    fs::write(&unsupported, "data")?;
    assert!(matches!(
        engine.extract(&unsupported),
        Err(ExtractError::Unsupported(_))
    ));
    Ok(())
}

#[test]
fn typescript_import_resolution_checks_extensions_and_directory_indexes()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    fs::create_dir_all(directory.path().join("pkg"))?;
    fs::write(
        directory.path().join("target.ts"),
        "export function target() {}\n",
    )?;
    fs::write(
        directory.path().join("view.tsx"),
        "export const View = () => null;\n",
    )?;
    fs::write(
        directory.path().join("pkg/index.tsx"),
        "export const item = 1;\n",
    )?;
    let source = directory.path().join("main.js");
    fs::write(
        &source,
        r#"import { target } from "./target.js";
import { View } from "./view.jsx";
import { item } from "./pkg";
export function run() { target(); View(); return item; }
"#,
    )?;
    let mut engine = Engine::default();
    let extraction = engine.extract(&source)?;
    assert!(
        extraction
            .edges
            .iter()
            .any(|edge| matches!(edge.string("relation").as_str(), "imports" | "imports_from")),
        "edges={:?}",
        extraction.edges
    );
    Ok(())
}

#[test]
fn javascript_modules_reexports_require_and_decorators_keep_graphify_contracts()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    fs::create_dir_all(directory.path().join("pkg"))?;
    fs::write(
        directory.path().join("target.ts"),
        "export function target() {}\nexport function second() {}\n",
    )?;
    fs::write(
        directory.path().join("pkg/index.ts"),
        "export const item = 1;\n",
    )?;
    let barrel = directory.path().join("barrel.ts");
    fs::write(
        &barrel,
        "export { target, second } from './target';\nexport * from './pkg';\n",
    )?;
    let decorated = directory.path().join("decorated.ts");
    fs::write(
        &decorated,
        "import { Injectable } from '@nestjs/common';\n@Injectable()\nexport class Service {}\n",
    )?;
    let common_js = directory.path().join("loader.cjs");
    fs::write(
        &common_js,
        "const { target } = require('./target');\nmodule.exports = { target };\n",
    )?;

    let mut engine = Engine::default();
    let barrel_facts = engine.extract(&barrel)?;
    assert_eq!(
        barrel_facts
            .edges
            .iter()
            .filter(|edge| {
                edge.string("relation") == "re_exports" && edge.string("context") == "export"
            })
            .count(),
        0,
        "file-level re-export edges belong to the collection resolver"
    );
    assert_eq!(
        barrel_facts
            .edges
            .iter()
            .filter(|edge| {
                edge.string("relation") == "imports_from" && edge.string("context") == "re-export"
            })
            .count(),
        2
    );
    assert!(barrel_facts.edges.iter().any(|edge| {
        edge.string("relation") == "re_exports" && edge.string("context") == "re-export"
    }));

    let decorated_facts = engine.extract(&decorated)?;
    let decorator_id = make_id(&["Injectable"]);
    assert!(decorated_facts.nodes.iter().any(|node| {
        node.id == decorator_id
            && node.label() == "Injectable"
            && node.string("source_file").is_empty()
    }));
    assert!(decorated_facts.edges.iter().any(|edge| {
        edge.target == decorator_id
            && edge.string("relation") == "references"
            && edge.string("context") == "decorator"
    }));
    assert!(decorated_facts.edges.iter().any(|edge| {
        edge.target == make_id(&["ref", "@nestjs/common"])
            && edge.string("relation") == "imports_from"
    }));
    assert!(
        !decorated_facts
            .edges
            .iter()
            .any(|edge| { edge.string("relation") == "imports" && edge.target == decorator_id })
    );

    let cjs_facts = engine.extract(&common_js)?;
    assert!(
        cjs_facts
            .edges
            .iter()
            .any(|edge| edge.string("relation") == "imports_from")
    );
    assert!(
        cjs_facts
            .edges
            .iter()
            .any(|edge| edge.string("relation") == "imports")
    );
    Ok(())
}

#[test]
fn objective_c_go_and_swift_fixtures_cover_type_members_calls_and_imports()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    fs::write(
        directory.path().join("Local.h"),
        "@interface Local : NSObject\n@end\n",
    )?;
    let fixtures = [
        (
            "Service.m",
            r#"NS_ASSUME_NONNULL_BEGIN
#import <Foundation/Foundation.h>
#import "Local.h"
@import UIKit;

@protocol Child <NSObject>
- (void)required;
@end

@interface Service : NSObject <Child>
@property(nonatomic, strong) ExternalType *field;
- (void)helper;
- (Result *)run:(Input *)input;
@end

@implementation Service
- (void)helper {}
- (Result *)run:(Input *)input {
    ExternalType *local = [[ExternalType alloc] init];
    [self helper];
    [ExternalType alloc];
    self.helper;
    @selector(helper);
    return [local execute:input];
}
@end
NS_ASSUME_NONNULL_END
"#,
        ),
        (
            "service.go",
            r#"package service

import (
    "context"
    alias "example.com/project/dependency"
)

type Embedded interface { Base; Run(context.Context) error }
type Box[T any] struct { Value T; Client *alias.Client }

func NewBox[T any](value T) *Box[T] { return &Box[T]{Value: value} }
func (b *Box[T]) Run(ctx context.Context) error {
    defer cleanup()
    go notify(b.Value)
    alias.Handle(ctx)
    return b.Client.Execute(ctx)
}
"#,
        ),
        (
            "Service.swift",
            r#"import Foundation

protocol Runnable: AnyObject { func run(_ input: Input) async throws -> Output }
class Base {}
final class Service<T: Contract>: Base, Runnable {
    let dependency: Dependency
    init(dependency: Dependency) { self.dependency = dependency }
    func run(_ input: Input) async throws -> Output {
        let value: Intermediate = try await dependency.load(input)
        return helper(value)
    }
}
extension Service { func helper(_ value: Intermediate) -> Output { Output(value) } }
enum Mode { case fast; case safe }
struct Wrapper { var service: Service<Concrete> }
"#,
        ),
    ];

    let mut engine = Engine::default();
    for (name, text) in fixtures {
        let path = directory.path().join(name);
        fs::write(&path, text)?;
        let extraction = engine.extract(&path)?;
        assert!(
            extraction.nodes.len() >= 3,
            "{name}: {:?}",
            extraction.nodes
        );
        assert!(
            extraction.edges.iter().any(|edge| {
                matches!(edge.string("relation").as_str(), "imports" | "imports_from")
            }),
            "{name}: {:?}",
            extraction.edges
        );
        assert!(
            extraction.edges.iter().any(|edge| {
                matches!(
                    edge.string("relation").as_str(),
                    "calls" | "references" | "inherits" | "implements"
                )
            }),
            "{name}: {:?}",
            extraction.edges
        );
    }
    let objc = engine.extract(&directory.path().join("Service.m"))?;
    assert!(objc.extensions.contains_key("objc_type_table"));
    assert!(!objc.raw_calls.as_deref().unwrap_or_default().is_empty());
    Ok(())
}

#[test]
fn cpp_and_dream_maker_fixtures_cover_qualified_generics_overrides_and_receivers()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    fs::write(directory.path().join("local.hpp"), "struct Local {};\n")?;
    fs::write(
        directory.path().join("helpers.dm"),
        "/proc/helper()\n\treturn 1\n",
    )?;
    let fixtures = [
        (
            "advanced.cpp",
            r#"#include "local.hpp"
#include <vector>
namespace api { class Base {}; void global_call(); }
template <typename T> class GenericBase {};
class Service : public api::Base, public GenericBase<Local> {
public:
    Local value;
    std::vector<Local*> items;
    Local *pointer, &reference;
    Local* run(const Local& input) {
        this->helper();
        api::global_call();
        pointer->execute();
        return factory(input);
    }
    void helper() {}
};
Local* free_call(Service& service) { service.helper(); return create(); }
"#,
        ),
        (
            "advanced.dm",
            r#"#include "helpers.dm"
/proc/log_event(message)
	world.log << message

/datum/base
	proc/run()
		return helper()

/datum/service
	parent_type = /datum/base
	var/datum/base/dependency
	proc/helper()
		return 1
	proc/run()
		var/datum/service/local = new /datum/service()
		local.helper()
		src.helper()
		return ..()

/datum/service/proc/external()
	log_event("external")
	return new /datum/base()
"#,
        ),
    ];
    let mut engine = Engine::default();
    for (name, source) in fixtures {
        let path = directory.path().join(name);
        fs::write(&path, source)?;
        let extraction = engine.extract(&path)?;
        assert!(
            extraction.nodes.len() >= 4,
            "{name}: {:?}",
            extraction.nodes
        );
        assert!(
            extraction
                .edges
                .iter()
                .any(|edge| edge.string("relation") == "imports"
                    || edge.string("relation") == "imports_from")
        );
        assert!(
            extraction.edges.iter().any(|edge| {
                matches!(
                    edge.string("relation").as_str(),
                    "calls" | "references" | "inherits"
                )
            }),
            "{name}: {:?}",
            extraction.edges
        );
        if name == "advanced.cpp" {
            assert!(
                !extraction
                    .raw_calls
                    .as_deref()
                    .unwrap_or_default()
                    .is_empty()
            );
        }
    }
    Ok(())
}

#[test]
fn dotnet_pascal_xaml_and_template_fixtures_cover_project_and_ui_relationships()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let fixtures = [
        (
            "Service.cs",
            r#"using System;
using System.Collections.Generic;
namespace Compass.App;
public interface IRunner<in T> { Result Run(T input); }
public abstract class Base<T> where T : Contract { protected T Value { get; init; } }
public sealed record Service<T>(Dependency Dependency) : Base<T>, IRunner<Input>
    where T : Contract, new()
{
    public event EventHandler? Changed;
    public Result Run(Input input)
    {
        var item = Dependency.Load(input);
        Changed?.Invoke(this, EventArgs.Empty);
        return Helper.Create<Result>(item);
    }
}
public enum Mode { Fast, Safe }
public delegate Output Transform(Input input);
"#,
        ),
        (
            "Compass.csproj",
            r#"<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup><TargetFramework>net9.0</TargetFramework></PropertyGroup>
  <ItemGroup>
    <ProjectReference Include="../Core/Core.csproj" />
    <PackageReference Include="System.Text.Json" Version="9.0.0" />
    <Compile Include="Generated.cs" Link="Shared/Generated.cs" />
  </ItemGroup>
</Project>"#,
        ),
        (
            "MainWindow.xaml",
            r#"<Window x:Class="Compass.App.MainWindow"
 xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation"
 xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml"
 xmlns:local="clr-namespace:Compass.App">
 <Grid DataContext="{Binding Main}">
  <local:GraphView x:Name="Graph" ItemsSource="{Binding Nodes}" />
  <Button Click="OnRefresh" Command="{Binding RefreshCommand}" />
 </Grid>
</Window>"#,
        ),
        (
            "units.pas",
            r#"unit Units;
interface
uses SysUtils, Classes;
type
  IRunner = interface
    function Run(const Input: TInput): TOutput;
  end;
  TService = class(TBase, IRunner)
  private
    FDependency: TDependency;
    procedure Helper(Sender: TObject);
  public
    constructor Create(const Dependency: TDependency);
    function Run(const Input: TInput): TOutput; override;
    property Dependency: TDependency read FDependency write FDependency;
  end;
implementation
constructor TService.Create(const Dependency: TDependency);
begin inherited Create; FDependency := Dependency; end;
procedure TService.Helper(Sender: TObject);
begin FDependency.Notify(Sender); end;
function TService.Run(const Input: TInput): TOutput;
begin Result := FDependency.Execute(Input); Helper(Self); end;
end."#,
        ),
        (
            "MainForm.dfm",
            r#"object MainForm: TMainForm
  Caption = 'Compass'
  object RefreshButton: TButton
    OnClick = RefreshButtonClick
  end
  object DataSource1: TDataSource
    DataSet = Query1
  end
end"#,
        ),
        (
            "Component.vue",
            r#"<script setup lang="ts">
import { computed, ref } from 'vue'
import GraphView from './GraphView.vue'
const props = defineProps<{ nodes: Node[] }>()
const emit = defineEmits<{ select: [Node] }>()
const count = computed(() => props.nodes.length)
function choose(node: Node) { emit('select', node) }
</script>
<template><GraphView v-for="node in props.nodes" :key="node.id" @click="choose(node)" /></template>"#,
        ),
        (
            "Widget.svelte",
            r#"<script lang="ts">
 import { onMount } from 'svelte';
 export let service: Service;
 $: result = service.compute();
 onMount(() => service.start());
</script>
{#each result as item}<button on:click={() => service.select(item)}>{item.name}</button>{/each}"#,
        ),
        (
            "Card.astro",
            r#"---
import Layout from './Layout.astro';
const { title, items } = Astro.props;
const rows = items.map(formatRow);
---
<Layout title={title}>{rows.map(row => <span>{row}</span>)}</Layout>"#,
        ),
    ];

    let mut engine = Engine::default();
    for (name, source) in fixtures {
        let path = directory.path().join(name);
        fs::write(&path, source)?;
        let extraction = engine.extract(&path)?;
        assert!(!extraction.nodes.is_empty(), "{name}");
        assert!(
            extraction.edges.iter().any(|edge| matches!(
                edge.string("relation").as_str(),
                "contains"
                    | "calls"
                    | "references"
                    | "imports"
                    | "imports_from"
                    | "inherits"
                    | "implements"
            )),
            "{name}: {:?}",
            extraction.edges
        );
    }
    Ok(())
}
