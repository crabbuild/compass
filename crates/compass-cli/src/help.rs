use std::ffi::{OsStr, OsString};

use crate::Outcome;

const WIDTH: usize = 92;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HelpStyle {
    Plain,
    Ansi,
}

impl HelpStyle {
    #[must_use]
    pub fn detect(is_terminal: bool, no_color: Option<&OsStr>, term: Option<&OsStr>) -> Self {
        if !is_terminal
            || no_color.is_some()
            || term.is_some_and(|value| value.to_string_lossy().eq_ignore_ascii_case("dumb"))
        {
            Self::Plain
        } else {
            Self::Ansi
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Visibility {
    Public,
    Internal,
}

#[derive(Clone, Copy)]
struct Page {
    path: &'static str,
    summary: &'static str,
    usage: &'static [&'static str],
    details: &'static str,
    visibility: Visibility,
}

macro_rules! page {
    ($path:literal, $summary:literal, [$($usage:literal),+ $(,)?], $details:literal) => {
        Page {
            path: $path,
            summary: $summary,
            usage: &[$($usage),+],
            details: $details,
            visibility: Visibility::Public,
        }
    };
}

macro_rules! internal_page {
    ($path:literal, $summary:literal, [$($usage:literal),+ $(,)?], $details:literal) => {
        Page {
            path: $path,
            summary: $summary,
            usage: &[$($usage),+],
            details: $details,
            visibility: Visibility::Internal,
        }
    };
}

struct Group {
    title: &'static str,
    commands: &'static [&'static str],
}

const GROUPS: &[Group] = &[
    Group {
        title: "Build and maintain",
        commands: &[
            "update",
            "extract",
            "watch",
            "cluster-only",
            "label",
            "merge-graphs",
            "cache-check",
            "merge-chunks",
            "merge-semantic",
        ],
    },
    Group {
        title: "Explore",
        commands: &["query", "path", "explain", "affected", "benchmark"],
    },
    Group {
        title: "History",
        commands: &["history", "diff"],
    },
    Group {
        title: "Visualize and export",
        commands: &["tree", "export"],
    },
    Group {
        title: "Integrate and automate",
        commands: &[
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
        ],
    },
    Group {
        title: "Diagnose and support",
        commands: &[
            "diagnose",
            "check-update",
            "merge-driver",
            "hook-check",
            "hook-guard",
        ],
    },
];

const PAGES: &[Page] = &[
    page!(
        "update",
        "Incrementally refresh the local knowledge graph",
        ["compass update [PATH] [OPTIONS]"],
        "Arguments:\n  [PATH]                       Project directory to scan [default: saved root or .]\n\nOptions:\n  --program-artifact <PATH>    Add an offline program-evidence artifact; repeatable\n  --out <DIR>                  Write artifacts below this directory\n  --force                      Rebuild even when inputs appear unchanged\n  --no-cluster                 Skip community detection\n  --no-viz                     Skip graph.html generation\n  --no-gitignore               Ignore .gitignore rules while scanning\n  --exclude <PATTERN>          Exclude a glob pattern; repeatable\n  --resolution <NUMBER>        Community-detection resolution [default: 1.0]\n  --exclude-hubs <NUMBER>      Exclude high-degree hubs from clustering\n\nExamples:\n  compass update\n  compass update ./services/api --force\n  compass update --program-artifact index.scip\n\nTips:\n  Use `compass watch` to refresh the graph whenever project files change."
    ),
    page!(
        "extract",
        "Build a graph with optional semantic sources and model enrichment",
        [
            "compass extract [PATH] [OPTIONS]",
            "compass extract --postgres <DSN> [OPTIONS]"
        ],
        "Arguments:\n  [PATH]                       Project directory to scan\n\nOptions:\n  --program-artifact <PATH>    Add an offline program-evidence artifact; repeatable\n  --code-only                  Extract structural code without semantic sources\n  --cargo                      Include Cargo metadata\n  --google-workspace           Include Google Workspace shortcuts\n  --postgres <DSN>             Extract PostgreSQL schema objects\n  --backend <NAME>             Semantic provider name\n  --model <MODEL>              Override the provider's default model\n  --mode <deep>                Use deep semantic extraction\n  --token-budget <N>           Maximum semantic token budget\n  --max-concurrency <N>        Maximum concurrent provider requests\n  --max-workers <N>            Maximum local extraction workers\n  --api-timeout <SECONDS>      Provider request timeout\n  --allow-partial              Publish results when semantic chunks fail\n  --dedup-llm                  Use the model to review likely duplicates\n  --timing                     Print stage timings\n  --global                     Merge the completed graph into the global graph\n  --as <TAG>                   Repository tag used with --global\n  --out <DIR>                  Output directory\n  --force                      Rebuild unchanged inputs\n  --no-cluster                 Skip community detection\n  --no-viz                     Skip graph.html generation\n  --no-gitignore               Ignore .gitignore rules\n  --exclude <PATTERN>          Exclude a glob pattern; repeatable\n  --resolution <NUMBER>        Community resolution [default: 1.0]\n  --exclude-hubs <NUMBER>      Exclude high-degree clustering hubs\n\nExamples:\n  compass extract ./project --code-only\n  compass extract ./project --program-artifact index.scip --code-only\n  compass extract --postgres \"postgresql://localhost/app\" --code-only\n\nNotes:\n  Semantic extraction may require credentials for the selected provider."
    ),
    page!(
        "watch",
        "Rebuild the graph automatically when project files change",
        ["compass watch [PATH] [OPTIONS]"],
        "Arguments:\n  [PATH]                       Project directory to watch [default: .]\n\nOptions:\n  --program-artifact <PATH>    Add an offline program-evidence artifact; repeatable\n  --debounce <SECONDS>         Wait after changes before rebuilding [default: 3]\n  --out <DIR>                  Output directory\n  --no-cluster                 Skip community detection\n  --no-viz                     Skip graph.html generation\n  --no-gitignore               Ignore .gitignore rules\n  --exclude <PATTERN>          Exclude a glob pattern; repeatable\n  --poll                       Poll instead of using filesystem notifications\n\nExamples:\n  compass watch\n  compass watch ./services/api --program-artifact index.scip --poll"
    ),
    page!(
        "cluster-only",
        "Recompute graph communities without re-extracting source files",
        ["compass cluster-only [PATH] [OPTIONS]"],
        "Arguments:\n  [PATH]                       Project or output directory [default: .]\n\nOptions:\n  --graph <PATH>               Graph JSON path\n  --no-viz                     Skip graph.html generation\n  --no-label                   Skip model-generated community labels\n  --missing-only               Compatibility flag; existing labels are preserved\n  --timing                     Print clustering timings\n  --resolution <NUMBER>        Community resolution [default: 1.0]\n  --exclude-hubs <NUMBER>      Exclude high-degree hubs\n  --min-community-size=<N>     Minimum community size [default: 3]\n  --backend <NAME>             Accepted compatibility provider setting\n  --model <MODEL>              Accepted compatibility model setting\n  --max-concurrency <N>        Accepted compatibility request limit\n  --batch-size <N>             Accepted compatibility batch size\n\nExamples:\n  compass cluster-only\n  compass cluster-only --graph compass-out/graph.json --no-label --timing"
    ),
    page!(
        "label",
        "Generate readable labels for graph communities",
        ["compass label [PATH] [OPTIONS]"],
        "Arguments:\n  [PATH]                       Project or output directory [default: .]\n\nOptions:\n  --graph <PATH>               Graph JSON path\n  --backend <NAME>             Semantic provider\n  --model <MODEL>              Provider model\n  --missing-only               Preserve existing labels\n  --no-viz                     Skip graph.html regeneration\n  --resolution <NUMBER>        Community resolution [default: 1.0]\n  --exclude-hubs <NUMBER>      Exclude high-degree hubs\n  --max-concurrency <N>        Concurrent provider requests\n  --batch-size <N>             Communities per request\n  --min-community-size=<N>     Minimum community size\n  --timing                     Print stage timings\n\nExamples:\n  compass label\n  compass label --backend gemini --missing-only --batch-size 8"
    ),
    page!(
        "merge-graphs",
        "Combine two or more graph JSON files",
        ["compass merge-graphs <GRAPH> <GRAPH> [GRAPH...] [OPTIONS]"],
        "Arguments:\n  <GRAPH>                 Input graph; provide at least two\n\nOptions:\n  --out <PATH>            Merged graph path [default: compass-out/merged-graph.json]\n\nExamples:\n  compass merge-graphs api.json web.json\n  compass merge-graphs api.json web.json jobs.json --out system.json"
    ),
    page!(
        "cache-check",
        "Inspect cached semantic extraction files",
        ["compass cache-check <FILES_FROM> [OPTIONS]"],
        "Arguments:\n  <FILES_FROM>             File containing source paths to inspect\n\nOptions:\n  --root <DIR>             Resolve source paths below this directory\n  --mode <MODE>            Semantic extraction mode\n  --deep                   Select deep semantic mode\n  --prompt-file <PATH>     Use a custom extraction prompt\n\nExamples:\n  compass cache-check files.txt\n  compass cache-check files.txt --root . --deep"
    ),
    page!(
        "merge-chunks",
        "Merge semantic chunk files into one result",
        ["compass merge-chunks <CHUNK_FILES...> --out <PATH>"],
        "Arguments:\n  <CHUNK_FILES...>        Semantic chunk JSON files\n\nOptions:\n  --out <PATH>            Required merged output path\n\nExamples:\n  compass merge-chunks chunks/*.json --out semantic.json"
    ),
    page!(
        "merge-semantic",
        "Merge cached and newly extracted semantic results",
        ["compass merge-semantic --cached <PATH> --new <PATH> --out <PATH>"],
        "Options:\n  --cached <PATH>         Existing semantic result\n  --new <PATH>            Newly extracted semantic result\n  --out <PATH>            Required merged output path\n\nExamples:\n  compass merge-semantic --cached old.json --new fresh.json --out semantic.json"
    ),
    page!(
        "query",
        "Search the graph with natural language or CompassQL",
        [
            "compass query <QUESTION> [OPTIONS]",
            "compass query --cql <QUERY> [OPTIONS]",
            "compass query --cql --file <PATH> [OPTIONS]",
            "compass query --cql --stdin",
            "compass query --cql --repl"
        ],
        "Arguments:\n  <QUESTION>                      Natural-language graph question\n  <QUERY>                         Inline CompassQL query\n\nOptions:\n  --dfs                           Use depth-first traversal\n  --context <VALUE>               Add query context\n  --budget <N>                    Limit returned context\n  --graph <PATH>                  Read a graph JSON file\n  --at <REV>                      Query an immutable Git revision; conflicts with --graph\n  --cql                           Use CompassQL mode\n  --file <PATH>                   Read CompassQL from a file\n  --stdin                         Read CompassQL from standard input\n  --repl                          Start the interactive CompassQL shell\n  --param <NAME=VALUE>            Bind a parameter; repeatable\n  --params-file <PATH>            Read parameters from JSON\n  --format <table|json|jsonl>     Result format [default: table]\n  --output <PATH>                 Write results to a file\n  --timeout-ms <N>                Execution timeout [default: 5000]\n  --max-rows <N>                  Row limit [default: 10000]\n  --max-path-depth <N>            Path-depth limit [default: 32]\n  --max-expanded-relationships <N> Relationship expansion limit [default: 5000000]\n  --max-memory-bytes <N>          Memory limit [default: 268435456]\n\nExamples:\n  compass query \"authentication flow\"\n  compass query \"payment service\" --at HEAD~10\n  compass query --cql \"MATCH (n) RETURN n LIMIT 10\" --format json\n  compass query --cql --file report.cql --params-file params.json\n\nTips:\n  Use `compass path` when you know both endpoints of the relationship to trace."
    ),
    page!(
        "path",
        "Find the shortest relationship path between two graph nodes",
        ["compass path <SOURCE> <TARGET> [OPTIONS]"],
        "Arguments:\n  <SOURCE>                Source node name or identifier\n  <TARGET>                Target node name or identifier\n\nOptions:\n  --graph <PATH>          Read a graph JSON file\n  --at <REV>              Use an immutable Git revision; conflicts with --graph\n\nExamples:\n  compass path CheckoutHandler PaymentGateway\n  compass path api route --at v1.2.0"
    ),
    page!(
        "explain",
        "Explain a node and its important relationships",
        ["compass explain <NODE> [OPTIONS]"],
        "Arguments:\n  <NODE>                  Node name, label, or identifier\n\nOptions:\n  --graph <PATH>          Read a graph JSON file\n  --at <REV>              Use an immutable Git revision; conflicts with --graph\n\nExamples:\n  compass explain PaymentService\n  compass explain auth --at HEAD~5"
    ),
    page!(
        "affected",
        "Find code reachable from a proposed change",
        ["compass affected <NODE_OR_LABEL> [OPTIONS]"],
        "Arguments:\n  <NODE_OR_LABEL>         Starting node or community label\n\nOptions:\n  --relation <RELATION>   Follow one relationship type; repeatable\n  --depth <N>             Maximum traversal depth\n  --graph <PATH>          Read a graph JSON file\n\nExamples:\n  compass affected PaymentGateway\n  compass affected auth --relation CALLS --depth 3"
    ),
    page!(
        "benchmark",
        "Measure graph query characteristics",
        ["compass benchmark [GRAPH_JSON]"],
        "Arguments:\n  [GRAPH_JSON]            Graph to benchmark [default: compass-out/graph.json]\n\nExamples:\n  compass benchmark\n  compass benchmark fixtures/large-graph.json"
    ),
    page!(
        "history",
        "Manage immutable graphs for Git revisions",
        ["compass history <COMMAND>"],
        "Examples:\n  compass history enable\n  compass history build HEAD\n  compass history status HEAD\n\nTips:\n  Run `compass help history <command>` for revision, format, and safety details."
    ),
    page!(
        "history enable",
        "Enable eager graph history for future commits",
        ["compass history enable [BUILD_PROFILE_OPTIONS]"],
        "Options:\n  --backend <NAME>         Pin a semantic provider\n  --model <MODEL>          Pin the provider model\n  --mode <deep>            Pin deep semantic extraction\n  --cargo                  Include Cargo metadata\n  --dedup-llm              Enable model-assisted deduplication\n  --token-budget <N>       Pin the semantic token budget\n  --resolution <NUMBER>    Pin community resolution [default: 1.0]\n  --exclude-hubs <NUMBER>  Pin the hub-exclusion threshold\n  --no-gitignore           Ignore committed .gitignore rules\n  --exclude <PATTERN>      Pin an exclusion glob; repeatable\n\nExamples:\n  compass history enable\n  compass history enable --cargo --backend gemini --model gemini-2.5-flash\n\nNotes:\n  This installs managed post-commit and post-merge hooks."
    ),
    page!(
        "history disable",
        "Stop eager history builds while retaining stored graphs",
        ["compass history disable"],
        "Examples:\n  compass history disable\n\nNotes:\n  Explicit builds, historical queries, and existing realizations remain available."
    ),
    page!(
        "history status",
        "Show history configuration and realization status",
        ["compass history status [REV] [OPTIONS]"],
        "Arguments:\n  [REV]                   Git revision [default: HEAD]\n\nOptions:\n  --format <text|json>    Output format [default: text]\n\nExamples:\n  compass history status\n  compass history status HEAD~10 --format json"
    ),
    page!(
        "history build",
        "Materialize an immutable graph for a Git revision",
        ["compass history build <REV> [BUILD_PROFILE_OPTIONS] [OPTIONS]"],
        "Arguments:\n  <REV>                    Git revision to materialize\n\nOptions:\n  --profile-from <SOURCE>  Reuse a profile from a revision or realization\n  --format <text|json>      Output format [default: text]\n  --backend <NAME>          Pin a semantic provider\n  --model <MODEL>           Pin the provider model\n  --mode <deep>             Use deep semantic extraction\n  --cargo                   Include Cargo metadata\n  --dedup-llm               Enable model-assisted deduplication\n  --token-budget <N>        Semantic token budget\n  --resolution <NUMBER>     Community resolution [default: 1.0]\n  --exclude-hubs <NUMBER>   Hub-exclusion threshold\n  --no-gitignore            Ignore committed .gitignore rules\n  --exclude <PATTERN>       Exclusion glob; repeatable\n\nExamples:\n  compass history build HEAD\n  compass history build v1.2.0 --profile-from HEAD --format json\n\nNotes:\n  --profile-from conflicts with build-profile options."
    ),
    page!(
        "history rebuild",
        "Build a new realization for an already materialized revision",
        ["compass history rebuild <REV> [BUILD_PROFILE_OPTIONS] [OPTIONS]"],
        "Arguments:\n  <REV>                    Git revision to rebuild\n\nOptions:\n  --replace-corrupt        Replace an unreadable preferred pointer\n  --format <text|json>      Output format [default: text]\n  --backend <NAME>          Pin a semantic provider\n  --model <MODEL>           Pin the provider model\n  --mode <deep>             Use deep semantic extraction\n  --cargo                   Include Cargo metadata\n  --dedup-llm               Enable model-assisted deduplication\n  --token-budget <N>        Semantic token budget\n  --resolution <NUMBER>     Community resolution [default: 1.0]\n  --exclude-hubs <NUMBER>   Hub-exclusion threshold\n  --no-gitignore            Ignore committed .gitignore rules\n  --exclude <PATTERN>       Exclusion glob; repeatable\n\nExamples:\n  compass history rebuild HEAD\n  compass history rebuild HEAD --replace-corrupt\n\nNotes:\n  Use --replace-corrupt only after status reports an unreadable preferred realization."
    ),
    page!(
        "history list",
        "List stored graph realizations",
        ["compass history list [REV] [OPTIONS]"],
        "Arguments:\n  [REV]                   Limit results to one Git revision\n\nOptions:\n  --format <text|json>    Output format [default: text]\n\nExamples:\n  compass history list\n  compass history list HEAD --format json"
    ),
    page!(
        "history show",
        "Show metadata for one graph realization",
        ["compass history show <REALIZATION> [OPTIONS]"],
        "Arguments:\n  <REALIZATION>           Realization identifier\n\nOptions:\n  --format <text|json>    Output format [default: text]\n\nExamples:\n  compass history show 0123456789abcdef\n  compass history show 0123456789abcdef --format json"
    ),
    page!(
        "history prefer",
        "Select the preferred realization for a revision",
        ["compass history prefer <REV> <REALIZATION> [OPTIONS]"],
        "Arguments:\n  <REV>                   Git revision\n  <REALIZATION>           Validated realization identifier\n\nOptions:\n  --format <text|json>    Output format [default: text]\n\nExamples:\n  compass history prefer HEAD 0123456789abcdef\n\nNotes:\n  Compass validates the realization before changing the preferred pointer."
    ),
    page!(
        "history export",
        "Restore a historical graph or Compass output bundle",
        ["compass history export <REV> --format <FORMAT> --output <PATH>"],
        "Arguments:\n  <REV>                         Git revision\n\nOptions:\n  --format <graph-json|compass-out> Required export format\n  --output <PATH>                Required destination\n\nExamples:\n  compass history export HEAD~10 --format graph-json --output old.json\n  compass history export v1.2.0 --format compass-out --output release-graph\n\nNotes:\n  A compass-out destination must not already exist."
    ),
    page!(
        "history gc",
        "Inspect or reclaim unreachable history storage",
        ["compass history gc [OPTIONS]"],
        "Options:\n  --prune-non-preferred    Include alternate realizations in the plan\n  --yes                    Apply non-preferred pruning; requires --prune-non-preferred\n  --format <text|json>     Output format [default: text]\n\nExamples:\n  compass history gc\n  compass history gc --prune-non-preferred\n  compass history gc --prune-non-preferred --yes\n\nNotes:\n  Non-preferred pruning is a dry run until repeated with --yes."
    ),
    page!(
        "diff",
        "Compare knowledge graphs from two Git revisions",
        ["compass diff <OLD> <NEW> [OPTIONS]"],
        "Arguments:\n  <OLD>                         Base Git revision\n  <NEW>                         Target Git revision\n\nOptions:\n  --detailed                    Include node and relationship details\n  --format <text|json>          Output format [default: text]\n  --topology-only               Compare only graph topology\n  --include-locations           Include source-location changes\n  --include-analysis            Include analysis artifact changes\n  --include-metadata            Include graph metadata changes\n  --fingerprint <SHA256>        Select one extraction fingerprint\n  --allow-profile-mismatch      Compare different extraction profiles\n\nExamples:\n  compass diff v1.2.0 HEAD\n  compass diff HEAD~1 HEAD --detailed --include-locations\n  compass diff main feature --format json --allow-profile-mismatch\n\nNotes:\n  --fingerprint conflicts with --allow-profile-mismatch; --detailed conflicts with JSON output."
    ),
    page!(
        "tree",
        "Generate an interactive filesystem and symbol tree",
        ["compass tree [OPTIONS]"],
        "Options:\n  --graph <PATH>          Graph JSON [default: compass-out/graph.json]\n  --output <HTML>         Output page [default: compass-out/GRAPH_TREE.html]\n  --root <PATH>           Filesystem root shown in the tree\n  --max-children <N>      Visible children per node [default: 200]\n  --top-k-edges <N>       Outbound edges per symbol [default: 12]\n  --label <NAME>          Project label shown in the header\n\nExamples:\n  compass tree\n  compass tree --root ./src --max-children 100"
    ),
    page!(
        "export",
        "Export or publish the graph in another format",
        ["compass export <FORMAT> [OPTIONS]"],
        "Examples:\n  compass export html\n  compass export graphml --graph compass-out/graph.json\n  compass export neo4j --push bolt://localhost:7687\n\nTips:\n  Run `compass help export <format>` for format-specific options."
    ),
    page!(
        "export html",
        "Generate the interactive graph HTML report",
        ["compass export html [OPTIONS]"],
        "Options:\n  --graph <PATH>          Graph JSON [default: compass-out/graph.json]\n  --labels <PATH>         Community-label JSON\n  --node-limit <N>        Maximum nodes rendered [default: 5000]\n  --no-viz                Skip visualization output\n\nExamples:\n  compass export html\n  compass export html --node-limit 2000"
    ),
    page!(
        "export callflow-html",
        "Generate a sectioned call-flow report",
        ["compass export callflow-html [GRAPH_OR_DIR] [OPTIONS]"],
        "Arguments:\n  [GRAPH_OR_DIR]               Graph JSON or project/output directory\n\nOptions:\n  --graph <PATH>               Graph JSON\n  --labels <PATH>              Community-label JSON\n  --report <PATH>              GRAPH_REPORT.md path\n  --sections <PATH>            JSON section definitions\n  --output <HTML>              Output page\n  --lang <LANG>                Report language [default: auto]\n  --max-sections <N>           Maximum derived sections [default: 15]\n  --diagram-scale <NUMBER>     Mermaid scale [default: 1.0]\n  --max-diagram-nodes <N>      Nodes per diagram [default: 18]\n  --max-diagram-edges <N>      Edges per diagram [default: 24]\n\nExamples:\n  compass export callflow-html\n  compass export callflow-html ./compass-out --lang en --max-sections 10"
    ),
    page!(
        "export obsidian",
        "Export graph notes for an Obsidian vault",
        ["compass export obsidian [OPTIONS]"],
        "Options:\n  --graph <PATH>          Graph JSON\n  --labels <PATH>         Community-label JSON\n  --dir <PATH>            Vault output directory [default: compass-out/obsidian]\n\nExamples:\n  compass export obsidian\n  compass export obsidian --dir ./notes/compass"
    ),
    page!(
        "export wiki",
        "Export a linked Markdown knowledge wiki",
        ["compass export wiki [OPTIONS]"],
        "Options:\n  --graph <PATH>          Graph JSON\n  --labels <PATH>         Community-label JSON\n\nExamples:\n  compass export wiki\n  compass export wiki --graph artifacts/graph.json"
    ),
    page!(
        "export svg",
        "Export a static SVG graph overview",
        ["compass export svg [OPTIONS]"],
        "Options:\n  --graph <PATH>          Graph JSON\n  --labels <PATH>         Community-label JSON\n\nExamples:\n  compass export svg\n  compass export svg --graph artifacts/graph.json"
    ),
    page!(
        "export graphml",
        "Export portable GraphML",
        ["compass export graphml [OPTIONS]"],
        "Options:\n  --graph <PATH>          Graph JSON\n\nExamples:\n  compass export graphml\n  compass export graphml --graph artifacts/graph.json"
    ),
    page!(
        "export neo4j",
        "Write Cypher or push the graph to Neo4j",
        ["compass export neo4j [OPTIONS]"],
        "Options:\n  --graph <PATH>          Graph JSON\n  --push <URI>            Neo4j Bolt URI\n  --user <NAME>           Database user [default: neo4j]\n  --password <PASSWORD>   Database password [env: NEO4J_PASSWORD]\n\nExamples:\n  compass export neo4j\n  compass export neo4j --push bolt://localhost:7687 --user neo4j\n\nNotes:\n  Prefer NEO4J_PASSWORD over entering a secret in shell history."
    ),
    page!(
        "export falkordb",
        "Write Cypher or push the graph to FalkorDB",
        ["compass export falkordb [OPTIONS]"],
        "Options:\n  --graph <PATH>          Graph JSON\n  --push <URI>            FalkorDB URI\n  --user <NAME>           Database user\n  --password <PASSWORD>   Database password [env: FALKORDB_PASSWORD]\n\nExamples:\n  compass export falkordb\n  compass export falkordb --push redis://localhost:6379\n\nNotes:\n  Prefer FALKORDB_PASSWORD over entering a secret in shell history."
    ),
    page!(
        "serve",
        "Serve the knowledge graph through the Model Context Protocol",
        ["compass serve [GRAPH_PATH] [OPTIONS]"],
        "Arguments:\n  [GRAPH_PATH]                   Graph JSON [default: compass-out/graph.json]\n\nOptions:\n  --graph <PATH>                 Graph JSON; conflicts with GRAPH_PATH\n  --transport <stdio|http>       Transport [default: stdio]\n  --host <HOST>                  HTTP bind host [default: 127.0.0.1]\n  --port <PORT>                  HTTP bind port [default: 8080]\n  --api-key <KEY>                HTTP bearer token [env: GRAPHIFY_API_KEY]\n  --path <PATH>                  HTTP endpoint [default: /mcp]\n  --json-response                Use JSON responses for HTTP\n  --stateless                    Disable HTTP sessions\n  --session-timeout <SECONDS>    Session timeout [default: 3600]\n\nExamples:\n  compass serve\n  compass serve --transport http --port 9000\n\nNotes:\n  HTTP transport requires an API key when binding outside a trusted local environment."
    ),
    page!(
        "global",
        "Manage a graph assembled from multiple repositories",
        ["compass global <COMMAND>"],
        "Examples:\n  compass global list\n  compass global add compass-out/graph.json --as api\n\nTips:\n  Run `compass help global <command>` for command-specific arguments."
    ),
    page!(
        "global add",
        "Add or refresh one repository in the global graph",
        ["compass global add <GRAPH_JSON> [OPTIONS]"],
        "Arguments:\n  <GRAPH_JSON>             Repository graph JSON\n\nOptions:\n  --as <TAG>               Stable repository tag\n\nExamples:\n  compass global add compass-out/graph.json --as api"
    ),
    page!(
        "global remove",
        "Remove one repository from the global graph",
        ["compass global remove <TAG>"],
        "Arguments:\n  <TAG>                    Repository tag\n\nExamples:\n  compass global remove api"
    ),
    page!(
        "global list",
        "List repositories in the global graph",
        ["compass global list"],
        "Examples:\n  compass global list"
    ),
    page!(
        "global path",
        "Print the global graph file path",
        ["compass global path"],
        "Examples:\n  compass global path"
    ),
    page!(
        "clone",
        "Clone or refresh a GitHub repository in Compass storage",
        ["compass clone <GITHUB_URL> [OPTIONS]"],
        "Arguments:\n  <GITHUB_URL>             GitHub HTTPS or SSH repository URL\n\nOptions:\n  --branch <BRANCH>        Clone or update one branch\n  --out <DIR>              Destination directory\n\nExamples:\n  compass clone https://github.com/org/project\n  compass clone git@github.com:org/project.git --branch main --out ./project"
    ),
    page!(
        "add",
        "Download a supported source into the ingestion directory",
        ["compass add <URL> [OPTIONS]"],
        "Arguments:\n  <URL>                    Source URL\n\nOptions:\n  --author <NAME>          Record the original author\n  --contributor <NAME>     Record the contributor\n  --dir <DIR>              Download directory [default: ./raw]\n\nExamples:\n  compass add https://example.com/paper.pdf\n  compass add https://example.com/post --author \"Ada Lovelace\" --dir ./research"
    ),
    page!(
        "prs",
        "Inspect and prioritize GitHub pull requests",
        ["compass prs [NUMBER] [OPTIONS]"],
        "Arguments:\n  [NUMBER]                 Pull request number for a detailed view\n\nOptions:\n  --triage                 Rank the review queue with a configured model\n  --worktrees              Show worktree, branch, and PR mapping\n  --conflicts              Show graph-community overlap\n  --wrong-base             Show pull requests targeting the wrong base\n  -b, --base <BRANCH>      Filter by base branch\n  -R, --repo <OWNER/REPO>  Select a GitHub repository\n  --graph <PATH>           Graph JSON used for conflict analysis\n\nExamples:\n  compass prs\n  compass prs 42\n  compass prs --conflicts --base main"
    ),
    page!(
        "hook",
        "Manage Compass Git hooks",
        ["compass hook <COMMAND>"],
        "Examples:\n  compass hook install\n  compass hook status\n  compass hook uninstall"
    ),
    page!(
        "hook install",
        "Install managed Compass Git hooks",
        ["compass hook install"],
        "Examples:\n  compass hook install"
    ),
    page!(
        "hook uninstall",
        "Remove managed Compass Git hooks",
        ["compass hook uninstall"],
        "Examples:\n  compass hook uninstall"
    ),
    page!(
        "hook status",
        "Show whether managed Compass Git hooks are installed",
        ["compass hook status"],
        "Examples:\n  compass hook status"
    ),
    page!(
        "install",
        "Install Compass guidance for coding assistants",
        ["compass install [PLATFORM] [OPTIONS]"],
        "Arguments:\n  [PLATFORM]               Assistant platform; may also use --platform\n\nOptions:\n  --platform <NAME>        claude, codex, opencode, kilo, aider, copilot, claw, droid, trae, trae-cn, hermes, kiro, pi, codebuddy, antigravity, antigravity-windows, windows, kimi, amp, agents, devin, gemini, or cursor\n  --project                Install into the current project\n  --strict                 Require an initial graph query; requires --project\n\nExamples:\n  compass install --platform codex\n  compass install claude --project --strict\n\nTips:\n  Direct aliases such as `compass codex install` remain available for compatibility."
    ),
    page!(
        "uninstall",
        "Remove installed Compass assistant guidance",
        ["compass uninstall [PLATFORM] [OPTIONS]"],
        "Arguments:\n  [PLATFORM]               Remove one assistant platform\n\nOptions:\n  --platform <NAME>        Select one assistant platform\n  --project                Remove project-scoped files\n  --purge                  Remove all installed Compass guidance\n\nExamples:\n  compass uninstall --platform codex\n  compass uninstall --project --purge"
    ),
    page!(
        "provider",
        "Manage custom semantic model providers",
        ["compass provider <COMMAND>"],
        "Examples:\n  compass provider list\n  compass provider show local\n\nTips:\n  Run `compass help provider add` for endpoint and credential configuration."
    ),
    page!(
        "provider add",
        "Register a custom OpenAI-compatible provider",
        [
            "compass provider add <NAME> --base-url <URL> --default-model <MODEL> --env-key <KEY> [OPTIONS]"
        ],
        "Arguments:\n  <NAME>                       Provider name\n\nOptions:\n  --base-url <URL>             API base URL\n  --default-model <MODEL>      Default model identifier\n  --env-key <KEY>              Environment variable containing credentials\n  --pricing-input <NUMBER>     Input price per million tokens [default: 0]\n  --pricing-output <NUMBER>    Output price per million tokens [default: 0]\n\nExamples:\n  compass provider add local --base-url http://localhost:11434/v1 --default-model qwen3 --env-key LOCAL_API_KEY"
    ),
    page!(
        "provider list",
        "List registered custom providers",
        ["compass provider list"],
        "Examples:\n  compass provider list"
    ),
    page!(
        "provider show",
        "Show one custom provider configuration",
        ["compass provider show <NAME>"],
        "Arguments:\n  <NAME>                   Provider name\n\nExamples:\n  compass provider show local"
    ),
    page!(
        "provider remove",
        "Remove a custom provider",
        ["compass provider remove <NAME>"],
        "Arguments:\n  <NAME>                   Provider name\n\nExamples:\n  compass provider remove local"
    ),
    page!(
        "save-result",
        "Store a graph question and its outcome as reusable memory",
        [
            "compass save-result --question <TEXT> (--answer <TEXT> | --answer-file <PATH>) [OPTIONS]"
        ],
        "Options:\n  --question <TEXT>        Required original question\n  --answer <TEXT>          Inline answer; conflicts with --answer-file\n  --answer-file <PATH>     Read the answer from a file; conflicts with --answer\n  --type <TYPE>            Query type\n  --nodes <NODE...>        Related graph nodes\n  --outcome <useful|dead_end|corrected> Result classification\n  --correction <TEXT>      Corrected answer or guidance\n  --memory-dir <DIR>       Memory directory\n\nExamples:\n  compass save-result --question \"Where is auth?\" --answer \"src/auth.rs\" --outcome useful\n  compass save-result --question \"Old path\" --answer-file answer.md --outcome corrected --correction \"Use src/session.rs\""
    ),
    page!(
        "reflect",
        "Consolidate saved query outcomes into durable lessons",
        ["compass reflect [OPTIONS]"],
        "Options:\n  --memory-dir <DIR>           Memory directory\n  --out <PATH>                 Lesson output path\n  --graph <PATH>               Graph JSON\n  --analysis <PATH>            Graph analysis JSON\n  --labels <PATH>              Community-label JSON\n  --half-life-days <N>         Evidence half-life\n  --min-corroboration <N>      Minimum supporting results\n  --if-stale                   Skip reflection when lessons are fresh\n\nExamples:\n  compass reflect\n  compass reflect --if-stale --min-corroboration 2"
    ),
    page!(
        "diagnose",
        "Inspect graph health and compatibility problems",
        ["compass diagnose <COMMAND>"],
        "Examples:\n  compass diagnose multigraph\n  compass diagnose multigraph --json"
    ),
    page!(
        "diagnose multigraph",
        "Diagnose duplicate and directed-edge behavior in a graph",
        ["compass diagnose multigraph [OPTIONS]"],
        "Options:\n  --graph <PATH>          Graph JSON [default: compass-out/graph.json]\n  --json                  Emit machine-readable JSON\n  --max-examples <N>      Limit diagnostic examples\n  --directed              Force directed interpretation; conflicts with --undirected\n  --undirected            Force undirected interpretation; conflicts with --directed\n  --extract-path <PATH>   Compare against extraction output\n\nExamples:\n  compass diagnose multigraph\n  compass diagnose multigraph --graph old.json --json --max-examples 20"
    ),
    page!(
        "check-update",
        "Check whether semantic inputs require a graph update",
        ["compass check-update <PATH>"],
        "Arguments:\n  <PATH>                   Project directory\n\nExamples:\n  compass check-update ."
    ),
    page!(
        "merge-driver",
        "Merge graph JSON files for Git's custom merge driver",
        ["compass merge-driver <BASE> <CURRENT> <OTHER>"],
        "Arguments:\n  <BASE>                   Common ancestor file\n  <CURRENT>                Current branch file\n  <OTHER>                  Other branch file\n\nExamples:\n  compass merge-driver %O %A %B\n\nNotes:\n  This command is normally invoked by Git after Compass installation."
    ),
    page!(
        "hook-check",
        "Run the lightweight installed-hook compatibility check",
        ["compass hook-check"],
        "Examples:\n  compass hook-check\n\nNotes:\n  Installed assistant integrations invoke this command automatically."
    ),
    page!(
        "hook-guard",
        "Evaluate an installed assistant tool guard",
        ["compass hook-guard <KIND> [OPTIONS]"],
        "Arguments:\n  <KIND>                   search, read, or gemini\n\nOptions:\n  --strict                 Enforce the first-read graph-query guard\n\nExamples:\n  compass hook-guard read --strict\n\nNotes:\n  Installed assistant integrations invoke this command with JSON on standard input."
    ),
    internal_page!(
        "history-worker",
        "Process queued immutable history builds",
        ["compass history-worker"],
        "Notes:\n  Internal command. Use `compass history status` to inspect public history state."
    ),
    internal_page!(
        "hook-spawn",
        "Start a managed background hook refresh",
        ["compass hook-spawn [PATH]"],
        "Notes:\n  Internal command. Use `compass hook status` to inspect managed hooks."
    ),
    internal_page!(
        "hook-refresh",
        "Refresh graph artifacts for a managed hook",
        ["compass hook-refresh [PATH]"],
        "Notes:\n  Internal command. Use `compass update` for an interactive refresh."
    ),
];

pub fn request_os(arguments: &[OsString], style: HelpStyle) -> Option<Outcome> {
    let arguments = arguments
        .iter()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    request(&arguments, style)
}

pub(crate) fn request(arguments: &[String], style: HelpStyle) -> Option<Outcome> {
    if arguments.is_empty() {
        return Some(Outcome::success(render_root(style)));
    }
    let explicit_help = arguments.first().is_some_and(|value| value == "help");
    let help_index = arguments.iter().position(|value| is_help_flag(value));
    if !explicit_help && help_index.is_none() {
        return None;
    }
    let tokens = if explicit_help {
        let arguments = &arguments[1..];
        &arguments[..arguments
            .iter()
            .position(|value| is_help_flag(value))
            .unwrap_or(arguments.len())]
    } else {
        &arguments[..help_index.unwrap_or(arguments.len())]
    };
    if tokens.is_empty() {
        return Some(Outcome::success(render_root(style)));
    }
    if let Some(alias) = alias_page(tokens) {
        return Some(Outcome::success(render_page(alias, style)));
    }
    if explicit_help {
        let path = tokens.join(" ");
        return Some(match page(&path) {
            Some(page) => Outcome::success(render_page(page, style)),
            None => help_path_error(tokens),
        });
    }
    let leading = tokens
        .iter()
        .take_while(|value| !value.starts_with('-'))
        .map(String::as_str)
        .collect::<Vec<_>>();
    let Some((matched, length)) = longest_page_prefix(&leading) else {
        return Some(help_path_error(tokens));
    };
    if length < leading.len() && has_children(matched.path) {
        return Some(unknown_child_error(matched.path, leading[length]));
    }
    Some(Outcome::success(render_page(matched, style)))
}

fn is_help_flag(value: &str) -> bool {
    matches!(value, "-h" | "--help" | "-?")
}

pub(crate) fn unknown_command(command: &str) -> String {
    let mut message = format!("error: unknown command '{command}'");
    if let Some(suggestion) = closest(command, root_commands()) {
        message.push_str(&format!("\nDid you mean '{suggestion}'?"));
    }
    message.push_str("\nRun 'compass --help' for usage.");
    message
}

pub(crate) fn append_usage_hint(
    mut outcome: Outcome,
    command: &str,
    arguments: &[String],
) -> Outcome {
    if outcome.code != 2 || outcome.stderr.is_empty() || outcome.stderr.contains("--help") {
        return outcome;
    }
    let mut tokens = vec![command];
    tokens.extend(
        arguments
            .iter()
            .take_while(|value| !value.starts_with('-'))
            .map(String::as_str),
    );
    let path = longest_page_prefix(&tokens).map_or(command, |(page, _)| page.path);
    outcome
        .stderr
        .push_str(&format!("\nRun `compass {path} --help` for usage."));
    outcome
}

fn alias_page(tokens: &[String]) -> Option<&'static Page> {
    const ALIASES: &[&str] = &[
        "agents",
        "skills",
        "aider",
        "amp",
        "antigravity",
        "claude",
        "claw",
        "codebuddy",
        "codex",
        "copilot",
        "cursor",
        "devin",
        "droid",
        "gemini",
        "hermes",
        "kilo",
        "kiro",
        "opencode",
        "pi",
        "trae",
        "trae-cn",
        "vscode",
    ];
    tokens
        .first()
        .filter(|name| ALIASES.contains(&name.as_str()))
        .and_then(|_| page("install"))
}

