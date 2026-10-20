//! Shared pre-retention accounting for the versioned TypeScript proof tree.

use super::model::{
    MAX_BATCH_TYPESCRIPT_TYPE_ARGUMENT_BYTES, MAX_BATCH_TYPESCRIPT_TYPE_ARGUMENT_NODES,
    MAX_TYPESCRIPT_TYPE_ARGUMENT_ARITY, MAX_TYPESCRIPT_TYPE_ARGUMENT_BYTES,
    MAX_TYPESCRIPT_TYPE_ARGUMENT_DEPTH, MAX_TYPESCRIPT_TYPE_ARGUMENT_NODES,
    MAX_TYPESCRIPT_TYPE_ARGUMENT_REFERENCE_BYTES, TypeScriptTypeArgument, TypeScriptTypeArguments,
};
use super::validate::{EvidenceError, EvidenceErrorCode};

#[derive(Clone, Copy, Default, Debug, Eq, PartialEq)]
#[doc(hidden)]
pub struct TypeScriptArgumentUsage {
    pub nodes: usize,
    /// UTF-8 bytes in owner/reference IDs and path segments, counting copies.
    pub bytes: usize,
}

fn limit(message: &'static str) -> EvidenceError {
    EvidenceError::new(EvidenceErrorCode::ResourceLimit, message)
}

impl TypeScriptArgumentUsage {
    /// Reuse the wire proof's accounting, including repeated copies and owner
    /// text for a present empty proof. No proof payload is cloned.
    pub fn for_batch(batch: &super::SemanticEvidenceBatch) -> Result<Self, EvidenceError> {
        batch
            .candidates
            .iter()
            .filter_map(|candidate| candidate.constraints.typescript_type_arguments.as_ref())
            .try_fold(Self::default(), |usage, proof| {
                usage.retaining(Self::for_proof(proof)?)
            })
    }
    pub(super) fn for_proof(proof: &TypeScriptTypeArguments) -> Result<Self, EvidenceError> {
        let mut usage = Self::default();
        usage.add_text(&proof.owner_binding_id)?;
        let mut pending = vec![(proof.arguments.as_slice(), 0_usize)];
        while let Some((arguments, depth)) = pending.pop() {
            if arguments.len() > MAX_TYPESCRIPT_TYPE_ARGUMENT_ARITY
                || depth > MAX_TYPESCRIPT_TYPE_ARGUMENT_DEPTH
            {
                return Err(limit(
                    "TypeScript type arguments exceed arity or depth limit",
                ));
            }
            usage.nodes = usage.nodes.saturating_add(arguments.len());
            if usage.nodes > MAX_TYPESCRIPT_TYPE_ARGUMENT_NODES {
                return Err(limit(
                    "TypeScript type arguments exceed candidate node limit",
                ));
            }
            for argument in arguments {
                let nested = match argument {
                    TypeScriptTypeArgument::Source {
                        declaration_id,
                        arguments,
                    } => {
                        usage.add_text(declaration_id)?;
                        arguments
                    }
                    TypeScriptTypeArgument::Import {
                        binding_id,
                        member_path,
                        arguments,
                    } => {
                        usage.add_text(binding_id)?;
                        if member_path.len() > MAX_TYPESCRIPT_TYPE_ARGUMENT_DEPTH {
                            return Err(limit("TypeScript argument path exceeds depth limit"));
                        }
                        for segment in member_path {
                            usage.add_text(segment)?;
                        }
                        arguments
                    }
                    TypeScriptTypeArgument::Unknown => continue,
                };
                // Empty children are leaves, not a further level of nesting.
                if !nested.is_empty() {
                    pending.push((nested, depth + 1));
                }
            }
        }
        Ok(usage)
    }

    fn add_text(&mut self, text: &str) -> Result<(), EvidenceError> {
        if text.len() > MAX_TYPESCRIPT_TYPE_ARGUMENT_REFERENCE_BYTES {
            return Err(limit(
                "TypeScript argument reference or segment exceeds byte limit",
            ));
        }
        self.bytes = self.bytes.saturating_add(text.len());
        if self.bytes > MAX_TYPESCRIPT_TYPE_ARGUMENT_BYTES {
            return Err(limit(
                "TypeScript type arguments exceed candidate text byte limit",
            ));
        }
        Ok(())
    }

