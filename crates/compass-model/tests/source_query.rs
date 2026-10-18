use compass_model::provenance::SourceAnchor;
use compass_model::source_query::{
    SourceQueryArtifact, SourceQueryFile, SourceQueryOwner, SourceQuerySite,
};

fn artifact() -> SourceQueryArtifact {
    let source = SourceAnchor {
        file: "src/sample.rs".to_owned(),
        start_byte: 0,
        end_byte: 100,
        start_line: 1,
        start_column: 0,
        end_line: 4,
        end_column: 1,
    };
    let site = SourceQuerySite {
        source: SourceAnchor {
            file: source.file.clone(),
            start_byte: 12,
            end_byte: 17,
            start_line: 2,
            start_column: 4,
            end_line: 2,
            end_column: 9,
        },
        text: "café".to_owned(),
    };
    let owner = SourceQueryOwner {
        node_id: "owner".to_owned(),
        source,
        source_digest: format!("sha256:{}", "b".repeat(64)),
        identifiers: vec![site],
        comments: Vec::new(),
        code_windows: Vec::new(),
        omitted_code_windows: 0,
        nested_contexts: Vec::new(),
        omitted_nested_contexts: 0,
        omitted_sites: 0,
        truncated: false,
        syntax_recovery: false,
    };
    let mut artifact = SourceQueryArtifact::empty("a".repeat(64));
    artifact.additional_document_frequencies = owner
        .document()
        .terms()
        .into_iter()
        .map(|term| (term, 1))
        .collect();
    artifact.files.push(SourceQueryFile {
        file: owner.source.file.clone(),
        source_digest: owner.source_digest.clone(),
        byte_size: 100,
        language: "rust".to_owned(),
        truncated: false,
        syntax_recovery: false,
        omitted_owners: 0,
    });
    artifact.owners.push(owner);
    artifact
}

#[test]
fn immutable_v1_projection_stays_readable_and_unknown_profiles_fail()
-> Result<(), Box<dyn std::error::Error>> {
    use compass_model::source_query::{
        SOURCE_QUERY_PROFILE, SOURCE_QUERY_PROFILE_V1, SOURCE_QUERY_PROFILE_V2,
        SOURCE_QUERY_PROFILE_V3, SOURCE_QUERY_PROFILE_V4,
    };
    for profile in [
        SOURCE_QUERY_PROFILE_V1,
        SOURCE_QUERY_PROFILE_V2,
        SOURCE_QUERY_PROFILE_V3,
        SOURCE_QUERY_PROFILE_V4,
        SOURCE_QUERY_PROFILE,
    ] {
        let mut value = artifact();
        value.profile = profile.to_owned();
        let decoded: SourceQueryArtifact = serde_json::from_slice(&serde_json::to_vec(&value)?)?;
        decoded.validate()?;
        assert_eq!(decoded, value);
    }
    let mut value = artifact();
    value.profile = "source-query/999-future".to_owned();
    assert!(value.validate().is_err());
    Ok(())
}
#[test]
fn round_trip_keeps_source_sites_and_shared_field_terms() -> Result<(), Box<dyn std::error::Error>>
{
    let artifact = artifact();
    artifact.validate()?;
    assert!(!artifact.owners[0].document().fields["body_identifier"].is_empty());
    let bytes = serde_json::to_vec(&artifact)?;
    let decoded: SourceQueryArtifact = serde_json::from_slice(&bytes)?;
    decoded.validate()?;
    assert_eq!(decoded, artifact);
    assert_eq!(serde_json::to_vec(&decoded)?, bytes);
    Ok(())
}
#[test]
fn invalid_sources_coordinates_order_and_versions_fail_explicitly() {
    let mut a = artifact();
    a.schema = "compass.source-query/2".to_owned();
    assert!(a.validate().is_err());
    let mut a = artifact();
    a.profile = "unknown".to_owned();
    assert!(a.validate().is_err());
    let mut a = artifact();
    a.graph_digest = "x".repeat(64);
    assert!(a.validate().is_err());
    let mut a = artifact();
    a.owners[0].source.file = "../outside.rs".to_owned();
    assert!(a.validate().is_err());
    let mut a = artifact();
    a.owners[0].identifiers[0].source.end_column = 8;
    assert!(a.validate().is_err());
    let mut a = artifact();
    a.owners[0].identifiers[0].source.end_byte = 101;
    assert!(a.validate().is_err());
    let mut a = artifact();
    a.owners.push(a.owners[0].clone());
    assert!(a.validate().is_err());
    let mut a = artifact();
    let duplicate = a.owners[0].identifiers[0].clone();
    a.owners[0].identifiers.push(duplicate);
    assert!(a.validate().is_err());
    let mut a = artifact();
    a.additional_document_frequencies.clear();
    assert!(a.validate().is_err());
    let mut a = artifact();
    for df in a.additional_document_frequencies.values_mut() {
        *df = 2;
    }
    assert!(a.validate().is_err());
}
#[test]
fn field_caps_report_omissions_without_changing_literal_excerpts() {
    let mut a = artifact();
    let owner = &mut a.owners[0];
    owner.comments = (0..8)
        .map(|_| SourceQuerySite {
            source: owner.source.clone(),
            text: "a".repeat(4096),
        })
        .collect();
    let document = owner.document();
    assert!(document.truncated);
    assert_eq!(owner.comments[0].text.len(), 4096);
    assert!(document.fields["body_comment"].len() <= 512);
}

