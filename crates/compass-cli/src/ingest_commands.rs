use std::path::PathBuf;

use compass_ingest::{IngestRequest, ingest};

use crate::{Frontend, Outcome};

pub(super) fn command_add(frontend: Frontend, args: &[String]) -> Outcome {
    if frontend == Frontend::Graphify
        && args
            .iter()
            .any(|argument| matches!(argument.as_str(), "--help" | "-h"))
    {
        return Outcome::success("Run 'graphify --help' for full usage.".to_owned());
    }
    let Some(url) = args.first() else {
        return Outcome::failure(add_help(frontend));
    };
    let mut author = None;
    let mut contributor = None;
    let mut target_dir = PathBuf::from("raw");
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--author" if index + 1 < args.len() => {
                author = Some(args[index + 1].as_str());
                index += 2;
            }
            "--contributor" if index + 1 < args.len() => {
                contributor = Some(args[index + 1].as_str());
                index += 2;
            }
            "--dir" if index + 1 < args.len() => {
                target_dir = PathBuf::from(&args[index + 1]);
                index += 2;
            }
            _ => index += 1,
        }
    }
    match ingest(&IngestRequest {
        url,
        target_dir: &target_dir,
        author,
        contributor,
    }) {
        Ok(result) => Outcome::success(format!(
            "{}\nSaved to {}\nRun /graphify --update in your AI assistant to update the graph.",
            result.message,
            result.path.display()
        )),
        Err(error) => Outcome::failure(format!("error: {error}")),
    }
}

pub(super) fn add_help(frontend: Frontend) -> String {
    match frontend {
        Frontend::Compass => {
            "Usage: compass add <url> [--author Name] [--contributor Name] [--dir ./raw]"
        }
        Frontend::Graphify => {
            "Usage: graphify add <url> [--author Name] [--contributor Name] [--dir ./raw]"
        }
    }
    .to_owned()
}