fn page(path: &str) -> Option<&'static Page> {
    PAGES.iter().find(|page| page.path == path)
}

fn longest_page_prefix(tokens: &[&str]) -> Option<(&'static Page, usize)> {
    (1..=tokens.len()).rev().find_map(|length| {
        let path = tokens[..length].join(" ");
        page(&path).map(|page| (page, length))
    })
}

fn help_path_error(tokens: &[String]) -> Outcome {
    let borrowed = tokens.iter().map(String::as_str).collect::<Vec<_>>();
    if let Some((parent, length)) = longest_page_prefix(&borrowed)
        && length < borrowed.len()
    {
        return unknown_child_error(parent.path, borrowed[length]);
    }
    let unknown = borrowed.first().copied().unwrap_or_default();
    Outcome::failure_with_code(unknown_help_message("", unknown), 2)
}

fn unknown_child_error(parent: &str, unknown: &str) -> Outcome {
    Outcome::failure_with_code(unknown_help_message(parent, unknown), 2)
}

fn unknown_help_message(parent: &str, unknown: &str) -> String {
    let noun = if parent.is_empty() {
        "command"
    } else {
        "subcommand"
    };
    let mut message = format!("error: unknown {noun} '{unknown}'");
    let candidates = if parent.is_empty() {
        root_commands()
    } else {
        child_names(parent)
    };
    if let Some(suggestion) = closest(unknown, candidates) {
        message.push_str(&format!("\nDid you mean '{suggestion}'?"));
    }
    let help = if parent.is_empty() {
        "compass --help".to_owned()
    } else {
        format!("compass {parent} --help")
    };
    message.push_str(&format!("\nRun `{help}` for usage."));
    message
}

