#[test]
fn go_closures_resolve_typed_parameters_and_captured_receivers() {
    let go_source = br#"package pkg
import "io/fs"
type Worker struct{}
func (worker *Worker) Run() {}
func caller(worker *Worker) {
    visit := func(entry fs.DirEntry) {
        entry.Name()
        worker.Run()
    }
    shadow := func(worker any) {
        worker.Run()
    }
    variadic := func(worker ...*Worker) {
        worker.Run()
    }
    capture := func() {
        worker.Run()
    }
    _ = visit
    _ = shadow
    _ = variadic
    _ = capture
}
"#;
    let extracted = extract("pkg/caller.go", go_source);
    let sources = HashMap::from([(
        "pkg/caller.go".to_owned(),
        String::from_utf8(go_source.to_vec()).expect("source"),
    )]);
    let resolved = compass_resolve::resolve(&[extracted], &sources);
    let worker_run = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "pkg.Worker::Run")
        .expect("Worker.Run declaration");
    let entry_name = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "io/fs.DirEntry::Name")
        .expect("external fs.DirEntry.Name method");

    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.target == worker_run.id
            && edge.string("source_location") == "L8"
    }));
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.target == entry_name.id
            && edge.string("source_location") == "L7"
            && edge.string("resolution_rule") == "qualified-external"
    }));
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.target == worker_run.id
            && edge.string("source_location") == "L17"
    }));
    assert!(!resolved.edges.iter().any(|edge| {
        let location = edge.string("source_location");
        edge.string("relation") == "calls" && (location == "L11" || location == "L14")
    }));
}

#[test]
fn go_variadic_ranges_and_nested_closures_resolve_element_receivers() {
    let go_source = br#"package pkg
type Worker struct{}
func (worker *Worker) Run() {}
type Command struct{}
func (*Command) Commands() []*Command { return nil }
func (*Command) IsAvailableCommand() bool { return true }
func (*Command) Name() string { return "" }
func caller(workers ...*Worker) {
    for _, worker := range workers {
        worker.Run()
    }
    visit := func(command *Command) {
        for _, subCommand := range command.Commands() {
            if subCommand.IsAvailableCommand() {
                _ = subCommand.Name()
            }
        }
    }
    _ = visit
}
"#;
    let extracted = extract("pkg/caller.go", go_source);
    let sources = HashMap::from([(
        "pkg/caller.go".to_owned(),
        String::from_utf8(go_source.to_vec()).expect("source"),
    )]);
    let resolved = compass_resolve::resolve(&[extracted], &sources);
    let worker_run = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "pkg.Worker::Run")
        .expect("Worker.Run declaration");
    let command_available = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "pkg.Command::IsAvailableCommand")
        .expect("Command.IsAvailableCommand declaration");
    let command_name = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "pkg.Command::Name")
        .expect("Command.Name declaration");

    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.target == worker_run.id
            && edge.string("source_location") == "L10"
    }));
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.target == command_available.id
            && edge.string("source_location") == "L14"
    }));
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.target == command_name.id
            && edge.string("source_location") == "L15"
    }));
}

#[test]
fn go_top_level_range_variable_preserves_method_attribution() {
    let go_source = br#"package pkg
type Command struct{}
func (*Command) Commands() []*Command { return nil }
func (*Command) IsAvailableCommand() bool { return true }
func (*Command) Name() string { return "" }
func caller(command *Command) {
    for _, c := range command.Commands() {
        if !c.IsAvailableCommand() {
            continue
        }
        _ = c.Name()
    }
}
"#;
    let extracted = extract("pkg/caller.go", go_source);
    let sources = HashMap::from([(
        "pkg/caller.go".to_owned(),
        String::from_utf8(go_source.to_vec()).expect("source"),
    )]);
    let resolved = compass_resolve::resolve(&[extracted], &sources);
    let command_methods = ["Commands", "IsAvailableCommand", "Name"]
        .into_iter()
        .map(|method| {
            resolved
                .nodes
                .iter()
                .find(|node| node.string("qualified_name") == format!("pkg.Command::{method}"))
                .unwrap_or_else(|| panic!("Command.{method} declaration"))
                .id
                .clone()
        })
        .collect::<Vec<_>>();

    for (target, location) in command_methods.into_iter().zip(["L7", "L8", "L11"]) {
        assert!(resolved.edges.iter().any(|edge| {
            edge.string("relation") == "calls"
                && edge.target == target
                && edge.string("source_location") == location
        }));
    }
}

