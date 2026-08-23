use std::error::Error;
use std::fs;
use std::io::Write as _;
use std::path::Path;
use std::process::Command;

use serde_json::{Value, json};
use zip::write::SimpleFileOptions;

fn write_docx(path: &Path) -> Result<(), Box<dyn Error>> {
    let file = fs::File::create(path)?;
    let mut archive = zip::ZipWriter::new(file);
    archive.start_file("word/document.xml", SimpleFileOptions::default())?;
    archive.write_all(br#"<w:document xmlns:w="urn:w"><w:body><w:p><w:r><w:t>CLI document sentinel</w:t></w:r></w:p></w:body></w:document>"#)?;
    archive.finish()?;
    Ok(())
}

#[test]
fn document_inspect_json_is_native_local_and_path_safe() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let document = directory.path().join("contract.docx");
    write_docx(&document)?;
    let model_cache = directory.path().join("empty-model-cache");
    let output = Command::new(env!("CARGO_BIN_EXE_compass"))
        .args([
            "document",
            "inspect",
            document.to_str().ok_or("non-UTF-8 test path")?,
            "--format",
            "json",
            "--ocr-language",
            "EN-us",
            "--ocr-language",
            "en-US",
            "--allow-partial",
        ])
        .env("COMPASS_CACHE_DIR", &model_cache)
        .output()?;
    assert!(
        output.status.success(),
        "document inspect failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(value["schema"], "compass.document.inspect/1");
    assert_eq!(value["source"], "contract.docx");
    assert_eq!(value["processing"]["ocr_mode"], "off");
    assert_eq!(value["processing"]["allow_partial"], true);
    assert_eq!(value["processing"]["language_hints"], json!(["en-us"]));
    assert_eq!(value["artifact"]["schema"], "compass.document/1");
    assert!(
        value["artifact"]["blocks"]
            .as_array()
            .is_some_and(|blocks| {
                blocks
                    .iter()
                    .any(|block| block["text"] == "CLI document sentinel")
            })
    );
    let serialized = String::from_utf8(output.stdout)?;
    assert!(!serialized.contains(&directory.path().to_string_lossy().into_owned()));
    assert!(!model_cache.exists());
    Ok(())
}

#[test]
fn explicit_ocr_missing_profile_has_one_actionable_command() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let document = directory.path().join("scan.docx");
    write_docx(&document)?;
    let output = Command::new(env!("CARGO_BIN_EXE_compass"))
        .args([
            "document",
            "inspect",
            document.to_str().ok_or("non-UTF-8 test path")?,
            "--ocr",
            "auto",
        ])
        .env(
            "COMPASS_CACHE_DIR",
            directory.path().join("empty-model-cache"),
        )
        .output()?;
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr)?;
    let command = "compass models install pp-ocrv6-small";
    assert_eq!(stderr.matches(command).count(), 1, "{stderr}");
    assert!(stderr.contains("no system OCR package is required"));
    Ok(())
}

#[test]
fn model_listing_and_verification_are_offline() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let cache = directory.path().join("models");
    let listed = Command::new(env!("CARGO_BIN_EXE_compass"))
        .args(["models", "list", "--format", "json"])
        .env("COMPASS_CACHE_DIR", &cache)
        .output()?;
    assert!(listed.status.success());
    let value: Value = serde_json::from_slice(&listed.stdout)?;
    assert_eq!(value["schema"], "compass.models/1");
    assert_eq!(value["profiles"].as_array().map(Vec::len), Some(2));
    let verified = Command::new(env!("CARGO_BIN_EXE_compass"))
        .args(["models", "verify", "pp-ocrv6-small"])
        .env("COMPASS_CACHE_DIR", &cache)
        .output()?;
    assert!(!verified.status.success());
    assert!(
        !cache.exists(),
        "offline verification created the model cache"
    );
    Ok(())
}

#[test]
fn extract_ocr_uses_the_same_managed_profile_contract() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    write_docx(&directory.path().join("report.docx"))?;
    let output = Command::new(env!("CARGO_BIN_EXE_compass"))
        .args([
            "extract",
            directory.path().to_str().ok_or("non-UTF-8 test path")?,
            "--ocr",
            "auto",
            "--no-viz",
            "--no-cluster",
        ])
        .env(
            "COMPASS_CACHE_DIR",
            directory.path().join("empty-model-cache"),
        )
        .output()?;
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr)?;
    assert_eq!(
        stderr
            .matches("compass models install pp-ocrv6-small")
            .count(),
        1
    );
    assert!(stderr.contains("no system OCR package is required"));
    Ok(())
}

#[test]
fn document_inspect_rejects_non_files_without_publishing_output() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let fake_document = directory.path().join("directory.docx");
    fs::create_dir(&fake_document)?;
    let output = Command::new(env!("CARGO_BIN_EXE_compass"))
        .args([
            "document",
            "inspect",
            fake_document.to_str().ok_or("non-UTF-8 test path")?,
            "--format",
            "json",
        ])
        .output()?;
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8(output.stderr)?.contains("not a regular file"));
    Ok(())
}
