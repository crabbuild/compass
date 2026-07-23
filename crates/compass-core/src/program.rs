use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use compass_files::{Cache, CacheKind, FileError, write_bytes_atomic};
use compass_ir::{ProviderDescriptor, canonical_json_bytes, hex_sha256};
use compass_languages::{Registry, TREE_SITTER_PROGRAM_PROVIDER_VERSION, TreeSitterSyntaxProvider};
use compass_program::{
    ArtifactInput, ArtifactProvider, DecodedScipArtifact, DecodedScipDocument, EvidenceBatch,
    OfficialScipProvider, SyntaxProvider, merge_evidence, parse_artifact_manifest,
};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{BuildOptions, CoreError};

pub(crate) const PROGRAM_ARTIFACT: &str = "program.json";

#[derive(Debug)]
pub(crate) struct ProgramBuild {
    pub analysis: compass_analysis::AnalysisBundle,
    pub syntax_analyzed: usize,
    pub syntax_reused: usize,
    pub artifacts_loaded: usize,
    pub artifacts_reused: usize,
    pub artifact_documents_analyzed: usize,
    pub artifact_documents_reused: usize,
    pub conflicts: usize,
}

#[derive(Clone)]
struct SourceInput {
    source_file: String,
    language: String,
    bytes: Vec<u8>,
    digest: String,
    cache_key: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ArtifactCacheHeader {
    metadata_protobuf_base64: String,
    documents: Vec<String>,
}

struct ArtifactCacheContext<'a> {
    cache: &'a Cache,
    kind: &'a CacheKind,
    artifact_digest: &'a str,
    live: &'a mut BTreeSet<String>,
}

#[derive(Default)]
struct DocumentCacheStats {
    analyzed: usize,
    reused: usize,
}

