//! Small helpers ported from `whisper/utils.py`.

use flate2::Compression;
use flate2::write::ZlibEncoder;
use std::io::Write;

/// gzip-style repetitiveness measure: len(text) / len(zlib(text)).
/// Matches Python's `zlib.compress(text.encode("utf-8"))` at default level 6.
pub fn compression_ratio(text: &str) -> f64 {
    let bytes = text.as_bytes();
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    let compressed = match encoder.write_all(bytes).and_then(|()| encoder.finish()) {
        Ok(compressed) if !compressed.is_empty() => compressed,
        Ok(_) | Err(_) => return f64::INFINITY,
    };
    bytes.len() as f64 / compressed.len() as f64
}

/// `utils.format_timestamp`: "MM:SS.mmm" (or with hours), used by writers.
pub fn format_timestamp(seconds: f64, always_include_hours: bool, decimal_marker: &str) -> String {
    assert!(seconds >= 0.0, "non-negative timestamp expected");
    let milliseconds = (seconds * 1000.0).round() as u64;
    let hours = milliseconds / 3_600_000;
    let minutes = (milliseconds % 3_600_000) / 60_000;
    let secs = (milliseconds % 60_000) / 1000;
    let millis = milliseconds % 1000;
    let hours_marker = if always_include_hours || hours > 0 {
        format!("{hours:02}:")
    } else {
        String::new()
    };
    format!("{hours_marker}{minutes:02}:{secs:02}{decimal_marker}{millis:03}")
}
