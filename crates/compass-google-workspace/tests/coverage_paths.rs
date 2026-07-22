use std::error::Error;
use std::fs;
use std::path::Path;

use compass_google_workspace::{
    GOOGLE_WORKSPACE_EXTENSIONS, GoogleWorkspaceError, GwsExporter,
    convert_google_workspace_file_with, google_workspace_enabled, is_google_workspace_path,
    read_google_shortcut,
};

struct FixtureExporter {
    body: Result<Vec<u8>, &'static str>,
}

impl GwsExporter for FixtureExporter {
    fn export(
        &self,
        file_id: &str,
        mime_type: &str,
        output: &Path,
        resource_key: Option<&str>,
    ) -> Result<(), GoogleWorkspaceError> {
        assert!(!file_id.is_empty());
        assert!(!mime_type.is_empty());
        if file_id == "doc-1" {
            assert_eq!(resource_key, Some("rk-1"));
        }
        match &self.body {
            Ok(body) => {
                fs::write(output, body).map_err(|source| GoogleWorkspaceError::WriteSidecar {
                    path: output.to_path_buf(),
                    source,
                })
            }
            Err(message) => Err(GoogleWorkspaceError::Export {
                file_id: file_id.to_owned(),
                message: (*message).to_owned(),
            }),
        }
    }
}

#[test]
fn shortcut_reader_covers_identifier_precedence_url_resource_and_python_scalar_shapes()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    assert_eq!(GOOGLE_WORKSPACE_EXTENSIONS.len(), 3);
    for name in ["a.gdoc", "b.GSHEET", "c.gslides"] {
        assert!(is_google_workspace_path(Path::new(name)));
    }
    assert!(!is_google_workspace_path(Path::new("notes.md")));
    assert!(!google_workspace_enabled(Some("off")));

    let cases = [
        (
            "direct.gdoc",
            r#"{"doc_id":"doc-direct","file_id":"ignored","resourceKey":"key","email":true}"#,
            "doc-direct",
            Some("key"),
        ),
        (
            "query.gdoc",
            r#"{"url":"https://drive.google.com/open?id=query-id&resourcekey=query-key","email":7}"#,
            "query-id",
            Some("query-key"),
        ),
        (
            "path.gslides",
            r#"{"url":"https://docs.google.com/presentation/d/path-id/edit","email":["a",true,null]}"#,
            "path-id",
            None,
        ),
        (
            "resource.gsheet",
            r#"{"resource_id":"drive:resource-id","email":{"team":"graph"}}"#,
            "resource-id",
            None,
        ),
        (
            "numeric.gdoc",
            r#"{"fileId":42,"resource_key":false,"email":0}"#,
            "42",
            None,
        ),
    ];
    for (name, body, id, key) in cases {
        let path = directory.path().join(name);
        fs::write(&path, body)?;
        let shortcut = read_google_shortcut(&path)?;
        assert_eq!(shortcut.file_id, id);
        assert_eq!(shortcut.resource_key.as_deref(), key);
    }

    let non_object = directory.path().join("array.gdoc");
    fs::write(&non_object, "[]")?;
    assert!(matches!(
        read_google_shortcut(&non_object),
        Err(GoogleWorkspaceError::Read { .. })
    ));
    let invalid = directory.path().join("invalid.gdoc");
    fs::write(&invalid, "{")?;
    assert!(matches!(
        read_google_shortcut(&invalid),
        Err(GoogleWorkspaceError::Read { .. })
    ));
    let missing_id = directory.path().join("missing.gdoc");
    fs::write(&missing_id, r#"{"url":"https://example.com/no-id"}"#)?;
    assert!(matches!(
        read_google_shortcut(&missing_id),
        Err(GoogleWorkspaceError::MissingFileId(_))
    ));
    assert!(matches!(
        read_google_shortcut(&directory.path().join("absent.gdoc")),
        Err(GoogleWorkspaceError::Read { .. })
    ));
    Ok(())
}

#[test]
fn conversion_covers_documents_slides_empty_exports_errors_and_sheet_validation()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let output = directory.path().join("converted");

    let document = directory.path().join("Notes.gdoc");
    fs::write(
        &document,
        r#"{"doc_id":"doc-1","url":"https://docs.google.com/document/d/doc-1/edit","resource_key":"rk-1","email":"person@example.com"}"#,
    )?;
    let sidecar = convert_google_workspace_file_with(
        &document,
        &output,
        &FixtureExporter {
            body: Ok(b"# Notes\n\nConverted body.\n".to_vec()),
        },
    )?
    .ok_or("document sidecar missing")?;
    let text = fs::read_to_string(sidecar)?;
    assert!(text.contains("source_type: \"google_workspace\""));
    assert!(text.contains("google_file_id: \"doc-1\""));
    assert!(text.contains("google_account_hash:"));
    assert!(text.contains("Converted body."));

    let slides = directory.path().join("Deck.gslides");
    fs::write(&slides, r#"{"id":"slides-1"}"#)?;
    assert!(
        convert_google_workspace_file_with(
            &slides,
            &output,
            &FixtureExporter {
                body: Ok(b"Slide one\n".to_vec())
            }
        )?
        .is_some()
    );
    assert!(
        convert_google_workspace_file_with(
            &slides,
            &output,
            &FixtureExporter {
                body: Ok(b"  \n".to_vec())
            }
        )?
        .is_none()
    );

    let sheet = directory.path().join("Budget.gsheet");
    fs::write(&sheet, r#"{"file_id":"sheet-1"}"#)?;
    assert!(matches!(
        convert_google_workspace_file_with(
            &sheet,
            &output,
            &FixtureExporter {
                body: Ok(b"not an xlsx".to_vec())
            }
        ),
        Err(GoogleWorkspaceError::Sheet(_))
    ));

    assert!(matches!(
        convert_google_workspace_file_with(
            &document,
            &output,
            &FixtureExporter {
                body: Err("fixture export failure")
            }
        ),
        Err(GoogleWorkspaceError::Export { .. })
    ));
    assert!(
        convert_google_workspace_file_with(
            &directory.path().join("notes.md"),
            &output,
            &FixtureExporter {
                body: Ok(Vec::new())
            }
        )?
        .is_none()
    );
    Ok(())
}