    pub(super) fn retaining(self, proof: Self) -> Result<Self, EvidenceError> {
        let next = Self {
            nodes: self.nodes.saturating_add(proof.nodes),
            bytes: self.bytes.saturating_add(proof.bytes),
        };
        if next.nodes > MAX_BATCH_TYPESCRIPT_TYPE_ARGUMENT_NODES
            || next.bytes > MAX_BATCH_TYPESCRIPT_TYPE_ARGUMENT_BYTES
        {
            return Err(limit(
                "TypeScript type arguments exceed retained batch node or byte limit",
            ));
        }
        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_accounting_counts_nested_copies_unknown_and_empty_owner()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut batch = crate::Engine::default().extract_source_universal_evidence(
            std::path::Path::new("main.ts"),
            "main.ts",
            b"export function run() {}",
        )?;
        let mut candidate = batch
            .candidates
            .first()
            .ok_or("missing synthetic candidate")?
            .clone();
        candidate.constraints.typescript_type_arguments = Some(TypeScriptTypeArguments {
            version: 1,
            owner_binding_id: "é".to_owned(),
            arguments: vec![TypeScriptTypeArgument::Source {
                declaration_id: "s".to_owned(),
                arguments: vec![TypeScriptTypeArgument::Import {
                    binding_id: "i".to_owned(),
                    member_path: vec!["界".to_owned()],
                    arguments: vec![TypeScriptTypeArgument::Unknown],
                }],
            }],
        });
        batch.candidates = vec![candidate.clone(), candidate.clone()];
        assert_eq!(
            TypeScriptArgumentUsage::for_batch(&batch)?,
            TypeScriptArgumentUsage {
                nodes: 6,
                bytes: 14
            }
        );
        candidate.constraints.typescript_type_arguments = Some(TypeScriptTypeArguments {
            version: 1,
            owner_binding_id: "é".to_owned(),
            arguments: Vec::new(),
        });
        batch.candidates.push(candidate.clone());
        assert_eq!(
            TypeScriptArgumentUsage::for_batch(&batch)?,
            TypeScriptArgumentUsage {
                nodes: 6,
                bytes: 16
            }
        );
        candidate.constraints.typescript_type_arguments = None;
        batch.candidates.push(candidate);
        assert_eq!(
            TypeScriptArgumentUsage::for_batch(&batch)?,
            TypeScriptArgumentUsage {
                nodes: 6,
                bytes: 16
            }
        );
        Ok(())
    }

    #[test]
    fn reference_segment_and_cumulative_text_limits_are_inclusive() -> Result<(), EvidenceError> {
        let mut proof = TypeScriptTypeArguments {
            version: 1,
            owner_binding_id: "b".repeat(1024),
            arguments: Vec::new(),
        };
        assert_eq!(TypeScriptArgumentUsage::for_proof(&proof)?.bytes, 1024);
        proof.owner_binding_id.push('b');
        assert_eq!(
            TypeScriptArgumentUsage::for_proof(&proof)
                .err()
                .map(|error| error.code),
            Some(EvidenceErrorCode::ResourceLimit)
        );
        proof.owner_binding_id = "b".to_owned();
        proof.arguments = vec![TypeScriptTypeArgument::Import {
            binding_id: "b".to_owned(),
            member_path: vec!["x".repeat(1024)],
            arguments: Vec::new(),
        }];
        assert_eq!(TypeScriptArgumentUsage::for_proof(&proof)?.bytes, 1026);
        if let TypeScriptTypeArgument::Import { member_path, .. } = &mut proof.arguments[0] {
            member_path[0].push('x');
        }
        assert_eq!(
            TypeScriptArgumentUsage::for_proof(&proof)
                .err()
                .map(|error| error.code),
            Some(EvidenceErrorCode::ResourceLimit)
        );
        proof.arguments = (0..4)
            .map(|_| TypeScriptTypeArgument::Import {
                binding_id: "b".to_owned(),
                member_path: vec!["x".repeat(1024); 16],
                arguments: Vec::new(),
            })
            .collect();
        if let TypeScriptTypeArgument::Import { member_path, .. } = &mut proof.arguments[3] {
            member_path[15].truncate(1019);
        }
        assert_eq!(
            TypeScriptArgumentUsage::for_proof(&proof)?.bytes,
            MAX_TYPESCRIPT_TYPE_ARGUMENT_BYTES
        );
        if let TypeScriptTypeArgument::Import { member_path, .. } = &mut proof.arguments[3] {
            member_path[15].push('x');
        }
        assert_eq!(
            TypeScriptArgumentUsage::for_proof(&proof)
                .err()
                .map(|error| error.code),
            Some(EvidenceErrorCode::ResourceLimit)
        );
        Ok(())
    }
}
