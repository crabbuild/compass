use std::collections::{BTreeMap, HashSet};
use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

use compass_semantic::{CommunityLabelOptions, PlainTextResponse, label_communities_with};
use serde_json::Value;

fn repository_root() -> PathBuf {
    if let Some(root) = std::env::var_os("GRAPHIFY_REPO_ROOT") {
        return PathBuf::from(root);
    }
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .map_or_else(|| PathBuf::from("../.."), Path::to_path_buf)
}

fn fixture() -> (BTreeMap<String, String>, BTreeMap<usize, Vec<String>>) {
    let node_labels = [
        ("order_place", "place_order"),
        ("order_repo", "OrderRepository"),
        ("pay_charge", "charge_card"),
        ("pay_stripe", "StripeClient"),
    ]
    .into_iter()
    .map(|(id, label)| (id.to_owned(), label.to_owned()))
    .collect();
    (
        node_labels,
        BTreeMap::from([
            (0, vec!["order_place".to_owned(), "order_repo".to_owned()]),
            (1, vec!["pay_charge".to_owned(), "pay_stripe".to_owned()]),
        ]),
    )
}

#[test]
fn prompt_budget_and_labels_match_python_oracle() -> Result<(), Box<dyn Error>> {
    let repo = repository_root();
    let python = if cfg!(windows) {
        repo.join(".venv/Scripts/python.exe")
    } else {
        repo.join(".venv/bin/python")
    };
    let script = r#"
import json
import networkx as nx
import graphify.llm as llm
G = nx.Graph()
G.add_node('order_place', label='place_order')
G.add_node('order_repo', label='OrderRepository')
G.add_node('pay_charge', label='charge_card')
G.add_node('pay_stripe', label='StripeClient')
communities = {0: ['order_place', 'order_repo'], 1: ['pay_charge', 'pay_stripe']}
captured = {}
def call(prompt, **kwargs):
    captured['prompt'] = prompt
    captured['max_tokens'] = kwargs['max_tokens']
    return '{"0":"Order Management","1":"Payment Flow"}'
llm._call_llm = call
labels = llm.label_communities(G, communities, backend='gemini', max_concurrency=1)
print(json.dumps({'captured': captured, 'labels': labels}, ensure_ascii=False))
"#;
    let output = Command::new(python)
        .args(["-c", script])
        .current_dir(&repo)
        .env("PYTHONPATH", &repo)
        .output()?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned().into());
    }
    let expected: Value = serde_json::from_slice(&output.stdout)?;

    let (node_labels, communities) = fixture();
    let captured = Mutex::new(None);
    let mut options = CommunityLabelOptions::new("gemini");
    options.max_concurrency = 1;
    let result = label_communities_with(
        &node_labels,
        &communities,
        &HashSet::new(),
        &options,
        &|prompt, max| {
            if let Ok(mut captured) = captured.lock() {
                *captured = Some((prompt.to_owned(), max));
            }
            Ok(PlainTextResponse {
                text: "{\"0\":\"Order Management\",\"1\":\"Payment Flow\"}".to_owned(),
                input_tokens: 0,
                output_tokens: 0,
                model: "fixture".to_owned(),
            })
        },
    );
    let actual_capture = captured
        .into_inner()
        .map_err(|_| "capture mutex was poisoned")?
        .ok_or("provider callback was not invoked")?;
    assert_eq!(
        actual_capture.0,
        expected
            .pointer("/captured/prompt")
            .and_then(Value::as_str)
            .unwrap_or_default()
    );
    assert_eq!(
        actual_capture.1 as u64,
        expected
            .pointer("/captured/max_tokens")
            .and_then(Value::as_u64)
            .unwrap_or_default()
    );
    let expected_labels = expected
        .get("labels")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter_map(|(key, value)| Some((key.parse().ok()?, value.as_str()?.to_owned())))
        .collect::<BTreeMap<usize, String>>();
    assert_eq!(result.labels, expected_labels);
    Ok(())
}
