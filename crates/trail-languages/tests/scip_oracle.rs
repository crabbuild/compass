use std::error::Error;
use std::path::PathBuf;
use std::process::Command;

use serde_json::{Value, json};
use trail_languages::ingest_scip_json;

#[test]
fn simplified_scip_ingestion_matches_python_oracle() -> Result<(), Box<dyn Error>> {
    let cases = vec![
        Value::Null,
        json!({}),
        json!({"documents":"not-a-list"}),
        json!({
            "documents": [
                {"relative_path":"src/a.py","language":"python","symbols":[
                    {"symbol":"pkg core#A","kind":"class","display_name":"A<root>",
                     "documentation":["Docs <unsafe>\u{0}"],"occurrences":[{"range":[12,0,12,1]}],
                     "relationships":[
                         {"symbol":"pkg core#B","is_implementation":true,"note":"<impl>"},
                         {"symbol":"external lib#Thing","is_definition":"true"}
                     ]},
                    {"symbol":"pkg core#B","kind":"method","occurrences":[{"range":[0]}]},
                    {"symbol":"pkg core#B","relationships":[{"symbol":"pkg core#A","is_type_definition":true}]}
                ]},
                {"relative_path":"src/b.py","symbols":[
                    {"symbol":"pkg core#B","kind":"function"},
                    {"symbol":"other#Caller","relationships":[{"symbol":"pkg core#A","is_definition":true}]}
                ]},
                {"relative_path":"src/c.py","symbols":[
                    {"symbol":"third#Caller","relationships":[{"symbol":"pkg core#B","is_reference":true}]}
                ]},
                null,
                {"symbols":"bad"}
            ]
        }),
        json!({"documents":[{"symbols":[
            {"symbol":"#!!!","documentation":["x".repeat(700)],"occurrences":[{"range":[true]}],
             "relationships":[{"symbol":"#target","is_implementation":false,"list":vec!["x"; 60]}]},
            {"symbol":9}, null
        ]}]}),
    ];
    let source_file = "fallback.scip";
    let language = "typescript";
    let rust = cases
        .iter()
        .map(|case| serde_json::to_value(ingest_scip_json(case, source_file, language)))
        .collect::<Result<Vec<_>, _>>()?;

    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let python = std::env::var_os("GRAPHIFY_PYTHON")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo.join(".venv/bin/python"));
    let output = Command::new(python)
        .args([
            "-c",
            r#"import json,sys
from graphify.scip_ingest import ingest_scip_json
cases=json.loads(sys.stdin.read())
print(json.dumps([ingest_scip_json(case, source_file='fallback.scip', language='typescript') for case in cases], ensure_ascii=False))"#,
        ])
        .current_dir(&repo)
        .env("PYTHONPATH", &repo)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()?;
    use std::io::Write;
    let mut child = output;
    child
        .stdin
        .take()
        .ok_or("Python oracle stdin was unavailable")?
        .write_all(&serde_json::to_vec(&cases)?)?;
    let output = child.wait_with_output()?;
    assert!(
        output.status.success(),
        "Python SCIP oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let python: Vec<Value> = serde_json::from_slice(&output.stdout)?;
    assert_eq!(rust, python);
    Ok(())
}