#[test]
fn legacy_code_windows_remain_inspection_only_and_older_profiles_reject_them()
-> Result<(), Box<dyn std::error::Error>> {
    let mut value = artifact();
    value.profile = compass_model::source_query::SOURCE_QUERY_PROFILE_V4.to_owned();
    let document = value.owners[0].document_for_profile(&value.profile)?;
    let mut window = value.owners[0].identifiers[0].clone();
    window.text = "unindexed_literal".to_owned();
    window.source.end_byte = window.source.start_byte + window.text.len() as u64;
    window.source.end_column = window.source.start_column + window.text.len() as u32;
    value.owners[0].code_windows = vec![window];
    value.validate()?;
    assert_eq!(
        document,
        value.owners[0].document_for_profile(&value.profile)?
    );
    let encoded = serde_json::to_vec(&value)?;
    assert_eq!(
        serde_json::from_slice::<SourceQueryArtifact>(&encoded)?,
        value
    );
    let mut historical = value.clone();
    historical.profile = compass_model::source_query::SOURCE_QUERY_PROFILE_V3.to_owned();
    historical.validate()?;
    let mut bad = value.clone();
    bad.profile = compass_model::source_query::SOURCE_QUERY_PROFILE_V2.to_owned();
    assert!(bad.validate().is_err());
    let mut bad = value.clone();
    bad.owners[0].code_windows[0].source.file = "other.rs".to_owned();
    assert!(bad.validate().is_err());
    let mut bad = value.clone();
    bad.owners[0].code_windows[0].text = "x".repeat(513);
    assert!(bad.validate().is_err());
    let mut bad = value;
    bad.owners[0].code_windows[0].source.end_column += 1;
    assert!(bad.validate().is_err());
    Ok(())
}