fn render_root(style: HelpStyle) -> String {
    let mut output = String::from(
        "Compass: turn a codebase into a searchable knowledge graph\n\nUsage:\n  compass <COMMAND> [OPTIONS]",
    );
    for group in GROUPS {
        output.push_str("\n\n");
        output.push_str(group.title);
        output.push_str(":\n");
        let width = group
            .commands
            .iter()
            .map(|command| command.len())
            .max()
            .unwrap_or(0);
        for command in group.commands {
            let summary = page(command).map_or("", |page| page.summary);
            output.push_str(&format!("  {command:width$}  {summary}\n"));
        }
        let _ = output.pop();
    }
    output.push_str(
        "\n\nOptions:\n  -h, --help     Show help\n  -V, --version  Show version\n\nRun `compass help <command>` for detailed help.",
    );
    finish(output, style)
}

fn render_page(page: &Page, style: HelpStyle) -> String {
    let mut output = format!("{}\n\nUsage:", page.summary);
    for usage in page.usage {
        output.push_str("\n  ");
        output.push_str(usage);
    }
    let children = children(page.path);
    if !children.is_empty() {
        output.push_str("\n\nCommands:\n");
        let width = children
            .iter()
            .map(|child| child.path.rsplit(' ').next().unwrap_or(child.path).len())
            .max()
            .unwrap_or(0);
        for child in children {
            let name = child.path.rsplit(' ').next().unwrap_or(child.path);
            output.push_str(&format!("  {name:width$}  {}\n", child.summary));
        }
        let _ = output.pop();
    }
    let details = add_help_option(page.details);
    if !details.is_empty() {
        output.push_str("\n\n");
        output.push_str(&details);
    }
    finish(output, style)
}

