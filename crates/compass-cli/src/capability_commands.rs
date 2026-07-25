use std::collections::BTreeMap;

use serde::Serialize;

use crate::ide_contract::CAPABILITY_SCHEMA;
use crate::{Frontend, Outcome};

#[derive(Debug, Serialize)]
pub struct CapabilityReport {
    pub schema: &'static str,
    pub compass_version: &'static str,
    pub contracts: BTreeMap<&'static str, &'static str>,
    pub features: BTreeMap<&'static str, bool>,
}

pub fn command(frontend: Frontend, args: &[String]) -> Outcome {
    if frontend != Frontend::Compass {
        return Outcome::failure("error: capabilities is a Compass command".to_owned());
    }
    if args != ["--format", "json"] {
        return Outcome::failure("Usage: compass capabilities --format json".to_owned());
    }
    let report = CapabilityReport {
        schema: CAPABILITY_SCHEMA,
        compass_version: env!("CARGO_PKG_VERSION"),
        contracts: BTreeMap::from([
            ("graph_viewer", compass_output::GRAPH_VIEWER_SCHEMA),
            ("progress", crate::ide_contract::PROGRESS_SCHEMA),
        ]),
        features: BTreeMap::from([
            ("init", true),
            ("update", true),
            ("watch", true),
            ("graph", true),
            ("program", true),
            ("query", true),
            ("history", true),
            ("semantic_diff", true),
        ]),
    };
    match serde_json::to_string(&report) {
        Ok(json) => Outcome::success(json),
        Err(error) => Outcome::failure(format!("error: {error}")),
    }
}
