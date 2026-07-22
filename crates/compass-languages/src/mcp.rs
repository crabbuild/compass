use std::collections::HashSet;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::sync::LazyLock;

use compass_model::{EdgeRecord, NodeRecord};
use regex::Regex;
use serde_json::{Map, Value, json};

use crate::{ExtractError, Extraction, file_stem, make_id};

const MAX_BYTES: u64 = 1_048_576;
const MAX_SERVERS: usize = 200;

static NPM_PACKAGE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^@[a-z0-9][a-z0-9._-]*/[a-z0-9][a-z0-9._-]*(?:@[\w.\-+]+)?$")
        .unwrap_or_else(|error| unreachable!("static MCP npm regex is invalid: {error}"))
});
static PYTHON_MCP_PACKAGE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:[a-z0-9][a-z0-9._-]*-mcp(?:-[a-z0-9._-]+)?|mcp-[a-z0-9][a-z0-9._-]*)$")
        .unwrap_or_else(|error| unreachable!("static MCP Python regex is invalid: {error}"))
});
static ARGUMENT_FLAG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^-{1,2}\w")
        .unwrap_or_else(|error| unreachable!("static MCP argument regex is invalid: {error}"))
});

pub(crate) fn extract(path: &Path) -> Result<Extraction, ExtractError> {
    let mut raw = Vec::new();
    File::open(path)
        .map_err(|source| compass_files::FileError::Io {
            path: path.to_path_buf(),
            source,
        })?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut raw)
        .map_err(|source| compass_files::FileError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    if raw.len() > MAX_BYTES as usize {
        return Ok(failure("mcp config too large to index"));
    }
    let text = match std::str::from_utf8(&raw) {
        Ok(text) => text,
        Err(error) => return Ok(failure(&format!("mcp_ingest decode error: {error}"))),
    };
    let document: Value = match serde_json::from_str(text) {
        Ok(document) => document,
        Err(error) => return Ok(failure(&format!("mcp_ingest json error: {error}"))),
    };
    let Some(root) = document.as_object() else {
        return Ok(failure("mcp_ingest: root is not an object"));
    };
    let servers = root
        .get("mcpServers")
        .and_then(Value::as_object)
        .or_else(|| {
            root.get("mcp")
                .and_then(Value::as_object)
                .and_then(|mcp| mcp.get("servers"))
                .and_then(Value::as_object)
        });
    let Some(servers) = servers else {
        return Ok(failure("mcp_ingest: no mcpServers map"));
    };

    let source_file = path.to_string_lossy().into_owned();
    let file_id = make_id(&[&source_file]);
    let mut state = State {
        source_file,
        file_stem: file_stem(path),
        file_id: file_id.clone(),
        extraction: empty(),
        seen_nodes: HashSet::new(),
        seen_edges: HashSet::new(),
    };
    state.add_node(
        file_id,
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default(),
        "mcp_config_file",
    );
    let mut server_count = 0;
    for (server_name, specification) in servers {
        let Some(specification) = specification.as_object() else {
            continue;
        };
        if server_name.is_empty() {
            continue;
        }
        if server_count >= MAX_SERVERS {
            break;
        }
        server_count += 1;
        state.add_server(server_name, specification);
    }
    Ok(state.extraction)
}

struct State {
    source_file: String,
    file_stem: String,
    file_id: String,
    extraction: Extraction,
    seen_nodes: HashSet<String>,
    seen_edges: HashSet<(String, String, String)>,
}

impl State {
    fn add_server(&mut self, name: &str, specification: &Map<String, Value>) {
        let server_id = make_id(&[&self.file_stem, "mcp_server", name]);
        self.add_node(server_id.clone(), name, "mcp_server");
        self.add_edge(&self.file_id.clone(), &server_id, "contains", None);

        if let Some(command) = specification
            .get("command")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|command| !command.is_empty())
        {
            let command_id = make_id(&["mcp_command", command]);
            self.add_node(command_id.clone(), command, "mcp_command");
            self.add_edge(&server_id, &command_id, "references", Some("command"));
        }

