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
        parameter_bindings: Some(Default::default()),
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
        SOURCE_QUERY_PROFILE_V3, SOURCE_QUERY_PROFILE_V4, SOURCE_QUERY_PROFILE_V5,
    };
    for profile in [
        SOURCE_QUERY_PROFILE_V1,
        SOURCE_QUERY_PROFILE_V2,
        SOURCE_QUERY_PROFILE_V3,
        SOURCE_QUERY_PROFILE_V4,
        SOURCE_QUERY_PROFILE_V5,
        SOURCE_QUERY_PROFILE,
    ] {
        let mut value = artifact();
        value.profile = profile.to_owned();
        if profile != SOURCE_QUERY_PROFILE {
            value
                .owners
                .iter_mut()
                .for_each(|owner| owner.parameter_bindings = None);
        }
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
    value
        .owners
        .iter_mut()
        .for_each(|owner| owner.parameter_bindings = None);
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
    historical
        .owners
        .iter_mut()
        .for_each(|owner| owner.parameter_bindings = None);
    historical.validate()?;
    let mut bad = value.clone();
    bad.profile = compass_model::source_query::SOURCE_QUERY_PROFILE_V2.to_owned();
    bad.owners
        .iter_mut()
        .for_each(|owner| owner.parameter_bindings = None);
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
    bad.owners
        .iter_mut()
        .for_each(|owner| owner.parameter_bindings = None);
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
    value
        .owners
        .iter_mut()
        .for_each(|owner| owner.parameter_bindings = None);
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

#[test]
fn parameter_bindings_are_versioned_bounded_and_do_not_change_lexical_statistics()
-> Result<(), Box<dyn std::error::Error>> {
    use compass_model::source_query::*;
    let mut value = artifact();
    let before = value.owners[0].document();
    value.owners[0]
        .parameter_bindings
        .as_mut()
        .ok_or("missing current coverage")?
        .sites = value.owners[0].identifiers.clone();
    value.validate()?;
    assert_eq!(value.owners[0].document(), before);
    let bytes = serde_json::to_vec(&value)?;
    assert_eq!(
        serde_json::from_slice::<SourceQueryArtifact>(&bytes)?,
        value
    );
    let mut missing = value.clone();
    missing.owners[0].parameter_bindings = None;
    assert!(missing.validate().is_err());
    let mut invalid = value.clone();
    invalid.owners[0]
        .parameter_bindings
        .as_mut()
        .ok_or("missing coverage")?
        .omitted_sites = 1;
    assert!(invalid.validate().is_err());
    let mut invalid = value.clone();
    invalid.owners[0]
        .parameter_bindings
        .as_mut()
        .ok_or("missing coverage")?
        .sites
        .push(value.owners[0].identifiers[0].clone());
    assert!(invalid.validate().is_err());
    for profile in [
        SOURCE_QUERY_PROFILE_V1,
        SOURCE_QUERY_PROFILE_V2,
        SOURCE_QUERY_PROFILE_V3,
        SOURCE_QUERY_PROFILE_V4,
        SOURCE_QUERY_PROFILE_V5,
    ] {
        let mut legacy = value.clone();
        legacy.profile = profile.to_owned();
        assert!(legacy.validate().is_err());
        legacy.owners[0].parameter_bindings = None;
        legacy.validate()?;
    }
    let mut raw = serde_json::to_value(&value)?;
    raw["owners"][0]["parameterBindings"]["status"] = serde_json::json!("future");
    assert!(serde_json::from_value::<SourceQueryArtifact>(raw).is_err());
    Ok(())
}

#[test]
fn advancing_current_profile_keeps_v5_code_vocabulary_and_old_documents()
-> Result<(), Box<dyn std::error::Error>> {
    use compass_model::source_query::*;
    let mut value = artifact();
    let mut code = value.owners[0].identifiers[0].clone();
    code.text = "retired".to_owned();
    code.source.end_byte = code.source.start_byte + code.text.len() as u64;
    code.source.end_column = code.source.start_column + code.text.len() as u32;
    value.owners[0].code_windows.push(code);
    let current = value.owners[0].document_for_profile(SOURCE_QUERY_PROFILE)?;
    let v5 = value.owners[0].document_for_profile(SOURCE_QUERY_PROFILE_V5)?;
    assert_eq!(current, v5);
    assert!(v5.fields.contains_key("body_code"));
    assert!(
        !value.owners[0]
            .document_for_profile(SOURCE_QUERY_PROFILE_V4)?
            .fields
            .contains_key("body_code")
    );
    Ok(())
}

#[test]
fn parameter_coverage_and_sites_reject_corrupt_payloads() -> Result<(), Box<dyn std::error::Error>>
{
    use compass_model::source_query::*;
    let mut baseline = artifact();
    baseline.owners[0]
        .parameter_bindings
        .as_mut()
        .ok_or("missing parameter coverage")?
        .sites = baseline.owners[0].identifiers.clone();
    baseline.validate()?;
    let raw = serde_json::to_value(&baseline)?;
    for (field, replacement) in [
        ("status", serde_json::json!("unsupported")),
        ("status", serde_json::json!("partial")),
        ("omittedSites", serde_json::json!(200_001)),
        ("reasons", serde_json::json!(["text_limit"])),
    ] {
        let mut bad = raw.clone();
        bad["owners"][0]["parameterBindings"][field] = replacement;
        assert!(
            serde_json::from_value::<SourceQueryArtifact>(bad)?
                .validate()
                .is_err()
        );
    }
    for text in ["", "two names", "default()", "bad\nname", "'literal'"] {
        let mut bad = raw.clone();
        bad["owners"][0]["parameterBindings"]["sites"][0]["text"] = serde_json::json!(text);
        assert!(
            serde_json::from_value::<SourceQueryArtifact>(bad)?
                .validate()
                .is_err()
        );
    }
    for (field, replacement) in [
        ("file", serde_json::json!("other.rs")),
        ("endByte", serde_json::json!(101)),
        ("endColumn", serde_json::json!(10)),
    ] {
        let mut bad = raw.clone();
        bad["owners"][0]["parameterBindings"]["sites"][0]["source"][field] = replacement;
        assert!(
            serde_json::from_value::<SourceQueryArtifact>(bad)?
                .validate()
                .is_err()
        );
    }
    let mut bad = raw.clone();
    bad["owners"][0]["parameterBindings"]["reasons"] = serde_json::json!(["future"]);
    assert!(serde_json::from_value::<SourceQueryArtifact>(bad).is_err());
    let mut oversized = baseline.clone();
    oversized.owners[0]
        .parameter_bindings
        .as_mut()
        .ok_or("missing parameter coverage")?
        .sites = vec![baseline.owners[0].identifiers[0].clone(); 65];
    assert!(oversized.validate().is_err());
    for status in [
        ParameterBindingStatus::Partial,
        ParameterBindingStatus::Unsupported,
    ] {
        let mut limited = baseline.clone();
        let bindings = limited.owners[0]
            .parameter_bindings
            .as_mut()
            .ok_or("missing coverage")?;
        bindings.sites.clear();
        bindings.status = status;
        bindings
            .reasons
            .insert(ParameterBindingOmission::TraversalLimit);
        // Traversal can stop before the size of an omitted suffix is known.
        bindings.omitted_sites = 0;
        limited.validate()?;
        assert_eq!(limited.owners[0].document(), baseline.owners[0].document());
    }
    Ok(())
}

#[test]
fn parameter_spellings_are_whole_language_scoped_identifiers()
-> Result<(), Box<dyn std::error::Error>> {
    for (language, text, valid) in [
        ("python", "café", true),
        ("python", "e\u{301}clair", true),
        ("go", "κλειδί", true),
        ("typescript", "$block", true),
        ("tsx", "$$permit2", true),
        ("rust", "r#type", true),
        ("rust", "r#", false),
        ("python", "r#type", false),
        ("python", "$block", false),
        ("rust", "block+permit", false),
        ("rust", "block/permit", false),
        ("rust", "block.foo", false),
        ("rust", "block::permit", false),
        ("rust", "123", false),
        ("typescript", "\\u0062lock", false),
    ] {
        let mut value = artifact();
        value.files[0].language = language.to_owned();
        let mut binding = value.owners[0].identifiers[0].clone();
        binding.text = text.to_owned();
        binding.source.end_byte = binding.source.start_byte + text.len() as u64;
        binding.source.end_column = binding.source.start_column + text.len() as u32;
        value.owners[0]
            .parameter_bindings
            .as_mut()
            .ok_or("missing coverage")?
            .sites = vec![binding];
        assert_eq!(value.validate().is_ok(), valid, "{language}: {text}");
    }
    Ok(())
}

#[test]
fn parameter_coverage_state_machine_distinguishes_known_and_unknown_omissions()
-> Result<(), Box<dyn std::error::Error>> {
    use compass_model::source_query::{ParameterBindingOmission as R, ParameterBindingStatus as S};
    for (status, count, reasons, valid) in [
        (S::Complete, 0, vec![], true),
        (S::Complete, 1, vec![], false),
        (S::Complete, 0, vec![R::TraversalLimit], false),
        (S::Partial, 0, vec![R::TraversalLimit], true),
        (
            S::Partial,
            0,
            vec![R::UnsupportedPattern, R::SyntaxRecovery],
            true,
        ),
        (S::Partial, 1, vec![R::TraversalLimit], false),
        (S::Partial, 1, vec![R::TextLimit], true),
        (S::Partial, 0, vec![R::TextLimit], false),
        (S::Partial, 1, vec![R::UnsupportedSpelling], true),
        (S::Partial, 0, vec![R::UnsupportedSpelling], false),
        (
            S::Partial,
            2,
            vec![R::SiteLimit, R::UnsupportedPattern],
            true,
        ),
        (
            S::Partial,
            0,
            vec![R::SiteLimit, R::UnsupportedPattern],
            false,
        ),
        (S::Partial, 0, vec![], false),
        (S::Unsupported, 0, vec![R::UnsupportedPattern], true),
        (
            S::Unsupported,
            0,
            vec![R::UnsupportedReceiver, R::TraversalLimit],
            true,
        ),
        (S::Unsupported, 0, vec![R::SiteLimit], false),
        (S::Unsupported, 1, vec![R::AggregateLimit], false),
        (
            S::Unsupported,
            0,
            vec![R::UnsupportedPattern, R::TextLimit],
            false,
        ),
        (S::Unsupported, 0, vec![], false),
    ] {
        let mut value = artifact();
        let bindings = value.owners[0]
            .parameter_bindings
            .as_mut()
            .ok_or("missing coverage")?;
        bindings.status = status;
        bindings.omitted_sites = count;
        bindings.reasons = reasons.into_iter().collect();
        let description = format!("{bindings:?}");
        assert_eq!(value.validate().is_ok(), valid, "{description}");
    }
    Ok(())
}

#[test]
fn parameter_sites_are_charged_to_file_and_shared_artifact_limits()
-> Result<(), Box<dyn std::error::Error>> {
    for (file_count, owners_per_file, count, width, error) in [
        (1, 16, 64, 256, None),
        (1, 17, 64, 256, Some("per-file parameter binding limit")),
        (1025, 1, 62, 1, None),
        (1025, 1, 64, 1, Some("aggregate source evidence limit")),
    ] {
        let mut value = artifact();
        let original_owner = value.owners[0].clone();
        let original_file = value.files[0].clone();
        value.owners.clear();
        value.files.clear();
        for file_index in 0..file_count {
            let name = format!("file{file_index:04}.rs");
            let mut file = original_file.clone();
            file.file = name.clone();
            file.byte_size = (count * (width + 1) + 64) as u64;
            value.files.push(file);
            for owner_index in 0..owners_per_file {
                let mut owner = original_owner.clone();
                owner.node_id = format!("owner{file_index:04}-{owner_index:04}");
                owner.source.file = name.clone();
                owner.source.end_byte = value.files[file_index].byte_size;
                owner.identifiers[0].source.file = name.clone();
                let sites = (0..count)
                    .map(|i| {
                        let mut site = owner.identifiers[0].clone();
                        site.text = "a".repeat(width);
                        site.source.start_byte = (32 + i * (width + 1)) as u64;
                        site.source.end_byte = site.source.start_byte + width as u64;
                        site.source.start_column = site.source.start_byte as u32;
                        site.source.end_column = site.source.end_byte as u32;
                        site
                    })
                    .collect();
                owner
                    .parameter_bindings
                    .as_mut()
                    .ok_or("missing role coverage")?
                    .sites = sites;
                value.owners.push(owner);
            }
        }
        for frequency in value.additional_document_frequencies.values_mut() {
            *frequency = value.owners.len() as u64;
        }
        if let Some(message) = error {
            let failure = value.validate().err().ok_or("missing role limit error")?;
            assert!(failure.to_string().contains(message), "{failure}");
        } else {
            value.validate()?;
        }
    }
    Ok(())
}

#[test]
fn current_per_file_text_boundary_preserves_historical_v5_limits()
-> Result<(), Box<dyn std::error::Error>> {
    use compass_model::source_query::*;
    let mut value = artifact();
    let template = value.owners[0].clone();
    value.owners.clear();
    value.files[0].byte_size = 4 * 1024 * 1024;
    let make_site = |start: usize, width: usize, letter: &str| SourceQuerySite {
        source: SourceAnchor {
            file: template.source.file.clone(),
            start_byte: start as u64,
            end_byte: (start + width) as u64,
            start_line: 1,
            end_line: 1,
            start_column: start as u32,
            end_column: (start + width) as u32,
        },
        text: letter.repeat(width),
    };
    for index in 0..4 {
        let mut owner = template.clone();
        owner.node_id = format!("owner-{index}");
        owner.source.end_byte = value.files[0].byte_size;
        owner.source.end_line = 1;
        owner.source.end_column = value.files[0].byte_size as u32;
        let count = if index < 3 { 1024 } else { 640 };
        owner.identifiers = (0..count).map(|i| make_site(i * 256, 256, "a")).collect();
        owner.comments = if index < 3 {
            (0..8)
                .map(|i| make_site(1024 * 256 + i * 4096, 4096, "b"))
                .collect()
        } else {
            Vec::new()
        };
        value.owners.push(owner);
    }
    let mut frequencies = std::collections::BTreeMap::<String, u64>::new();
    for owner in &value.owners {
        for term in owner.document().terms() {
            *frequencies.entry(term).or_default() += 1;
        }
    }
    value.additional_document_frequencies = frequencies;
    assert_eq!(
        value
            .owners
            .iter()
            .flat_map(|o| o.identifiers.iter().chain(&o.comments))
            .map(|s| s.text.len())
            .sum::<usize>(),
        SOURCE_QUERY_MAX_FILE_TEXT_BYTES
    );
    value.validate()?;
    value.owners[3]
        .identifiers
        .push(make_site(640 * 256, 1, "a"));
    let error = value.validate().err().ok_or("per-file +1 was accepted")?;
    assert!(
        error
            .to_string()
            .contains("per-file lexical and parameter limit")
    );
    value.profile = SOURCE_QUERY_PROFILE_V5.to_owned();
    for owner in &mut value.owners {
        owner.parameter_bindings = None;
    }
    value.additional_document_frequencies.clear();
    for owner in &value.owners {
        for term in owner.document_for_profile(SOURCE_QUERY_PROFILE_V5)?.terms() {
            *value
                .additional_document_frequencies
                .entry(term)
                .or_default() += 1;
        }
    }
    value.validate()?;
    Ok(())
}