#[test]
fn go_multi_return_range_inside_closure_preserves_element_method_attribution() {
    let go_source = br#"package pkg
type Command struct{}
func (*Command) Find(args []string) (*Command, []string, error) { return nil, nil, nil }
func (*Command) Commands() []*Command { return nil }
func (*Command) IsAvailableCommand() bool { return true }
func (*Command) Name() string { return "" }
func caller(command *Command) {
    visit := func() {
        cmd, _, _ := command.Find(nil)
        for _, subCommand := range cmd.Commands() {
            if subCommand.IsAvailableCommand() {
                _ = subCommand.Name()
            }
        }
    }
    _ = visit
}
"#;
    let extracted = extract("pkg/caller.go", go_source);
    let sources = HashMap::from([(
        "pkg/caller.go".to_owned(),
        String::from_utf8(go_source.to_vec()).expect("source"),
    )]);
    let resolved = compass_resolve::resolve(&[extracted], &sources);
    let command_methods = ["Find", "Commands", "IsAvailableCommand", "Name"]
        .into_iter()
        .map(|method| {
            resolved
                .nodes
                .iter()
                .find(|node| node.string("qualified_name") == format!("pkg.Command::{method}"))
                .unwrap_or_else(|| panic!("Command.{method} declaration"))
                .id
                .clone()
        })
        .collect::<Vec<_>>();

    for (target, location) in command_methods.into_iter().zip(["L9", "L10", "L11", "L12"]) {
        assert!(
            resolved.edges.iter().any(|edge| {
                edge.string("relation") == "calls"
                    && edge.target == target
                    && edge.string("source_location") == location
            }),
            "missing Go call at {location}"
        );
    }
}

#[test]
fn go_cobra_shape_preserves_nested_closure_and_multi_return_method_attribution() {
    let go_source = br#"package pkg
type ShellCompDirective int
type Command struct { helpCommand *Command }
func (*Command) Root() *Command { return nil }
func (*Command) Find(args []string) (*Command, []string, error) { return nil, nil, nil }
func (*Command) Commands() []*Command { return nil }
func (*Command) IsAvailableCommand() bool { return true }
func (*Command) Name() string { return "" }
func (c *Command) initDefaultHelpCmd() {
    c.helpCommand = &Command{}
    c.helpCommand.ValidArgsFunction = func(cmd *Command, args []string, toComplete string) ([]string, ShellCompDirective) {
        cmd, _, _ := c.Root().Find(args)
        for _, subCmd := range cmd.Commands() {
            if subCmd.IsAvailableCommand() || subCmd == cmd.helpCommand {
                _ = subCmd.Name()
            }
        }
        return nil, 0
    }
}
"#;
    let extracted = extract("pkg/command.go", go_source);
    let sources = HashMap::from([(
        "pkg/command.go".to_owned(),
        String::from_utf8(go_source.to_vec()).expect("source"),
    )]);
    let resolved = compass_resolve::resolve(&[extracted], &sources);
    let command_methods = ["Root", "Find", "Commands", "IsAvailableCommand", "Name"]
        .into_iter()
        .map(|method| {
            resolved
                .nodes
                .iter()
                .find(|node| node.string("qualified_name") == format!("pkg.Command::{method}"))
                .unwrap_or_else(|| panic!("Command.{method} declaration"))
                .id
                .clone()
        })
        .collect::<Vec<_>>();

    for (target, location) in command_methods
        .into_iter()
        .zip(["L12", "L12", "L13", "L14", "L15"])
    {
        assert!(
            resolved.edges.iter().any(|edge| {
                edge.string("relation") == "calls"
                    && edge.target == target
                    && edge.string("source_location") == location
            }),
            "missing Cobra-shaped Go call at {location}"
        );
    }
}

