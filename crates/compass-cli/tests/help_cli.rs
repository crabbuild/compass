use std::ffi::{OsStr, OsString};

use compass_cli::{Frontend, HelpStyle, run};

fn invoke(arguments: &[&str]) -> compass_cli::Outcome {
    run(
        Frontend::Compass,
        arguments.iter().map(|argument| OsString::from(*argument)),
    )
}

#[test]
fn root_help_groups_every_public_command_with_descriptions() {
    let outcome = invoke(&["--help"]);
    assert_eq!(outcome.code, 0);
    assert!(outcome.stderr.is_empty());
    for heading in [
        "Build and maintain:",
        "Explore:",
        "History:",
        "Visualize and export:",
        "Integrate and automate:",
        "Diagnose and support:",
    ] {
        assert!(outcome.stdout.contains(heading), "missing {heading}");
    }
    for command in [
        "update",
        "extract",
        "watch",
        "cluster-only",
        "label",
        "merge-graphs",
        "cache-check",
        "merge-chunks",
        "merge-semantic",
        "query",
        "path",
        "explain",
        "affected",
        "benchmark",
        "history",
        "diff",
        "tree",
        "export",
        "serve",
        "global",
        "clone",
        "add",
        "prs",
        "hook",
        "install",
        "uninstall",
        "provider",
        "save-result",
        "reflect",
        "diagnose",
        "check-update",
        "merge-driver",
        "hook-check",
        "hook-guard",
    ] {
        let line = outcome
            .stdout
            .lines()
            .find(|line| line.trim_start().starts_with(command));
        assert!(line.is_some(), "missing command {command}");
        let Some(line) = line else {
            continue;
        };
        assert_ne!(line.trim(), command, "missing summary for {command}");
    }
}

#[test]
fn command_and_nested_help_explain_options_and_examples() {
    for arguments in [
        &["update", "--help"][..],
        &["query", "--help"],
        &["history", "build", "--help"],
        &["export", "neo4j", "--help"],
        &["provider", "add", "--help"],
        &["diagnose", "multigraph", "--help"],
    ] {
        let outcome = invoke(arguments);
        assert_eq!(outcome.code, 0, "{}", outcome.stderr);
        assert!(outcome.stdout.contains("Usage:"), "{arguments:?}");
        assert!(outcome.stdout.contains("Options:"), "{arguments:?}");
        assert!(outcome.stdout.contains("Examples:"), "{arguments:?}");
        assert!(outcome.stdout.contains("-h, --help"), "{arguments:?}");
        assert!(!outcome.stdout.contains('\x1b'), "{arguments:?}");
    }

    let flag = invoke(&["history", "build", "--help"]);
    let command = invoke(&["help", "history", "build"]);
    assert_eq!(flag.stdout, command.stdout);
}

#[test]
fn every_public_nested_command_has_a_dedicated_page() {
    for (parent, children) in [
        (
            "history",
            &[
                "enable", "disable", "status", "build", "rebuild", "list", "show", "prefer",
                "export", "gc",
            ][..],
        ),
        (
            "export",
            &[
                "html",
                "callflow-html",
                "obsidian",
                "wiki",
                "svg",
                "graphml",
                "neo4j",
                "falkordb",
            ],
        ),
        ("provider", &["add", "list", "show", "remove"]),
        ("global", &["add", "remove", "list", "path"]),
        ("hook", &["install", "uninstall", "status"]),
        ("diagnose", &["multigraph"]),
    ] {
        for child in children {
            let outcome = invoke(&[parent, child, "--help"]);
            assert_eq!(outcome.code, 0, "{parent} {child}: {}", outcome.stderr);
            assert!(
                outcome
                    .stdout
                    .contains(&format!("compass {parent} {child}")),
                "{parent} {child}"
            );
            assert!(outcome.stdout.contains("Examples:"), "{parent} {child}");
        }
    }
}

#[test]
fn help_suggestions_and_terminal_policy_are_conservative() {
    let typo = invoke(&["udpate"]);
    assert_eq!(typo.code, 1);
    assert!(typo.stderr.contains("Did you mean 'update'?"));

    let distant = invoke(&["bananas"]);
    assert_eq!(distant.code, 1);
    assert!(!distant.stderr.contains("Did you mean"));

    let nested = invoke(&["help", "history", "buidl"]);
    assert_eq!(nested.code, 2);
    assert!(nested.stderr.contains("Did you mean 'build'?"));

    assert_eq!(HelpStyle::detect(false, None, None), HelpStyle::Plain);
    assert_eq!(
        HelpStyle::detect(true, Some(OsStr::new("")), None),
        HelpStyle::Plain
    );
    assert_eq!(
        HelpStyle::detect(true, None, Some(OsStr::new("dumb"))),
        HelpStyle::Plain
    );
    assert_eq!(HelpStyle::detect(true, None, None), HelpStyle::Ansi);
}

#[test]
fn compatibility_help_flags_use_the_rich_renderer() {
    let root = invoke(&["--help"]);
    for arguments in [&["-?"][..], &["help", "--help"]] {
        let outcome = invoke(arguments);
        assert_eq!(outcome.code, 0, "{arguments:?}: {}", outcome.stderr);
        assert_eq!(outcome.stdout, root.stdout, "{arguments:?}");
    }

    let query = invoke(&["query", "-?"]);
    assert_eq!(query.code, 0, "{}", query.stderr);
    assert!(query.stdout.contains("Search the graph"));
    assert!(query.stdout.contains("Examples:"));
}

#[test]
fn graphify_help_asset_remains_byte_for_byte_unchanged() {
    let outcome = run(Frontend::Graphify, [OsString::from("--help")]);
    assert_eq!(outcome.code, 0);
    assert_eq!(outcome.stdout, include_str!("../assets/graphify-help.txt"));
}
