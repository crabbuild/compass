#![no_main]

use libfuzzer_sys::fuzz_target;
use trail_semantic::{sanitize_semantic_fragment, validate_semantic_fragment};

fuzz_target!(|data: &[u8]| {
    if data.len() > 1_048_576 {
        return;
    }
    let Ok(mut fragment) = serde_json::from_slice(data) else {
        return;
    };
    if validate_semantic_fragment(&mut fragment).is_empty() {
        sanitize_semantic_fragment(&mut fragment);
    }
});