#[test]
fn go_cobra_exact_find_guard_preserves_range_element_method_attribution() {
    let go_source = br#"package pkg
import "strings"
type ShellCompDirective int
type Completion struct{}
type Command struct {
    helpCommand *Command
    ValidArgsFunction func(*Command, []string, string) ([]Completion, ShellCompDirective)
    Short string
}
func (*Command) Root() *Command { return nil }
func (*Command) Find(args []string) (*Command, []string, error) { return nil, nil, nil }
func (*Command) Commands() []*Command { return nil }
func (*Command) IsAvailableCommand() bool { return true }
func (*Command) Name() string { return "" }
func (*Command) HasSubCommands() bool { return true }
func CompletionWithDesc(choice string, description string) Completion { return Completion{} }
func (c *Command) initDefaultHelpCmd() {
    if !c.HasSubCommands() { return }
    if c.helpCommand == nil {
        c.helpCommand = &Command{
            ValidArgsFunction: func(c *Command, args []string, toComplete string) ([]Completion, ShellCompDirective) {
                var completions []Completion
                cmd, _, e := c.Root().Find(args)
                if e != nil { return nil, 0 }
                if cmd == nil { cmd = c.Root() }
                for _, subCmd := range cmd.Commands() {
                    if subCmd.IsAvailableCommand() || subCmd == cmd.helpCommand {
                        if strings.HasPrefix(subCmd.Name(), toComplete) {
                            completions = append(completions, CompletionWithDesc(subCmd.Name(), subCmd.Short))
                        }
                    }
                }
                return completions, 0
            },
        }
    }
}
"#;
    let extracted = extract("pkg/command.go", go_source);
    let sources = HashMap::from([(
        "pkg/command.go".to_owned(),
        String::from_utf8(go_source.to_vec()).expect("source"),
    )]);
    let resolved = compass_resolve::resolve(&[extracted], &sources);
    for method in ["Root", "Find", "Commands", "IsAvailableCommand", "Name"] {
        let target = resolved
            .nodes
            .iter()
            .find(|node| node.string("qualified_name") == format!("pkg.Command::{method}"))
            .unwrap_or_else(|| panic!("Command.{method} declaration"))
            .id
            .clone();
        assert!(
            resolved
                .edges
                .iter()
                .any(|edge| { edge.string("relation") == "calls" && edge.target == target }),
            "missing Cobra exact-shape Go call to {method}"
        );
    }
}

#[test]
fn go_for_clause_initializers_and_empty_closures_keep_exact_attribution() {
    let go_source = br#"package pkg
type Worker struct{}
func (worker *Worker) Run() {}
func (worker *Worker) Parent() *Worker { return nil }
func caller(worker *Worker) {
    for current := worker; current != nil; current = current.Parent() {
        current.Run()
    }
    visit := func() {
        worker.Run()
    }
    _ = visit
}
"#;
    let extracted = extract("pkg/caller.go", go_source);
    let evidence = extracted
        .semantic_evidence
        .clone()
        .expect("Go universal evidence");
    let sources = HashMap::from([(
        "pkg/caller.go".to_owned(),
        String::from_utf8(go_source.to_vec()).expect("source"),
    )]);
    let resolved = compass_resolve::resolve(&[extracted], &sources);
    let worker_run = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "pkg.Worker::Run")
        .expect("Worker.Run declaration");
    let worker_parent = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "pkg.Worker::Parent")
        .expect("Worker.Parent declaration");

    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.target == worker_parent.id
            && edge.string("source_location") == "L6"
    }));
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.target == worker_run.id
            && edge.string("source_location") == "L7"
    }));
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.target == worker_run.id
            && edge.string("source_location") == "L10"
    }));

    let closure_scope_ids = evidence
        .scopes
        .iter()
        .filter(|scope| scope.kind == "closure")
        .map(|scope| scope.id.as_str())
        .collect::<HashSet<_>>();
    let empty_closure_call = evidence
        .occurrences
        .iter()
        .find(|occurrence| occurrence.range.start_line == 10 && occurrence.spelling == "Run")
        .expect("empty closure call occurrence");
    assert!(
        empty_closure_call
            .scope_id
            .as_deref()
            .is_some_and(|scope_id| closure_scope_ids.contains(scope_id)),
        "empty closures must own their call occurrences"
    );
}

#[test]
fn go_variadic_declarations_use_source_arity_for_calls() {
    let go_source = br#"package pkg
type Worker struct{}
func fanout(workers ...*Worker) {}
func caller(worker *Worker) {
    fanout(worker, worker)
}
"#;
    let extracted = extract("pkg/caller.go", go_source);
    let evidence = extracted
        .semantic_evidence
        .clone()
        .expect("Go universal evidence");
    let fanout = evidence
        .declarations
        .iter()
        .find(|declaration| declaration.name == "fanout")
        .expect("variadic declaration");
    assert_eq!(fanout.parameter_count, Some(1));
    assert!(fanout.variadic);

    let sources = HashMap::from([(
        "pkg/caller.go".to_owned(),
        String::from_utf8(go_source.to_vec()).expect("source"),
    )]);
    let resolved = compass_resolve::resolve(&[extracted], &sources);
    let fanout_node = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "pkg.fanout")
        .expect("fanout node");
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.target == fanout_node.id
            && edge.string("source_location") == "L5"
    }));
}