        if let Some(arguments) = specification.get("args").and_then(Value::as_array)
            && let Some(package) = detect_package(arguments)
        {
            let package_id = make_id(&["mcp_package", &package]);
            self.add_node(package_id.clone(), &package, "mcp_package");
            self.add_edge(&server_id, &package_id, "references", Some("package"));
        }

        if let Some(environment) = specification.get("env").and_then(Value::as_object) {
            for name in environment.keys().filter(|name| !name.is_empty()) {
                let environment_id = make_id(&["env_var", name]);
                self.add_node(environment_id.clone(), name, "env_var");
                self.add_edge(&server_id, &environment_id, "requires_env", None);
            }
        }
    }

    fn add_node(&mut self, id: String, label: &str, kind: &str) {
        if id.is_empty() || !self.seen_nodes.insert(id.clone()) {
            return;
        }
        let mut metadata = Map::new();
        metadata.insert("mcp_kind".to_owned(), Value::String(kind.to_owned()));
        let mut attributes = Map::new();
        attributes.insert("label".to_owned(), Value::String(sanitize_label(label)));
        attributes.insert("file_type".to_owned(), Value::String("code".to_owned()));
        attributes.insert(
            "source_file".to_owned(),
            Value::String(self.source_file.clone()),
        );
        attributes.insert("source_location".to_owned(), Value::String("L1".to_owned()));
        attributes.insert("metadata".to_owned(), Value::Object(metadata));
        self.extraction.nodes.push(NodeRecord { id, attributes });
    }

    fn add_edge(&mut self, source: &str, target: &str, relation: &str, context: Option<&str>) {
        if source.is_empty() || target.is_empty() || source == target {
            return;
        }
        let key = (source.to_owned(), target.to_owned(), relation.to_owned());
        if !self.seen_edges.insert(key) {
            return;
        }
        let mut attributes = Map::new();
        attributes.insert("relation".to_owned(), Value::String(relation.to_owned()));
        attributes.insert(
            "confidence".to_owned(),
            Value::String("EXTRACTED".to_owned()),
        );
        attributes.insert("confidence_score".to_owned(), json!(1.0));
        attributes.insert(
            "source_file".to_owned(),
            Value::String(self.source_file.clone()),
        );
        attributes.insert("source_location".to_owned(), Value::String("L1".to_owned()));
        attributes.insert("weight".to_owned(), json!(1.0));
        if let Some(context) = context {
            attributes.insert("context".to_owned(), Value::String(context.to_owned()));
        }
        self.extraction.edges.push(EdgeRecord {
            source: source.to_owned(),
            target: target.to_owned(),
            attributes,
        });
    }
}

fn detect_package(arguments: &[Value]) -> Option<String> {
    arguments.iter().find_map(|argument| {
        let argument = argument.as_str()?.trim();
        if argument.is_empty() || ARGUMENT_FLAG.is_match(argument) {
            return None;
        }
        if NPM_PACKAGE.is_match(argument) {
            Some(strip_version(argument))
        } else if PYTHON_MCP_PACKAGE.is_match(argument) {
            Some(argument.to_owned())
        } else {
            None
        }
    })
}

fn strip_version(package: &str) -> String {
    let separator = if let Some(scoped) = package.strip_prefix('@') {
        scoped.find('@').map(|index| index + 1)
    } else {
        package.find('@')
    };
    separator.map_or_else(|| package.to_owned(), |index| package[..index].to_owned())
}

fn sanitize_label(label: &str) -> String {
    label
        .chars()
        .filter(|character| !matches!(*character as u32, 0..=31 | 127))
        .take(256)
        .collect()
}

fn empty() -> Extraction {
    Extraction {
        raw_calls: None,
        ..Extraction::default()
    }
}

fn failure(message: &str) -> Extraction {
    let mut extraction = empty();
    extraction.error = Some(message.to_owned());
    extraction
}
