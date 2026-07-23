mod rust;
mod typescript;

use std::path::Path;

use compass_ir::{ProviderDescriptor, ProviderKind, hex_sha256};
use compass_program::{
    EvidenceBatch, FileInput, ProviderError, SyntaxProvider, merge_evidence, normalize_source_path,
};

use crate::{Engine, ExtractorKind, Registry};

pub const TREE_SITTER_PROGRAM_PROVIDER_VERSION: u32 = 1;

#[derive(Default)]
pub struct TreeSitterSyntaxProvider {
    engine: Engine,
}

impl SyntaxProvider for TreeSitterSyntaxProvider {
    fn descriptor(&self, input: &FileInput<'_>) -> ProviderDescriptor {
        let source_digest = hex_sha256(input.source);
        let provider_key = format!("{}:{}:{source_digest}", input.language, input.source_file);
        ProviderDescriptor {
            id: format!("tree-sitter:{}", hex_sha256(provider_key.as_bytes())),
            kind: ProviderKind::Syntax,
            version: format!("tree-sitter/{TREE_SITTER_PROGRAM_PROVIDER_VERSION}"),
            scope: input.source_file.to_owned(),
            input_digest: source_digest,
            configuration_digest: hex_sha256(
                format!("{}:{TREE_SITTER_PROGRAM_PROVIDER_VERSION}", input.language).as_bytes(),
            ),
        }
    }

    fn analyze_file(
        &mut self,
        input: FileInput<'_>,
    ) -> Result<Option<EvidenceBatch>, ProviderError> {
        let source_file = normalize_source_path(input.source_file)?;
        let path = Path::new(&source_file);
        let Some(spec) = Registry::resolve(path) else {
            return Ok(None);
        };
        if spec.kind != ExtractorKind::Generic
            || !matches!(spec.name, "rust" | "typescript" | "tsx" | "javascript")
        {
            return Ok(None);
        }
        let normalized = FileInput {
            source_file: &source_file,
            language: spec.name,
            source: input.source,
        };
        let descriptor = self.descriptor(&normalized);
        let tree = self
            .engine
            .parse(path, spec, input.source)
            .map_err(|error| ProviderError::InvalidInput(error.to_string()))?;
        let batch = match spec.name {
            "rust" => rust::extract(descriptor, &normalized, tree.root_node()),
            "typescript" | "tsx" | "javascript" => {
                typescript::extract(descriptor, &normalized, tree.root_node())
            }
            _ => return Ok(None),
        };
        merge_evidence(vec![batch.clone()])
            .map_err(|error| ProviderError::InvalidInput(error.to_string()))?;
        Ok(Some(batch))
    }
}