#[test]
fn nested_context_is_versioned_bounded_and_cannot_change_retrieval_terms()
-> Result<(), Box<dyn std::error::Error>> {
    use compass_model::source_query::{SOURCE_QUERY_PROFILE_V3, SourceQueryNestedContext};
    let mut value = artifact();
    let document = value.owners[0].document();
    let window = value.owners[0].identifiers[0].clone();
    let mut source = value.owners[0].source.clone();
    source.start_byte = 10;
    source.end_byte = 40;
    source.start_line = 2;
    source.start_column = 0;
    source.end_line = 3;
    source.end_column = 0;
    value.owners[0]
        .nested_contexts
        .push(SourceQueryNestedContext {
            source,
            syntax: "closure_expression".to_owned(),
            code_windows: vec![window],
            omitted_code_windows: 0,
        });
    value.validate()?;
    assert_eq!(document, value.owners[0].document());
    let roundtrip: SourceQueryArtifact = serde_json::from_slice(&serde_json::to_vec(&value)?)?;
    assert_eq!(roundtrip, value);
    let mut bad = value.clone();
    bad.profile = SOURCE_QUERY_PROFILE_V3.to_owned();
    assert!(bad.validate().is_err());
    let mut bad = value.clone();
    bad.owners[0].nested_contexts[0].source = bad.owners[0].source.clone();
    assert!(bad.validate().is_err());
    let mut bad = value.clone();
    bad.owners[0].nested_contexts[0].source.file = "other.rs".to_owned();
    assert!(bad.validate().is_err());
    let mut bad = value.clone();
    bad.files[0].language = "python".to_owned();
    assert!(bad.validate().is_err());
    let mut bad = value.clone();
    bad.owners[0].nested_contexts[0].syntax = "function_item".to_owned();
    assert!(bad.validate().is_err());
    let mut bad = value.clone();
    bad.owners[0].nested_contexts[0].code_windows[0]
        .source
        .end_column += 1;
    assert!(bad.validate().is_err());
    let mut bad = value.clone();
    bad.owners[0].nested_contexts[0].code_windows[0]
        .source
        .end_byte = 41;
    assert!(bad.validate().is_err());
    let mut bad = value.clone();
    bad.owners[0]
        .nested_contexts
        .resize(9, value.owners[0].nested_contexts[0].clone());
    assert!(bad.validate().is_err());
    let mut bad = value;
    let template = bad.owners[0].nested_contexts[0].code_windows[0].clone();
    bad.owners[0].nested_contexts[0].code_windows = (0..17)
        .map(|at| {
            let mut site = template.clone();
            site.text = "x".to_owned();
            site.source.start_byte = 20 + at;
            site.source.end_byte = 21 + at;
            site.source.start_column = 12 + at as u32;
            site.source.end_column = 13 + at as u32;
            site
        })
        .collect();
    assert!(bad.validate().is_err());
    Ok(())
}

#[test]
fn code_lexemes_are_versioned_deduplicated_bounded_and_not_nested()
-> Result<(), Box<dyn std::error::Error>> {
    use compass_model::source_query::{SOURCE_QUERY_PROFILE, SOURCE_QUERY_PROFILE_V4};
    let mut value = artifact();
    let mut window = value.owners[0].identifiers[0].clone();
    window.text = "café retired_account_access".to_owned();
    window.source.end_byte = window.source.start_byte + window.text.len() as u64;
    window.source.end_column = window.source.start_column + window.text.len() as u32;
    value.owners[0].code_windows.push(window);
    let legacy = value.owners[0].document_for_profile(SOURCE_QUERY_PROFILE_V4)?;
    let current = value.owners[0].document_for_profile(SOURCE_QUERY_PROFILE)?;
    assert!(!legacy.fields.contains_key("body_code"));
    assert!(!current.fields["body_code"].is_empty());
    assert!(current.fields["body_code"].is_disjoint(&current.fields["body_identifier"]));
    assert!(
        current.fields["body_code"]
            .contains(&compass_model::canonical_code_token("retired".to_owned()))
    );
    assert!(
        value.owners[0]
            .document_for_profile("source-query/999")
            .is_err()
    );
    value.additional_document_frequencies =
        current.terms().into_iter().map(|term| (term, 1)).collect();
    value.validate()?;
    value.profile = SOURCE_QUERY_PROFILE_V4.to_owned();
    // New vocabulary cannot silently enter an immutable older profile's DF.
    assert!(value.validate().is_err());
    value.additional_document_frequencies =
        legacy.terms().into_iter().map(|term| (term, 1)).collect();
    value.validate()?;
    value.owners[0].omitted_code_windows = 1;
    assert!(
        !value.owners[0]
            .document_for_profile(SOURCE_QUERY_PROFILE_V4)?
            .truncated
    );
    assert!(
        value.owners[0]
            .document_for_profile(SOURCE_QUERY_PROFILE)?
            .truncated
    );
    Ok(())
}
