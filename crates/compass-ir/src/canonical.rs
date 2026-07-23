use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{CoverageState, FunctionIr, IrError, ModuleIr, OperationKind, ProgramBundle, TypeRef};

impl ProgramBundle {
    pub fn canonicalized(&self) -> Self {
        let mut bundle = self.clone();
        bundle.providers.sort();
        bundle.evidence.sort();
        for record in &mut bundle.evidence {
            if let Some(path) = &mut record.source_file {
                *path = path.replace('\\', "/");
            }
        }
        for module in &mut bundle.modules {
            canonicalize_module(module);
        }
        bundle.modules.sort_by(|left, right| {
            left.source_file
                .as_bytes()
                .cmp(right.source_file.as_bytes())
                .then_with(|| left.language.as_bytes().cmp(right.language.as_bytes()))
        });
        bundle
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, IrError> {
        let canonical = self.canonicalized();
        canonical.validate()?;
        let value = serde_json::to_value(canonical)?;
        let mut output = Vec::new();
        write_value(&value, &mut output)?;
        output.push(b'\n');
        Ok(output)
    }

    pub fn digest(&self) -> Result<String, IrError> {
        Ok(hex_sha256(&self.canonical_bytes()?))
    }
}

pub fn canonical_json_bytes<T: serde::Serialize>(value: &T) -> Result<Vec<u8>, IrError> {
    let value = serde_json::to_value(value)?;
    let mut output = Vec::new();
    write_value(&value, &mut output)?;
    output.push(b'\n');
    Ok(output)
}

pub fn hex_sha256(bytes: &[u8]) -> String {
    use std::fmt::Write;

    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(64);
    for byte in digest {
        let _ = write!(output, "{byte:02x}");
    }
    output
}

fn canonicalize_module(module: &mut ModuleIr) {
    module.source_file = module.source_file.replace('\\', "/");
    sort_dedup(&mut module.evidence);
    canonicalize_coverage(&mut module.coverage);
    for function in &mut module.functions {
        canonicalize_function(function);
    }
    module.functions.sort_by(|left, right| {
        left.symbol_id
            .as_bytes()
            .cmp(right.symbol_id.as_bytes())
            .then_with(|| left.anchor.start_byte.cmp(&right.anchor.start_byte))
    });
}

fn canonicalize_function(function: &mut FunctionIr) {
    function.anchor.source_file = function.anchor.source_file.replace('\\', "/");
    sort_dedup(&mut function.evidence);
    canonicalize_coverage(&mut function.coverage);
    for parameter in &mut function.parameters {
        parameter.anchor.source_file = parameter.anchor.source_file.replace('\\', "/");
        sort_dedup(&mut parameter.evidence);
        if let Some(type_ref) = &mut parameter.type_ref {
            canonicalize_type_ref(type_ref);
        }
    }
    if let Some(type_ref) = &mut function.return_type {
        canonicalize_type_ref(type_ref);
    }
    for block in &mut function.blocks {
        sort_dedup(&mut block.evidence);
        for operation in &mut block.operations {
            operation.anchor.source_file = operation.anchor.source_file.replace('\\', "/");
            sort_dedup(&mut operation.evidence);
            if let OperationKind::Call {
                callee_anchor,
                resolved_symbols,
                receiver_type,
                ..
            } = &mut operation.kind
            {
                callee_anchor.source_file = callee_anchor.source_file.replace('\\', "/");
                sort_dedup(resolved_symbols);
                if let Some(type_ref) = receiver_type {
                    canonicalize_type_ref(type_ref);
                }
            }
        }
        block.operations.sort_by(|left, right| {
            left.ordinal
                .cmp(&right.ordinal)
                .then_with(|| left.anchor.start_byte.cmp(&right.anchor.start_byte))
                .then_with(|| left.anchor.end_byte.cmp(&right.anchor.end_byte))
        });
    }
    function.blocks.sort_by_key(|block| block.id);
}

fn canonicalize_type_ref(type_ref: &mut TypeRef) {
    sort_dedup(&mut type_ref.evidence);
}

fn canonicalize_coverage(coverage: &mut crate::Coverage) {
    for state in coverage.values_mut() {
        match state {
            CoverageState::Complete => {}
            CoverageState::Partial { reasons }
            | CoverageState::Indeterminate { reasons }
            | CoverageState::Failed { reasons }
            | CoverageState::Unavailable { reasons } => {
                sort_dedup(reasons);
            }
        }
    }
}

fn sort_dedup<T: Ord>(items: &mut Vec<T>) {
    items.sort();
    items.dedup();
}

fn write_value(value: &Value, output: &mut Vec<u8>) -> Result<(), IrError> {
    match value {
        Value::Null => output.extend_from_slice(b"null"),
        Value::Bool(true) => output.extend_from_slice(b"true"),
        Value::Bool(false) => output.extend_from_slice(b"false"),
        Value::Number(number) => output.extend_from_slice(number.to_string().as_bytes()),
        Value::String(text) => serde_json::to_writer(output, text)?,
        Value::Array(values) => {
            output.push(b'[');
            for (index, item) in values.iter().enumerate() {
                if index != 0 {
                    output.push(b',');
                }
                write_value(item, output)?;
            }
            output.push(b']');
        }
        Value::Object(values) => {
            output.push(b'{');
            let mut keys = values.keys().collect::<Vec<_>>();
            keys.sort_unstable_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
            for (index, key) in keys.into_iter().enumerate() {
                if index != 0 {
                    output.push(b',');
                }
                serde_json::to_writer(&mut *output, key)?;
                output.push(b':');
                let item = values
                    .get(key)
                    .ok_or_else(|| IrError::Canonical("object key disappeared".to_owned()))?;
                write_value(item, output)?;
            }
            output.push(b'}');
        }
    }
    Ok(())
}