pub(crate) fn build_program(
    root: &Path,
    sources: &[PathBuf],
    options: &BuildOptions,
    cache: &Cache,
) -> Result<ProgramBuild, CoreError> {
    let inputs = read_sources(root, sources)?;
    let source_digests = inputs
        .iter()
        .map(|input| (input.source_file.clone(), input.digest.clone()))
        .collect::<BTreeMap<_, _>>();
    let source_texts = inputs
        .iter()
        .map(|input| (input.source_file.clone(), input.bytes.clone()))
        .collect::<BTreeMap<_, _>>();

    let syntax_kind = CacheKind::ProgramSyntax {
        ir_schema: compass_ir::PROGRAM_SCHEMA_VERSION,
        provider_version: format!("tree-sitter/{TREE_SITTER_PROGRAM_PROVIDER_VERSION}"),
    };
    let mut batches = Vec::new();
    let mut missing = Vec::new();
    let mut syntax_reused = 0;
    let mut live_syntax_keys = BTreeSet::new();
    for input in &inputs {
        if !supports_program_syntax(&input.source_file) {
            continue;
        }
        live_syntax_keys.insert(input.cache_key.clone());
        if !options.force
            && let Some(batch) =
                cache.load_program::<EvidenceBatch>(&syntax_kind, &input.cache_key)?
            && valid_syntax_batch(&batch, &input.source_file)
        {
            batches.push(batch);
            syntax_reused += 1;
            continue;
        }
        missing.push(input.clone());
    }
    let fresh = analyze_syntax(&missing, options.max_workers)?;
    let syntax_analyzed = fresh.len();
    for (key, batch) in fresh {
        cache.save_program(&syntax_kind, &key, &batch)?;
        batches.push(batch);
    }
    cache.prune_program(&syntax_kind, &live_syntax_keys)?;

    let artifact_kind = CacheKind::ProgramArtifact {
        ir_schema: compass_ir::PROGRAM_SCHEMA_VERSION,
        decoder_version: format!("scip/{}", compass_program::SCIP_PROVIDER_VERSION),
    };
    let artifacts = discover_artifacts(root, options)?;
    let mut live_artifact_keys = BTreeSet::new();
    let mut artifacts_loaded = 0;
    let mut artifacts_reused = 0;
    let mut artifact_documents_analyzed = 0;
    let mut artifact_documents_reused = 0;
    for artifact in artifacts {
        let manifest = load_manifest(&artifact.path, &artifact.digest)?;
        let logical_name = artifact
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("index.scip");
        let input = ArtifactInput {
            logical_name,
            input_digest: &artifact.digest,
            byte_len: artifact.byte_len,
            manifest: manifest.as_ref(),
            source_digests: &source_digests,
            source_texts: &source_texts,
            limits: options.program_artifact_limits,
        };
        let header_key = format!("{}:decoded:header", artifact.digest);
        let header = if !options.force {
            cache.load_program::<ArtifactCacheHeader>(&artifact_kind, &header_key)?
        } else {
            None
        };
        let header_was_reused = header.is_some();
        if header_was_reused {
            artifacts_reused += 1;
        }
        let mut document_stats = DocumentCacheStats::default();
        let mut batch = if let Some(cached_header) = &header {
            assemble_decoded_artifact(
                &mut ArtifactCacheContext {
                    cache,
                    kind: &artifact_kind,
                    artifact_digest: &artifact.digest,
                    live: &mut live_artifact_keys,
                },
                input,
                cached_header,
                None,
                &mut document_stats,
            )?
        } else {
            None
        };
        if batch.is_some() {
            artifact_documents_analyzed += document_stats.analyzed;
            artifact_documents_reused += document_stats.reused;
        }
        if batch.is_none() {
            if header_was_reused {
                artifacts_reused -= 1;
            }
            document_stats = DocumentCacheStats::default();
            let decoded = decode_artifact_file(&artifact, input)?;
            save_decoded_artifact(
                cache,
                &artifact_kind,
                &artifact.digest,
                &header_key,
                &decoded,
                &mut live_artifact_keys,
            )?;
            let fresh_header = ArtifactCacheHeader {
                metadata_protobuf_base64: decoded.metadata_protobuf_base64.clone(),
                documents: decoded
                    .documents
                    .iter()
                    .map(|document| document.path.clone())
                    .collect(),
            };
            batch = assemble_decoded_artifact(
                &mut ArtifactCacheContext {
                    cache,
                    kind: &artifact_kind,
                    artifact_digest: &artifact.digest,
                    live: &mut live_artifact_keys,
                },
                input,
                &fresh_header,
                Some(&decoded),
                &mut document_stats,
            )?;
            artifact_documents_analyzed += document_stats.analyzed;
            artifact_documents_reused += document_stats.reused;
            artifacts_loaded += 1;
        }
        let batch = batch.ok_or_else(|| {
            CoreError::InvalidProgramInput(format!(
                "SCIP decoded cache could not be reconstructed for {}",
                artifact.path.display()
            ))
        })?;
        batches.push(batch);
    }
    cache.prune_program(&artifact_kind, &live_artifact_keys)?;

    let provider_digest = provider_manifest_digest(&batches)?;
    let merge_kind = CacheKind::ProgramMerge {
        ir_schema: compass_ir::PROGRAM_SCHEMA_VERSION,
        merger_version: compass_program::MERGER_VERSION,
        analyzer_version: compass_analysis::ANALYZER_VERSION,
    };
    let analysis = if !options.force
        && let Some(cached) =
            cache.load_program::<compass_analysis::AnalysisBundle>(&merge_kind, &provider_digest)?
        && cached.validate().is_ok()
    {
        cached
    } else {
        let program = merge_evidence(batches)?;
        let analysis = compass_analysis::analyze(program)?;
        cache.save_program(&merge_kind, &provider_digest, &analysis)?;
        analysis
    };
    cache.prune_program(
        &merge_kind,
        &[provider_digest].into_iter().collect::<BTreeSet<_>>(),
    )?;
    let conflicts = count_conflicts(&analysis);
    Ok(ProgramBuild {
        analysis,
        syntax_analyzed,
        syntax_reused,
        artifacts_loaded,
        artifacts_reused,
        artifact_documents_analyzed,
        artifact_documents_reused,
        conflicts,
    })
}