fn add_help_option(details: &str) -> String {
    let markers = ["\n\nExamples:", "\n\nTips:", "\n\nNotes:"];
    let insert_at = markers
        .iter()
        .filter_map(|marker| details.find(marker))
        .min()
        .unwrap_or(details.len());
    let (before, after) = details.split_at(insert_at);
    let addition = if before.contains("Options:") {
        let description_column = before
            .rsplit_once("Options:\n")
            .map_or(26, |(_, options)| option_description_column(options));
        let padding = description_column.saturating_sub(2 + "-h, --help".len());
        format!("\n  -h, --help{}Show this help", " ".repeat(padding))
    } else {
        "\n\nOptions:\n  -h, --help  Show this help".to_owned()
    };
    format!("{before}{addition}{after}")
}

fn option_description_column(options: &str) -> usize {
    options
        .lines()
        .filter_map(|line| {
            let bytes = line.as_bytes();
            let gap = 2 + bytes.get(2..)?.windows(2).position(|pair| pair == b"  ")?;
            let spaces = bytes[gap..]
                .iter()
                .take_while(|byte| **byte == b' ')
                .count();
            Some(gap + spaces)
        })
        .max()
        .unwrap_or(26)
}

fn finish(output: String, style: HelpStyle) -> String {
    let plain = output
        .lines()
        .flat_map(wrap_line)
        .collect::<Vec<_>>()
        .join("\n");
    match style {
        HelpStyle::Plain => plain,
        HelpStyle::Ansi => style_output(&plain),
    }
}

