use std::error::Error;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use serde_json::{Value, json};
use trail_model::{assert_valid_extraction, validate_extraction};

#[test]
fn extraction_validation_matches_python_oracle() -> Result<(), Box<dyn Error>> {
    let cases = vec![
        Value::Null,
        json!({}),
        json!({"nodes":[],"edges":[]}),
        json!({"nodes":"bad","edges":null}),
        json!({"nodes":[null, {}, {"id":[],"file_type":"bad"}],"edges":[null, {}]}),
        json!({
            "nodes":[
                {"id":"n1","label":"A","file_type":"code","source_file":"a.py"},
                {"id":{"bad":1},"label":"B","file_type":"video","source_file":"b.py"},
                {"id":true,"label":"C","file_type":"concept","source_file":"c.py"}
            ],
            "links":[
                {"source":"n1","target":"ghost","relation":"calls","confidence":"CERTAIN","source_file":"a.py"},
                {"source":["n1"],"target":1,"relation":"calls","confidence":"INFERRED","source_file":"a.py"}
            ]
        }),
        json!({"nodes":[],"edges":[{"source":[],"target":{},"confidence":"bad"}]}),
    ];
    let rust = cases
        .iter()
        .map(|case| {
            json!({
                "errors": validate_extraction(case),
                "assert_error": assert_valid_extraction(case).err().map(|error| error.to_string()),
            })
        })
        .collect::<Vec<_>>();

    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let python = std::env::var_os("GRAPHIFY_PYTHON")
        .map(PathBuf::from)
        .map(|path| {
            if path.is_absolute() {
                path
            } else {
                repo.join("rust").join(path)
            }
        })
        .unwrap_or_else(|| repo.join(".venv/bin/python"));
    let mut child = Command::new(python)
        .args([
            "-c",
            r#"import json,sys
from graphify.validate import validate_extraction,assert_valid
out=[]
for case in json.loads(sys.stdin.read()):
    errors=validate_extraction(case)
    try:
        assert_valid(case)
        message=None
    except ValueError as exc:
        message=str(exc)
    out.append({'errors':errors,'assert_error':message})
print(json.dumps(out,ensure_ascii=False))"#,
        ])
        .current_dir(&repo)
        .env("PYTHONPATH", &repo)
        .env("PYTHONHASHSEED", "0")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    child
        .stdin
        .take()
        .ok_or("Python validation oracle stdin was unavailable")?
        .write_all(&serde_json::to_vec(&cases)?)?;
    let output = child.wait_with_output()?;
    assert!(
        output.status.success(),
        "Python validation oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let python: Vec<Value> = serde_json::from_slice(&output.stdout)?;
    assert_eq!(rust, python);
    Ok(())
}