#[test]
fn go_module_imports_resolve_exported_functions_by_exact_source_directory() {
    let provider = extract(
        "cmd/entire/cli/trailers/trailers.go",
        b"package trailers\nfunc ParseMetadata(value string) {}\n",
    );
    let caller_source = br#"package checkpoint
import "github.com/entireio/cli/cmd/entire/cli/trailers"
func Load(value string) {
    visit := func() {
        trailers.ParseMetadata(value)
    }
    visit()
}
"#;
    let caller = extract("cmd/entire/cli/checkpoint/load.go", caller_source);
    let sources = HashMap::from([(
        "cmd/entire/cli/checkpoint/load.go".to_owned(),
        String::from_utf8(caller_source.to_vec()).expect("source"),
    )]);
    let resolved =
        compass_resolve::resolve_with_root(&[provider, caller], &sources, Path::new("."));
    let target = resolved
        .nodes
        .iter()
        .find(|node| node.label() == "ParseMetadata()")
        .expect("provider function");

    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "calls"
            && edge.target == target.id
            && edge.string("source_location") == "L5"
    }));
}

#[test]
fn go_packages_with_the_same_terminal_directory_keep_distinct_alias_owners() {
    let provider = extract(
        "api/checkpoint/metadata.go",
        b"package checkpoint\ntype Summary struct{}\n",
    );
    let alias = extract(
        "cmd/entire/cli/checkpoint/aliases.go",
        br#"package checkpoint
import api "github.com/example/project/api/checkpoint"
type Summary = api.Summary
"#,
    );
    let consumer_source = b"package checkpoint\nfunc Read() *Summary { return nil }\n";
    let consumer = extract("cmd/entire/cli/checkpoint/reader.go", consumer_source);
    let sources = HashMap::from([(
        "cmd/entire/cli/checkpoint/reader.go".to_owned(),
        String::from_utf8(consumer_source.to_vec()).expect("source"),
    )]);
    let resolved =
        compass_resolve::resolve_with_root(&[provider, alias, consumer], &sources, Path::new("."));
    let provider = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "api/checkpoint.Summary")
        .expect("provider Summary");
    let alias = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "cmd/entire/cli/checkpoint.Summary")
        .expect("local Summary alias");

    assert_ne!(provider.id, alias.id);
    assert!(resolved.edges.iter().any(|edge| {
        edge.string("relation") == "returns"
            && edge.target == alias.id
            && edge.string("source_file") == "cmd/entire/cli/checkpoint/reader.go"
            && edge.string("source_location") == "L2"
    }));
}

#[test]
fn go_selector_chains_use_declared_direct_field_types() {
    let types = br#"package generated
type Schema struct{}
type Body struct {
    Value Schema
    Pointer *Schema
    Many []Schema
}
"#;
    let methods = br#"package generated
func (schema *Schema) Encode() {}
func (body *Body) Encode() {
    body.Value.Encode()
    body.Pointer.Encode()
    body.Many.Encode()
}
"#;
    let types = extract("generated/types.go", types);
    let methods_extraction = extract("generated/methods.go", methods);
    let sources = HashMap::from([(
        "generated/methods.go".to_owned(),
        String::from_utf8(methods.to_vec()).expect("source"),
    )]);
    let resolved = compass_resolve::resolve(&[types, methods_extraction], &sources);
    let schema_encode = resolved
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "generated.Schema::Encode")
        .expect("Schema.Encode declaration");
    let call_sites = resolved
        .edges
        .iter()
        .filter(|edge| edge.string("relation") == "calls" && edge.target == schema_encode.id)
        .map(|edge| edge.string("source_location"))
        .collect::<std::collections::BTreeSet<_>>();

    assert_eq!(
        call_sites,
        std::collections::BTreeSet::from(["L4".to_owned(), "L5".to_owned()])
    );
    assert!(resolved.edges.iter().all(|edge| {
        edge.string("relation") != "calls" || edge.string("source_location") != "L6"
    }));
}
