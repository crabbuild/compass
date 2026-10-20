use std::error::Error;

use compass_languages::{
    Engine,
    source_query::{EXTRACTION_EXTENSION, SourceQueryExtraction},
};
use serde::Deserialize;

#[derive(Deserialize)]
struct Case {
    extensions: Vec<String>,
    source: String,
    definition: String,
}

#[test]
fn direct_initializer_projection_uses_the_exact_declarator_envelope() -> Result<(), Box<dyn Error>>
{
    let cases: Vec<Case> = serde_json::from_str(include_str!(
        "../../../fixtures/callable_initializer_definitions.json"
    ))?;
    for (index, case) in cases.iter().enumerate() {
        for extension in case
            .extensions
            .iter()
            .filter(|value| value.starts_with("ts"))
        {
            for seed in ["alpha", "zeta"] {
                let file = format!("{seed}_{index}.{extension}");
                let directory = tempfile::tempdir()?;
                let path = directory.path().join(&file);
                std::fs::write(&path, &case.source)?;
                let extraction = Engine::default().extract_source_graph_only(
                    &path,
                    &file,
                    case.source.as_bytes(),
                )?;
                assert!(extraction.error.is_none(), "{file}: {:?}", extraction.error);
                let raw: SourceQueryExtraction = serde_json::from_value(
                    extraction
                        .extensions
                        .get(EXTRACTION_EXTENSION)
                        .ok_or("missing source projection")?
                        .clone(),
                )?;
                let start = case
                    .source
                    .find(&case.definition)
                    .ok_or("missing definition")?;
                let end = start + case.definition.len();
                let matches = raw
                    .projection
                    .scopes
                    .iter()
                    .filter(|scope| scope.start == start && scope.end == end)
                    .collect::<Vec<_>>();
                assert_eq!(
                    matches.len(),
                    1,
                    "{file}: missing exact declarator envelope"
                );
                assert!(matches[0].names.contains_key("payload"), "{file}");
                assert!(
                    !matches[0].names.contains_key("opaque"),
                    "{file}: binding name is outside its body"
                );
                if case.source.contains("item =>") {
                    assert!(
                        !matches[0].names.contains_key("item"),
                        "{file}: callback body leaked"
                    );
                }
            }
        }
    }
    Ok(())
}