fn wrap_line(line: &str) -> Vec<String> {
    if line.chars().count() <= WIDTH {
        return vec![line.to_owned()];
    }
    let leading = line.len().saturating_sub(line.trim_start().len());
    let trimmed = line.trim_start();
    let split = trimmed.as_bytes().windows(2).position(|pair| pair == b"  ");
    let (prefix, text) = split.map_or_else(
        || (" ".repeat(leading), trimmed),
        |index| {
            let description = trimmed[index..].trim_start();
            (
                format!("{}{}  ", " ".repeat(leading), &trimmed[..index]),
                description,
            )
        },
    );
    let continuation = " ".repeat(prefix.chars().count());
    wrap_words(text, &prefix, &continuation)
}

fn wrap_words(text: &str, first_prefix: &str, next_prefix: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = first_prefix.to_owned();
    for word in text.split_whitespace() {
        let separator = usize::from(!current.is_empty() && !current.ends_with(' '));
        if current.chars().count() + separator + word.chars().count() > WIDTH
            && current.trim() != first_prefix.trim()
        {
            lines.push(current);
            current = next_prefix.to_owned();
        }
        if !current.is_empty() && !current.ends_with(' ') {
            current.push(' ');
        }
        current.push_str(word);
    }
    lines.push(current);
    lines
}

