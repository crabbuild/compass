use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use compass_files::{Cache, CacheKind, FileError, write_bytes_atomic};
use compass_ir::{EvidenceRecord, ProviderDescriptor, canonical_json_bytes, hex_sha256};
use compass_languages::{Registry, TREE_SITTER_PROGRAM_PROVIDER_VERSION, TreeSitterSyntaxProvider};
use compass_program::{
    ArtifactInput, ArtifactProvider, EvidenceBatch, OfficialScipProvider, SyntaxProvider,
    merge_evidence, parse_artifact_manifest,
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
    descriptor: ProviderDescriptor,
    global_evidence: Vec<EvidenceRecord>,
    documents: Vec<String>,
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
    for artifact in artifacts {
        let manifest = load_manifest(&artifact.path, &artifact.digest)?;
        let cache_digest =
            artifact_cache_digest(&artifact.digest, manifest.as_ref(), &source_digests)?;
        let header_key = format!("{cache_digest}:header");
        let cached = if !options.force {
            load_artifact_cache(
                cache,
                &artifact_kind,
                &cache_digest,
                &header_key,
                &mut live_artifact_keys,
            )?
        } else {
            None
        };
        let batch = if let Some(batch) = cached {
            artifacts_reused += 1;
            batch
        } else {
            let mut reader = File::open(&artifact.path).map_err(|source| FileError::Io {
                path: artifact.path.clone(),
                source,
            })?;
            let logical_name = artifact
                .path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("index.scip");
            let batch = OfficialScipProvider.analyze_artifact(
                ArtifactInput {
                    logical_name,
                    input_digest: &artifact.digest,
                    byte_len: artifact.byte_len,
                    manifest: manifest.as_ref(),
                    source_digests: &source_digests,
                    source_texts: &source_texts,
                    limits: options.program_artifact_limits,
                },
                &mut reader,
            )?;
            save_artifact_cache(
                cache,
                &artifact_kind,
                &cache_digest,
                &header_key,
                &batch,
                &mut live_artifact_keys,
            )?;
            artifacts_loaded += 1;
            batch
        };
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
        conflicts,
    }))
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

fn load_artifact_cache(
    cache: &Cache,
    kind: &CacheKind,
    digest: &str,
    header_key: &str,
    live: &mut BTreeSet<String>,
) -> Result<Option<EvidenceBatch>, CoreError> {
    let Some(header) = cache.load_program::<ArtifactCacheHeader>(kind, header_key)? else {
        return Ok(None);
    };
    let mut batch = EvidenceBatch {
        descriptor: header.descriptor.clone(),
        evidence: header.global_evidence,
        modules: Vec::new(),
        facts: Vec::new(),
        coverage: BTreeMap::new(),
    };
    let mut keys = vec![header_key.to_owned()];
    for document in header.documents {
        let key = format!("{digest}:document:{document}");
        let Some(shard) = cache.load_program::<EvidenceBatch>(kind, &key)? else {
            return Ok(None);
        };
        if shard.descriptor != header.descriptor {
            return Ok(None);
        }
        keys.push(key);
        batch.evidence.extend(shard.evidence);
        batch.facts.extend(shard.facts);
        batch.coverage.extend(shard.coverage);
    }
    if merge_evidence(vec![batch.clone()]).is_err() {
        return Ok(None);
    }
    live.extend(keys);
    Ok(Some(batch.canonicalized()))
}

fn save_artifact_cache(
    cache: &Cache,
    kind: &CacheKind,
    digest: &str,
    header_key: &str,
    batch: &EvidenceBatch,
    live: &mut BTreeSet<String>,
) -> Result<(), CoreError> {
    let mut documents = batch.coverage.keys().cloned().collect::<BTreeSet<_>>();
    documents.extend(
        batch
            .evidence
            .iter()
            .filter_map(|record| record.source_file.clone()),
    );
    documents.extend(
        batch
            .facts
            .iter()
            .map(|fact| fact.anchor.source_file.clone()),
    );
    let documents = documents.into_iter().collect::<Vec<_>>();
    let header = ArtifactCacheHeader {
        descriptor: batch.descriptor.clone(),
        global_evidence: batch
            .evidence
            .iter()
            .filter(|record| record.source_file.is_none())
            .cloned()
            .collect(),
        documents: documents.clone(),
    };
    cache.save_program(kind, header_key, &header)?;
    live.insert(header_key.to_owned());
    for document in documents {
        let key = format!("{digest}:document:{document}");
        let shard = EvidenceBatch {
            descriptor: batch.descriptor.clone(),
            evidence: batch
                .evidence
                .iter()
                .filter(|record| record.source_file.as_deref() == Some(document.as_str()))
                .cloned()
                .collect(),
            modules: Vec::new(),
            facts: batch
                .facts
                .iter()
                .filter(|fact| fact.anchor.source_file == document)
                .cloned()
                .collect(),
            coverage: batch
                .coverage
                .get(&document)
                .cloned()
                .map(|coverage| BTreeMap::from([(document.clone(), coverage)]))
                .unwrap_or_default(),
        };
        cache.save_program(kind, &key, &shard)?;
        live.insert(key);
    }
    Ok(())
}

fn provider_manifest_digest(batches: &[EvidenceBatch]) -> Result<String, CoreError> {
    let mut batch_digests = batches
        .iter()
        .map(EvidenceBatch::digest)
        .collect::<Result<Vec<_>, _>>()?;
    batch_digests.sort();
    Ok(hex_sha256(&canonical_json_bytes(&batch_digests)?))
}

fn artifact_cache_digest(
    artifact_digest: &str,
    manifest: Option<&compass_program::ArtifactManifest>,
    source_digests: &BTreeMap<String, String>,
) -> Result<String, CoreError> {
    Ok(hex_sha256(&canonical_json_bytes(&(
        artifact_digest,
        manifest,
        source_digests,
    ))?))
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
