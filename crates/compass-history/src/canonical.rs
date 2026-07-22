use serde_json::Value;

use crate::HistoryError;

/// Version of the byte-stable canonical JSON encoding.
pub const CANONICAL_ENCODING_VERSION: u32 = 1;

/// Encode JSON into deterministic, whitespace-free UTF-8 bytes.
pub fn canonical_json_bytes(value: &Value) -> Result<Vec<u8>, HistoryError> {
    let mut output = Vec::new();
    write_canonical_value(value, &mut output)?;
    Ok(output)
}

fn write_canonical_value(value: &Value, output: &mut Vec<u8>) -> Result<(), HistoryError> {
    match value {
        Value::Null => output.extend_from_slice(b"null"),
        Value::Bool(true) => output.extend_from_slice(b"true"),
        Value::Bool(false) => output.extend_from_slice(b"false"),
        Value::Number(number) => output.extend_from_slice(number.to_string().as_bytes()),
        Value::String(text) => {
            serde_json::to_writer(output, text)
                .map_err(|error| HistoryError::Canonical(error.to_string()))?;
        }
        Value::Array(values) => {
            output.push(b'[');
            for (index, item) in values.iter().enumerate() {
                if index != 0 {
                    output.push(b',');
                }
                write_canonical_value(item, output)?;
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
                serde_json::to_writer(&mut *output, key)
                    .map_err(|error| HistoryError::Canonical(error.to_string()))?;
                output.push(b':');
                let item = values.get(key).ok_or_else(|| {
                    HistoryError::Canonical("object key disappeared during encoding".to_owned())
                })?;
                write_canonical_value(item, output)?;
            }
            output.push(b'}');
        }
    }
    Ok(())
}