fn style_output(plain: &str) -> String {
    const BOLD_CYAN: &str = "\x1b[1;36m";
    const CYAN: &str = "\x1b[36m";
    const RESET: &str = "\x1b[0m";
    plain
        .lines()
        .enumerate()
        .map(|(index, line)| {
            if index == 0 {
                return format!("{BOLD_CYAN}{line}{RESET}");
            }
            if !line.starts_with(' ') && line.ends_with(':') {
                return format!("{BOLD_CYAN}{line}{RESET}");
            }
            if let Some(content) = line.strip_prefix("  ") {
                let term_end = content
                    .as_bytes()
                    .windows(2)
                    .position(|pair| pair == b"  ")
                    .unwrap_or(content.len());
                let (term, rest) = content.split_at(term_end);
                if !term.is_empty() {
                    return format!("  {CYAN}{term}{RESET}{rest}");
                }
            }
            line.to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn has_children(path: &str) -> bool {
    PAGES
        .iter()
        .any(|candidate| is_immediate_child(path, candidate.path))
}

fn children(path: &str) -> Vec<&'static Page> {
    PAGES
        .iter()
        .filter(|candidate| {
            candidate.visibility == Visibility::Public && is_immediate_child(path, candidate.path)
        })
        .collect()
}

fn child_names(path: &str) -> Vec<&'static str> {
    children(path)
        .into_iter()
        .filter_map(|page| page.path.rsplit(' ').next())
        .collect()
}

fn is_immediate_child(parent: &str, candidate: &str) -> bool {
    candidate
        .strip_prefix(parent)
        .and_then(|suffix| suffix.strip_prefix(' '))
        .is_some_and(|suffix| !suffix.contains(' '))
}

fn root_commands() -> Vec<&'static str> {
    GROUPS
        .iter()
        .flat_map(|group| group.commands.iter().copied())
        .collect()
}