pub(crate) fn load_current_program(
    root: &Path,
    sources: &[PathBuf],
    options: &BuildOptions,
    output_dir: &Path,
) -> Result<Option<ProgramBuild>, CoreError> {
    let path = output_dir.join(PROGRAM_ARTIFACT);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Ok(None),
    };
    let analysis = match serde_json::from_slice::<compass_analysis::AnalysisBundle>(&bytes) {
        Ok(analysis) => analysis,
        Err(_) => return Ok(None),
    };
    let canonical = match analysis.canonical_bytes() {
        Ok(canonical) => canonical,
        Err(_) => return Ok(None),
    };
    if canonical != bytes {
        return Ok(None);
    }

    let inputs = read_sources(root, sources)?;
    let source_digests = inputs
        .iter()
        .map(|input| (input.source_file.clone(), input.digest.clone()))
        .collect::<BTreeMap<_, _>>();
    let source_texts = inputs
        .iter()
        .map(|input| (input.source_file.clone(), input.bytes.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut providers = Vec::new();
    let syntax_provider = TreeSitterSyntaxProvider::default();
    let supported = inputs
        .iter()
        .filter(|input| supports_program_syntax(&input.source_file))
        .collect::<Vec<_>>();
    providers.extend(supported.iter().map(|input| {
        syntax_provider.descriptor(&compass_program::FileInput {
            source_file: &input.source_file,
            language: &input.language,
            source: &input.bytes,
        })
    }));
    let artifacts = discover_artifacts(root, options)?;
    for artifact in &artifacts {
        let manifest = load_manifest(&artifact.path, &artifact.digest)?;
        providers.push(
            OfficialScipProvider.descriptor(&ArtifactInput {
                logical_name: artifact
                    .path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("index.scip"),
                input_digest: &artifact.digest,
                byte_len: artifact.byte_len,
                manifest: manifest.as_ref(),
                source_digests: &source_digests,
                source_texts: &source_texts,
                limits: options.program_artifact_limits,
            }),
        );
    }
    providers.sort();
    providers.dedup();
    if analysis.program.providers != providers
        || analysis.program.modules.len() != supported.len()
        || !analysis.program.modules.iter().all(|module| {
            supported.iter().any(|input| {
                module.source_file == input.source_file
                    && module.language == input.language
                    && module.source_digest == input.digest
            })
        })
    {
        return Ok(None);
    }

    let conflicts = count_conflicts(&analysis);
    Ok(Some(ProgramBuild {
        analysis,
        syntax_analyzed: 0,
        syntax_reused: supported.len(),
        artifacts_loaded: 0,
        artifacts_reused: artifacts.len(),
        artifact_documents_analyzed: 0,
        artifact_documents_reused: 0,
        conflicts,
    }))
}

pub(crate) fn current_provider_manifest(
    root: &Path,
    sources: &[PathBuf],
    options: &BuildOptions,
) -> Result<Vec<ProviderDescriptor>, CoreError> {
    let inputs = read_sources(root, sources)?;
    let source_digests = inputs
        .iter()
        .map(|input| (input.source_file.clone(), input.digest.clone()))
        .collect::<BTreeMap<_, _>>();
    let source_texts = inputs
        .iter()
        .map(|input| (input.source_file.clone(), input.bytes.clone()))
        .collect::<BTreeMap<_, _>>();
    let syntax_provider = TreeSitterSyntaxProvider::default();
    let mut providers = inputs
        .iter()
        .filter(|input| supports_program_syntax(&input.source_file))
        .map(|input| {
            syntax_provider.descriptor(&compass_program::FileInput {
                source_file: &input.source_file,
                language: &input.language,
                source: &input.bytes,
            })
        })
        .collect::<Vec<_>>();
    for artifact in discover_artifacts(root, options)? {
        let manifest = load_manifest(&artifact.path, &artifact.digest)?;
        providers.push(
            OfficialScipProvider.descriptor(&ArtifactInput {
                logical_name: artifact
                    .path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("index.scip"),
                input_digest: &artifact.digest,
                byte_len: artifact.byte_len,
                manifest: manifest.as_ref(),
                source_digests: &source_digests,
                source_texts: &source_texts,
                limits: options.program_artifact_limits,
            }),
        );
    }
    providers.sort();
    providers.dedup();
    Ok(providers)
}

pub(crate) fn write_program(
    output_dir: &Path,
    analysis: &compass_analysis::AnalysisBundle,
) -> Result<(), CoreError> {
    write_bytes_atomic(
        output_dir.join(PROGRAM_ARTIFACT),
        &analysis.canonical_bytes()?,
    )?;
    Ok(())
}

fn read_sources(root: &Path, sources: &[PathBuf]) -> Result<Vec<SourceInput>, CoreError> {
    let mut inputs = Vec::with_capacity(sources.len());
    for path in sources {
        let canonical = fs::canonicalize(path).map_err(|source| FileError::Io {
            path: path.clone(),
            source,
        })?;
        let relative = canonical.strip_prefix(root).map_err(|_| {
            CoreError::InvalidProgramInput(format!(
                "source is outside repository: {}",
                path.display()
            ))
        })?;
        let source_file = compass_program::normalize_source_path(&relative.to_string_lossy())?;
        let bytes = fs::read(&canonical).map_err(|source| FileError::Io {
            path: canonical,
            source,
        })?;
        let digest = hex_sha256(&bytes);
        let language = Registry::resolve(Path::new(&source_file))
            .map_or("", |spec| spec.name)
            .to_owned();
        let cache_key = format!("{source_file}:{digest}");
        inputs.push(SourceInput {
            source_file,
            language,
            bytes,
            digest,
            cache_key,
        });
    }
    inputs.sort_by(|left, right| {
        left.source_file
            .as_bytes()
            .cmp(right.source_file.as_bytes())
    });
    Ok(inputs)
}

fn supports_program_syntax(source_file: &str) -> bool {
    Registry::resolve(Path::new(source_file))
        .is_some_and(|spec| matches!(spec.name, "rust" | "typescript" | "tsx" | "javascript"))
}

fn analyze_syntax(
    inputs: &[SourceInput],
    max_workers: Option<usize>,
) -> Result<Vec<(String, EvidenceBatch)>, CoreError> {
    let analyze = |provider: &mut TreeSitterSyntaxProvider,
                   input: &SourceInput|
     -> Result<(String, EvidenceBatch), CoreError> {
        let batch = provider
            .analyze_file(compass_program::FileInput {
                source_file: &input.source_file,
                language: &input.language,
                source: &input.bytes,
            })?
            .ok_or_else(|| {
                CoreError::InvalidProgramInput(format!(
                    "syntax provider rejected supported source {}",
                    input.source_file
                ))
            })?;
        Ok((input.cache_key.clone(), batch))
    };
    if inputs.len() < 256 {
        let mut provider = TreeSitterSyntaxProvider::default();
        return inputs
            .iter()
            .map(|input| analyze(&mut provider, input))
            .collect();
    }
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(max_workers.unwrap_or_else(num_cpus::get))
        .thread_name(|index| format!("compass-program-{index}"))
        .build()
        .map_err(|error| CoreError::WorkerPool(error.to_string()))?;
    pool.install(|| {
        inputs
            .par_iter()
            .map_init(TreeSitterSyntaxProvider::default, analyze)
            .collect()
    })
}

fn valid_syntax_batch(batch: &EvidenceBatch, source_file: &str) -> bool {
    batch.descriptor.scope == source_file
        && batch.modules.len() == 1
        && batch.modules[0].source_file == source_file
        && merge_evidence(vec![batch.clone()]).is_ok()
}

struct ArtifactFile {
    path: PathBuf,
    digest: String,
    byte_len: u64,
}

fn discover_artifacts(root: &Path, options: &BuildOptions) -> Result<Vec<ArtifactFile>, CoreError> {
    let conventional = root.join("index.scip");
    let mut candidates = Vec::new();
    if conventional.is_file() {
        candidates.push(conventional);
    }
    for path in &options.program_artifacts {
        let path = if path.is_absolute() {
            path.clone()
        } else {
            root.join(path)
        };
        if !path.is_file() {
            return Err(CoreError::InvalidProgramInput(format!(
                "program artifact is not a regular file: {}",
                path.display()
            )));
        }
        candidates.push(path);
    }
    let mut by_digest = BTreeMap::new();
    for path in candidates {
        let canonical = fs::canonicalize(&path).map_err(|source| FileError::Io {
            path: path.clone(),
            source,
        })?;
        if canonical.starts_with(root.join("compass-out"))
            || canonical.starts_with(root.join("graphify-out"))
        {
            return Err(CoreError::InvalidProgramInput(
                "program artifacts cannot come from output directories".to_owned(),
            ));
        }
        let (digest, byte_len) = hash_file(&canonical)?;
        by_digest.entry(digest.clone()).or_insert(ArtifactFile {
            path: canonical,
            digest,
            byte_len,
        });
    }
    Ok(by_digest.into_values().collect())
}

fn hash_file(path: &Path) -> Result<(String, u64), CoreError> {
    let mut file = File::open(path).map_err(|source| FileError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let byte_len = file
        .metadata()
        .map_err(|source| FileError::Io {
            path: path.to_path_buf(),
            source,
        })?
        .len();
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(|source| FileError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok((hex_sha256_bytes(digest.finalize().as_slice()), byte_len))
}

fn hex_sha256_bytes(bytes: &[u8]) -> String {
    use std::fmt::Write;

    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(output, "{byte:02x}");
    }
    output
}

fn companion_path(artifact: &Path) -> PathBuf {
    let mut name = artifact
        .file_name()
        .map_or_else(OsString::new, OsString::from);
    name.push(".compass-manifest.json");
    artifact.with_file_name(name)
}

fn load_manifest(
    artifact: &Path,
    digest: &str,
) -> Result<Option<compass_program::ArtifactManifest>, CoreError> {
    let path = companion_path(artifact);
    if !path.exists() {
        return Ok(None);
    }
    let metadata = fs::metadata(&path).map_err(|source| FileError::Io {
        path: path.clone(),
        source,
    })?;
    if !metadata.is_file() || metadata.len() > 8 * 1024 * 1024 {
        return Err(CoreError::InvalidProgramInput(format!(
            "invalid SCIP companion manifest {}",
            path.display()
        )));
    }
    let bytes = fs::read(&path).map_err(|source| FileError::Io { path, source })?;
    Ok(Some(parse_artifact_manifest(&bytes, digest)?))
}

fn decode_artifact_file(
    artifact: &ArtifactFile,
    input: ArtifactInput<'_>,
) -> Result<DecodedScipArtifact, CoreError> {
    let mut reader = File::open(&artifact.path).map_err(|source| FileError::Io {
        path: artifact.path.clone(),
        source,
    })?;
    Ok(OfficialScipProvider.decode_artifact(input, &mut reader)?)
}

fn save_decoded_artifact(
    cache: &Cache,
    kind: &CacheKind,
    artifact_digest: &str,
    header_key: &str,
    decoded: &DecodedScipArtifact,
    live: &mut BTreeSet<String>,
) -> Result<(), CoreError> {
    let header = ArtifactCacheHeader {
        metadata_protobuf_base64: decoded.metadata_protobuf_base64.clone(),
        documents: decoded
            .documents
            .iter()
            .map(|document| document.path.clone())
            .collect(),
    };
    cache.save_program(kind, header_key, &header)?;
    live.insert(header_key.to_owned());
    for document in &decoded.documents {
        let key = raw_document_key(artifact_digest, &document.path);
        cache.save_program(kind, &key, document)?;
        live.insert(key);
    }
    Ok(())
}

fn assemble_decoded_artifact(
    context: &mut ArtifactCacheContext<'_>,
    input: ArtifactInput<'_>,
    header: &ArtifactCacheHeader,
    decoded: Option<&DecodedScipArtifact>,
    stats: &mut DocumentCacheStats,
) -> Result<Option<EvidenceBatch>, CoreError> {
    let descriptor = OfficialScipProvider.descriptor(&input);
    let mut batch = OfficialScipProvider.normalize_decoded(
        input,
        &DecodedScipArtifact {
            metadata_protobuf_base64: header.metadata_protobuf_base64.clone(),
            documents: Vec::new(),
        },
    )?;
    let header_key = format!("{}:decoded:header", context.artifact_digest);
    context.live.insert(header_key);
    let decoded_documents = decoded
        .map(|artifact| {
            artifact
                .documents
                .iter()
                .map(|document| (document.path.as_str(), document))
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    for document in &header.documents {
        let raw_key = raw_document_key(context.artifact_digest, document);
        context.live.insert(raw_key.clone());
        let normalized_key = normalized_document_key(context.artifact_digest, document, input)?;
        let mut shard = context
            .cache
            .load_program::<EvidenceBatch>(context.kind, &normalized_key)?;
        if shard
            .as_ref()
            .is_some_and(|cached| cached.descriptor.id != descriptor.id)
        {
            shard = None;
        }
        let mut shard = if let Some(shard) = shard {
            stats.reused += 1;
            shard
        } else {
            let raw = if let Some(document) = decoded_documents.get(document.as_str()) {
                (*document).clone()
            } else {
                let Some(document) = context
                    .cache
                    .load_program::<DecodedScipDocument>(context.kind, &raw_key)?
                else {
                    return Ok(None);
                };
                document
            };
            let decoded = DecodedScipArtifact {
                metadata_protobuf_base64: header.metadata_protobuf_base64.clone(),
                documents: vec![raw],
            };
            let normalized = OfficialScipProvider.normalize_decoded(input, &decoded)?;
            context
                .cache
                .save_program(context.kind, &normalized_key, &normalized)?;
            stats.analyzed += 1;
            normalized
        };
        shard.descriptor = descriptor.clone();
        batch.evidence.extend(shard.evidence);
        batch.facts.extend(shard.facts);
        batch.coverage.extend(shard.coverage);
        context.live.insert(normalized_key);
    }
    batch.descriptor = descriptor;
    Ok(Some(batch.canonicalized()))
}

fn raw_document_key(artifact_digest: &str, document: &str) -> String {
    format!("{artifact_digest}:decoded:document:{document}")
}

fn normalized_document_key(
    artifact_digest: &str,
    document: &str,
    input: ArtifactInput<'_>,
) -> Result<String, CoreError> {
    let expected = input.manifest.and_then(|manifest| {
        manifest.documents.iter().find_map(|(path, digest)| {
            compass_program::normalize_source_path(path)
                .ok()
                .filter(|path| path == document)
                .map(|_| digest.as_str())
        })
    });
    let actual = expected.and_then(|_| input.source_digests.get(document).map(String::as_str));
    let digest = hex_sha256(&canonical_json_bytes(&(
        artifact_digest,
        document,
        expected,
        actual,
    ))?);
    Ok(format!("{artifact_digest}:normalized:{digest}"))
}

fn provider_manifest_digest(batches: &[EvidenceBatch]) -> Result<String, CoreError> {
    let mut batch_digests = batches
        .iter()
        .map(EvidenceBatch::digest)
        .collect::<Result<Vec<_>, _>>()?;
    batch_digests.sort();
    Ok(hex_sha256(&canonical_json_bytes(&batch_digests)?))
}

fn count_conflicts(analysis: &compass_analysis::AnalysisBundle) -> usize {
    analysis
        .program
        .modules
        .iter()
        .flat_map(|module| &module.functions)
        .flat_map(|function| &function.blocks)
        .flat_map(|block| &block.operations)
        .filter(|operation| {
            matches!(
                &operation.kind,
                compass_ir::OperationKind::Call {
                    resolved_symbols,
                    ..
                } if resolved_symbols.len() > 1
            )
        })
        .count()
}
