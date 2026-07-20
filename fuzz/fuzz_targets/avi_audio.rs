#![no_main]

use std::io::Write;

use libfuzzer_sys::fuzz_target;
use trail_transcribe::audio::{AudioLimits, decode_audio};

fuzz_target!(|data: &[u8]| {
    if data.len() > 1_000_000 {
        return;
    }
    let generated;
    let input = if data.first().is_some_and(|mode| mode & 1 == 1) {
        let Some(avi) = pcm_avi(&data[1..]) else {
            return;
        };
        generated = avi;
        generated.as_slice()
    } else {
        data
    };
    let Ok(mut source) = tempfile::NamedTempFile::new() else {
        return;
    };
    if source.write_all(input).is_err() {
        return;
    }
    let _ = decode_audio(
        source.path(),
        AudioLimits {
            max_source_bytes: 1_048_576,
            max_duration_seconds: 1,
        },
    );
});

fn pcm_avi(payload: &[u8]) -> Option<Vec<u8>> {
    let mut stream_header = vec![0_u8; 56];
    stream_header[..4].copy_from_slice(b"auds");
    let mut wave_format = Vec::with_capacity(16);
    wave_format.extend_from_slice(&1_u16.to_le_bytes());
    wave_format.extend_from_slice(&1_u16.to_le_bytes());
    wave_format.extend_from_slice(&16_000_u32.to_le_bytes());
    wave_format.extend_from_slice(&16_000_u32.to_le_bytes());
    wave_format.extend_from_slice(&1_u16.to_le_bytes());
    wave_format.extend_from_slice(&8_u16.to_le_bytes());
    let stream_list = riff_list(
        *b"strl",
        &[
            riff_chunk(*b"strh", &stream_header)?,
            riff_chunk(*b"strf", &wave_format)?,
        ]
        .concat(),
    )?;
    let header_list = riff_list(*b"hdrl", &stream_list)?;
    let media_list = riff_list(*b"movi", &riff_chunk(*b"00wb", payload)?)?;
    riff_form(*b"AVI ", &[header_list, media_list].concat())
}

fn riff_form(form: [u8; 4], payload: &[u8]) -> Option<Vec<u8>> {
    let size = u32::try_from(payload.len().checked_add(4)?).ok()?;
    let mut bytes = Vec::with_capacity(payload.len().checked_add(12)?);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&size.to_le_bytes());
    bytes.extend_from_slice(&form);
    bytes.extend_from_slice(payload);
    Some(bytes)
}

fn riff_list(form: [u8; 4], payload: &[u8]) -> Option<Vec<u8>> {
    let mut content = Vec::with_capacity(payload.len().checked_add(4)?);
    content.extend_from_slice(&form);
    content.extend_from_slice(payload);
    riff_chunk(*b"LIST", &content)
}

fn riff_chunk(id: [u8; 4], payload: &[u8]) -> Option<Vec<u8>> {
    let size = u32::try_from(payload.len()).ok()?;
    let mut bytes = Vec::with_capacity(payload.len().checked_add(9)?);
    bytes.extend_from_slice(&id);
    bytes.extend_from_slice(&size.to_le_bytes());
    bytes.extend_from_slice(payload);
    if payload.len() % 2 == 1 {
        bytes.push(0);
    }
    Some(bytes)
}