fn closest<'a>(unknown: &str, candidates: Vec<&'a str>) -> Option<&'a str> {
    let threshold = if unknown.chars().count() <= 3 { 1 } else { 2 };
    candidates
        .into_iter()
        .map(|candidate| (levenshtein(unknown, candidate), candidate))
        .filter(|(distance, _)| *distance <= threshold)
        .min_by_key(|(distance, candidate)| (*distance, candidate.len()))
        .map(|(_, candidate)| candidate)
}

fn levenshtein(left: &str, right: &str) -> usize {
    let right = right.chars().collect::<Vec<_>>();
    let mut previous = (0..=right.len()).collect::<Vec<_>>();
    for (left_index, left_char) in left.chars().enumerate() {
        let mut current = vec![left_index + 1];
        for (right_index, right_char) in right.iter().enumerate() {
            let substitution = previous[right_index] + usize::from(left_char != *right_char);
            current.push(usize::min(
                usize::min(previous[right_index + 1] + 1, current[right_index] + 1),
                substitution,
            ));
        }
        previous = current;
    }
    previous[right.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn catalog_has_unique_complete_public_roots() {
        let roots = root_commands();
        assert_eq!(roots.len(), 34);
        for root in roots {
            let matches = PAGES.iter().filter(|page| page.path == root).count();
            assert_eq!(matches, 1, "{root}");
            assert_eq!(
                page(root).map(|page| page.visibility),
                Some(Visibility::Public)
            );
        }
        for internal in ["history-worker", "hook-spawn", "hook-refresh"] {
            assert_eq!(
                page(internal).map(|page| page.visibility),
                Some(Visibility::Internal)
            );
        }
    }

    #[test]
    fn nested_help_forms_are_equivalent() {
        let flag = request(&strings(&["history", "build", "--help"]), HelpStyle::Plain);
        assert!(flag.is_some(), "expected help request");
        let Some(flag) = flag else {
            return;
        };
        let command = request(&strings(&["help", "history", "build"]), HelpStyle::Plain);
        assert!(command.is_some(), "expected help request");
        let Some(command) = command else {
            return;
        };
        assert_eq!(flag.code, 0);
        assert_eq!(flag.stdout, command.stdout);
        assert!(flag.stdout.contains("--profile-from"));
        assert!(flag.stdout.contains("Examples:"));
    }

    #[test]
    fn styling_policy_and_rendering_preserve_plain_text() {
        assert_eq!(HelpStyle::detect(false, None, None), HelpStyle::Plain);
        assert_eq!(
            HelpStyle::detect(true, Some(OsStr::new("")), None),
            HelpStyle::Plain
        );
        assert_eq!(
            HelpStyle::detect(true, None, Some(OsStr::new("DUMB"))),
            HelpStyle::Plain
        );
        assert_eq!(HelpStyle::detect(true, None, None), HelpStyle::Ansi);
        let update_page = page("update");
        assert!(update_page.is_some(), "expected update page");
        let Some(update_page) = update_page else {
            return;
        };
        let plain = render_page(update_page, HelpStyle::Plain);
        let styled = render_page(update_page, HelpStyle::Ansi);
        assert!(!plain.contains('\x1b'));
        assert_eq!(strip_ansi(&styled), plain);
        for page in PAGES {
            let output = render_page(page, HelpStyle::Plain);
            assert!(
                output.lines().all(|line| line.chars().count() <= WIDTH),
                "{}",
                page.path
            );
        }
    }

    #[test]
    fn typo_suggestions_are_conservative() {
        assert!(unknown_command("udpate").contains("Did you mean 'update'?"));
        assert!(!unknown_command("bananas").contains("Did you mean"));
        let outcome = request(&strings(&["help", "history", "buidl"]), HelpStyle::Plain);
        assert!(outcome.is_some(), "expected help request");
        let Some(outcome) = outcome else {
            return;
        };
        assert_eq!(outcome.code, 2);
        assert!(outcome.stderr.contains("Did you mean 'build'?"));
    }

    fn strip_ansi(value: &str) -> String {
        let mut output = String::new();
        let mut chars = value.chars().peekable();
        while let Some(character) = chars.next() {
            if character == '\x1b' && chars.peek() == Some(&'[') {
                let _ = chars.next();
                for value in chars.by_ref() {
                    if value == 'm' {
                        break;
                    }
                }
            } else {
                output.push(character);
            }
        }
        output
    }
}
