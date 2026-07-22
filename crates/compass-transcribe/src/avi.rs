//! Bounded RIFF/AVI audio demuxing without a system FFmpeg dependency.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use crate::audio::{AudioError, AudioLimits, DecodedAudio, decode_audio_file, finalize_pcm};

const RIFF_HEADER_LEN: u64 = 12;
const CHUNK_HEADER_LEN: u64 = 8;
const MAX_LIST_DEPTH: usize = 16;
const WAVE_FORMAT_PCM: u16 = 0x0001;
const WAVE_FORMAT_IEEE_FLOAT: u16 = 0x0003;
const WAVE_FORMAT_MP3: u16 = 0x0055;
const WAVE_FORMAT_RAW_AAC: u16 = 0x00ff;
const WAVE_FORMAT_EXTENSIBLE: u16 = 0xfffe;

#[derive(Debug, Clone, Copy)]
struct WaveFormat {
    tag: u16,
    channels: usize,
    sample_rate: usize,
    block_align: usize,
    bits_per_sample: usize,
    aac_config: Option<AacConfig>,
}

#[derive(Debug, Clone, Copy)]
struct AacConfig {
    object_type: u8,
    frequency_index: u8,
    channel_config: u8,
}

#[derive(Debug, Clone, Copy)]
struct AudioStream {
    index: u8,
    format: WaveFormat,
}

#[derive(Debug, Clone, Copy)]
struct Chunk {
    id: [u8; 4],
    data_start: u64,
    data_end: u64,
    next: u64,
}

enum PayloadSink {
    Pcm(Vec<f32>),
    Compressed(File),
}

/// Detect AVI by its RIFF form type, independent of the file extension.
pub(crate) fn is_avi(path: &Path) -> Result<bool, AudioError> {
    let mut file = File::open(path).map_err(|source| AudioError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut header = [0_u8; RIFF_HEADER_LEN as usize];
    match file.read_exact(&mut header) {
        Ok(()) => Ok(&header[..4] == b"RIFF" && &header[8..] == b"AVI "),
        Err(source) if source.kind() == std::io::ErrorKind::UnexpectedEof => Ok(false),
        Err(source) => Err(AudioError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// Decode the first AVI audio stream to 16 kHz mono PCM.
pub(crate) fn decode_avi_audio(
    path: &Path,
    limits: AudioLimits,
) -> Result<DecodedAudio, AudioError> {
    let mut file = File::open(path).map_err(|source| AudioError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let file_len = file
        .metadata()
        .map_err(|source| AudioError::Io {
            path: path.to_path_buf(),
            source,
        })?
        .len();
    let stream = find_audio_stream(&mut file, file_len)?;
    validate_format(stream.format, limits)?;

    let mut sink = match stream.format.tag {
        WAVE_FORMAT_PCM | WAVE_FORMAT_IEEE_FLOAT => PayloadSink::Pcm(Vec::new()),
        WAVE_FORMAT_MP3 | WAVE_FORMAT_RAW_AAC => PayloadSink::Compressed(
            tempfile::tempfile().map_err(|error| AudioError::Decode(error.to_string()))?,
        ),
        tag => {
            return Err(AudioError::Unsupported(format!(
                "AVI audio codec tag 0x{tag:04x} is not supported"
            )));
        }
    };
    collect_audio_payload(&mut file, file_len, stream, limits, &mut sink)?;

    match sink {
        PayloadSink::Pcm(samples) => {
            if samples.is_empty() {
                return Err(AudioError::Decode("AVI audio stream is empty".to_owned()));
            }
            finalize_pcm(samples, stream.format.sample_rate, limits)
        }
        PayloadSink::Compressed(mut payload) => {
            if payload.stream_position().map_err(decode_io)? == 0 {
                return Err(AudioError::Decode("AVI audio stream is empty".to_owned()));
            }
            payload.seek(SeekFrom::Start(0)).map_err(decode_io)?;
            let extension = if stream.format.tag == WAVE_FORMAT_MP3 {
                "mp3"
            } else {
                "aac"
            };
            decode_audio_file(payload, Some(extension), limits)
        }
    }
}

fn find_audio_stream(file: &mut File, file_len: u64) -> Result<AudioStream, AudioError> {
    let mut root_start = 0_u64;
    while root_start < file_len {
        let (form, content_start, root_end, next) = read_riff(file, root_start, file_len)?;
        if form == *b"AVI " {
            let mut position = content_start;
            while position < root_end {
                let chunk = read_chunk(file, position, root_end)?;
                if chunk.id == *b"LIST" && chunk.data_end.saturating_sub(chunk.data_start) >= 4 {
                    let list_type = read_fourcc(file, chunk.data_start)?;
                    if list_type == *b"hdrl"
                        && let Some(stream) =
                            parse_header_list(file, chunk.data_start + 4, chunk.data_end)?
                    {
                        return Ok(stream);
                    }
                }
                position = chunk.next;
            }
        }
        if next <= root_start {
            return Err(malformed("RIFF segment does not advance"));
        }
        root_start = next;
    }
    Err(AudioError::Unsupported(
        "AVI container has no audio stream".to_owned(),
    ))
}

fn parse_header_list(
    file: &mut File,
    start: u64,
    end: u64,
) -> Result<Option<AudioStream>, AudioError> {
    let mut position = start;
    let mut stream_index = 0_usize;
    while position < end {
        let chunk = read_chunk(file, position, end)?;
        if chunk.id == *b"LIST" && chunk.data_end.saturating_sub(chunk.data_start) >= 4 {
            let list_type = read_fourcc(file, chunk.data_start)?;
            if list_type == *b"strl" {
                let index = u8::try_from(stream_index).map_err(|_| {
                    AudioError::Unsupported("AVI has more than 256 streams".to_owned())
                })?;
                if let Some(format) = parse_stream_list(file, chunk.data_start + 4, chunk.data_end)?
                {
                    return Ok(Some(AudioStream { index, format }));
                }
                stream_index = stream_index.checked_add(1).ok_or_else(|| {
                    AudioError::Rejected("AVI stream count overflowed".to_owned())
                })?;
            }
        }
        position = chunk.next;
    }
    Ok(None)
}

fn parse_stream_list(
    file: &mut File,
    start: u64,
    end: u64,
) -> Result<Option<WaveFormat>, AudioError> {
    let mut position = start;
    let mut is_audio = false;
    let mut format_bytes = None;
    while position < end {
        let chunk = read_chunk(file, position, end)?;
        if chunk.id == *b"strh" {
            if chunk.data_end.saturating_sub(chunk.data_start) < 4 {
                return Err(malformed("AVI stream header is truncated"));
            }
            is_audio = read_fourcc(file, chunk.data_start)? == *b"auds";
        } else if chunk.id == *b"strf" {
            let length = usize::try_from(chunk.data_end - chunk.data_start)
                .map_err(|_| malformed("AVI stream format is too large"))?;
            if length > 256 {
                return Err(malformed("AVI stream format exceeds 256 bytes"));
            }
            format_bytes = Some(read_bytes(file, chunk.data_start, length)?);
        }
        position = chunk.next;
    }
    if !is_audio {
        return Ok(None);
    }
    let bytes = format_bytes.ok_or_else(|| malformed("AVI audio stream has no format"))?;
    parse_wave_format(&bytes).map(Some)
}

fn parse_wave_format(bytes: &[u8]) -> Result<WaveFormat, AudioError> {
    if bytes.len() < 16 {
        return Err(malformed("AVI audio format is truncated"));
    }
    let mut tag = le_u16(bytes, 0)?;
    let channels = usize::from(le_u16(bytes, 2)?);
    let sample_rate = usize::try_from(le_u32(bytes, 4)?)
        .map_err(|_| malformed("AVI sample rate is too large"))?;
    let block_align = usize::from(le_u16(bytes, 12)?);
    let bits_per_sample = usize::from(le_u16(bytes, 14)?);
    if tag == WAVE_FORMAT_EXTENSIBLE {
        if bytes.len() < 40 || le_u16(bytes, 16)? < 22 {
            return Err(malformed("AVI extensible audio format is truncated"));
        }
        const GUID_SUFFIX: [u8; 12] = [
            0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71,
        ];
        if bytes[28..40] != GUID_SUFFIX {
            return Err(AudioError::Unsupported(
                "AVI extensible audio subformat GUID is unknown".to_owned(),
            ));
        }
        let subformat = le_u32(bytes, 24)?;
        tag = u16::try_from(subformat).map_err(|_| {
            AudioError::Unsupported(format!(
                "AVI extensible audio codec tag 0x{subformat:08x} is not supported"
            ))
        })?;
    }
    let aac_config = if tag == WAVE_FORMAT_RAW_AAC {
        Some(parse_aac_config(bytes, sample_rate, channels)?)
    } else {
        None
    };
    Ok(WaveFormat {
        tag,
        channels,
        sample_rate,
        block_align,
        bits_per_sample,
        aac_config,
    })
}

fn parse_aac_config(
    bytes: &[u8],
    sample_rate: usize,
    channels: usize,
) -> Result<AacConfig, AudioError> {
    if bytes.len() < 20 {
        return Err(malformed("AVI AAC AudioSpecificConfig is missing"));
    }
    let extra_len = usize::from(le_u16(bytes, 16)?);
    let extra_end = 18_usize
        .checked_add(extra_len)
        .ok_or_else(|| malformed("AVI AAC codec metadata length overflowed"))?;
    if extra_len < 2 || extra_end > bytes.len() {
        return Err(malformed("AVI AAC AudioSpecificConfig is truncated"));
    }
    let first = bytes[18];
    let second = bytes[19];
    let object_type = first >> 3;
    let frequency_index = ((first & 0x07) << 1) | (second >> 7);
    let channel_config = (second >> 3) & 0x0f;
    if !(1..=4).contains(&object_type) {
        return Err(AudioError::Unsupported(format!(
            "AVI AAC object type {object_type} cannot be represented in ADTS"
        )));
    }
    const AAC_SAMPLE_RATES: [usize; 13] = [
        96_000, 88_200, 64_000, 48_000, 44_100, 32_000, 24_000, 22_050, 16_000, 12_000, 11_025,
        8_000, 7_350,
    ];
    let configured_rate = AAC_SAMPLE_RATES
        .get(usize::from(frequency_index))
        .ok_or_else(|| {
            AudioError::Unsupported(format!(
                "AVI AAC frequency index {frequency_index} cannot be represented in ADTS"
            ))
        })?;
    if *configured_rate != sample_rate {
        return Err(malformed(
            "AVI AAC sample rate disagrees with its AudioSpecificConfig",
        ));
    }
    if channel_config > 7 {
        return Err(AudioError::Unsupported(format!(
            "AVI AAC channel configuration {channel_config} cannot be represented in ADTS"
        )));
    }
    let configured_channels = match channel_config {
        0 => None,
        1..=6 => Some(usize::from(channel_config)),
        7 => Some(8),
        _ => None,
    };
    if configured_channels.is_some_and(|configured| configured != channels) {
        return Err(malformed(
            "AVI AAC channel count disagrees with its AudioSpecificConfig",
        ));
    }
    Ok(AacConfig {
        object_type,
        frequency_index,
        channel_config,
    })
}

fn validate_format(format: WaveFormat, limits: AudioLimits) -> Result<(), AudioError> {
    if format.channels == 0 || format.channels > 64 {
        return Err(AudioError::Rejected(format!(
            "AVI channel count {} is outside 1..=64",
            format.channels
        )));
    }
    if format.sample_rate == 0 || format.sample_rate > 768_000 {
        return Err(AudioError::Rejected(format!(
            "AVI sample rate {} is outside 1..=768000 Hz",
            format.sample_rate
        )));
    }
    if format.block_align == 0 {
        return Err(AudioError::Rejected(
            "AVI audio block alignment is zero".to_owned(),
        ));
    }
    format
        .sample_rate
        .checked_mul(limits.max_duration_seconds)
        .ok_or_else(|| AudioError::Rejected("AVI duration limit overflowed".to_owned()))?;

    if matches!(format.tag, WAVE_FORMAT_PCM | WAVE_FORMAT_IEEE_FLOAT) {
        let bytes_per_sample = match (format.tag, format.bits_per_sample) {
            (WAVE_FORMAT_PCM, 8 | 16 | 24 | 32) => format.bits_per_sample / 8,
            (WAVE_FORMAT_IEEE_FLOAT, 32 | 64) => format.bits_per_sample / 8,
            (WAVE_FORMAT_PCM, bits) => {
                return Err(AudioError::Unsupported(format!(
                    "AVI PCM with {bits} bits per sample is not supported"
                )));
            }
            (WAVE_FORMAT_IEEE_FLOAT, bits) => {
                return Err(AudioError::Unsupported(format!(
                    "AVI IEEE-float audio with {bits} bits per sample is not supported"
                )));
            }
            _ => {
                return Err(AudioError::Decode(
                    "internal AVI audio format mismatch".to_owned(),
                ));
            }
        };
        let minimum_alignment = format
            .channels
            .checked_mul(bytes_per_sample)
            .ok_or_else(|| malformed("AVI audio block alignment overflowed"))?;
        if format.block_align < minimum_alignment {
            return Err(malformed("AVI audio block alignment is too small"));
        }
    }
    Ok(())
}

fn collect_audio_payload(
    file: &mut File,
    file_len: u64,
    stream: AudioStream,
    limits: AudioLimits,
    sink: &mut PayloadSink,
) -> Result<(), AudioError> {
    let mut root_start = 0_u64;
    while root_start < file_len {
        let (form, content_start, root_end, next) = read_riff(file, root_start, file_len)?;
        if matches!(&form, b"AVI " | b"AVIX") {
            let mut position = content_start;
            while position < root_end {
                let chunk = read_chunk(file, position, root_end)?;
                if chunk.id == *b"LIST" && chunk.data_end.saturating_sub(chunk.data_start) >= 4 {
                    let list_type = read_fourcc(file, chunk.data_start)?;
                    if list_type == *b"movi" {
                        collect_chunks(
                            file,
                            chunk.data_start + 4,
                            chunk.data_end,
                            0,
                            stream,
                            limits,
                            sink,
                        )?;
                    }
                }
                position = chunk.next;
            }
        }
        root_start = next;
    }
    Ok(())
}

fn collect_chunks(
    file: &mut File,
    start: u64,
    end: u64,
    depth: usize,
    stream: AudioStream,
    limits: AudioLimits,
    sink: &mut PayloadSink,
) -> Result<(), AudioError> {
    if depth > MAX_LIST_DEPTH {
        return Err(AudioError::Rejected(format!(
            "AVI LIST nesting exceeds {MAX_LIST_DEPTH}"
        )));
    }
    let mut position = start;
    while position < end {
        let chunk = read_chunk(file, position, end)?;
        if chunk.id == *b"LIST" {
            if chunk.data_end.saturating_sub(chunk.data_start) < 4 {
                return Err(malformed("AVI LIST chunk is truncated"));
            }
            let list_type = read_fourcc(file, chunk.data_start)?;
            if matches!(&list_type, b"rec " | b"movi") {
                collect_chunks(
                    file,
                    chunk.data_start + 4,
                    chunk.data_end,
                    depth + 1,
                    stream,
                    limits,
                    sink,
                )?;
            }
        } else if audio_chunk_stream(chunk.id) == Some(stream.index) {
            consume_audio_chunk(file, chunk, stream.format, limits, sink)?;
        }
        position = chunk.next;
    }
    Ok(())
}

fn consume_audio_chunk(
    file: &mut File,
    chunk: Chunk,
    format: WaveFormat,
    limits: AudioLimits,
    sink: &mut PayloadSink,
) -> Result<(), AudioError> {
    let length = usize::try_from(chunk.data_end - chunk.data_start)
        .map_err(|_| malformed("AVI audio chunk is too large"))?;
    if length == 0 {
        return Ok(());
    }
    file.seek(SeekFrom::Start(chunk.data_start))
        .map_err(decode_io)?;
    match sink {
        PayloadSink::Compressed(payload) => {
            if format.tag == WAVE_FORMAT_RAW_AAC {
                let config = format.aac_config.ok_or_else(|| {
                    AudioError::Decode("internal AVI AAC configuration is missing".to_owned())
                })?;
                payload
                    .write_all(&adts_header(length, config)?)
                    .map_err(decode_io)?;
            }
            let mut bounded = file.take(length as u64);
            let copied = std::io::copy(&mut bounded, payload).map_err(decode_io)?;
            if copied != length as u64 {
                return Err(malformed("AVI audio chunk is truncated"));
            }
        }
        PayloadSink::Pcm(samples) => {
            if length % format.block_align != 0 {
                return Err(malformed("AVI PCM chunk is not block-aligned"));
            }
            let frames = length / format.block_align;
            let max_frames = format
                .sample_rate
                .checked_mul(limits.max_duration_seconds)
                .ok_or_else(|| AudioError::Rejected("AVI duration limit overflowed".to_owned()))?;
            if samples.len().saturating_add(frames) > max_frames {
                return Err(AudioError::Rejected(format!(
                    "decoded AVI audio exceeds {} seconds",
                    limits.max_duration_seconds
                )));
            }
            samples.try_reserve(frames).map_err(|error| {
                AudioError::Rejected(format!("AVI audio allocation was rejected: {error}"))
            })?;
            const TARGET_READ_BYTES: usize = 64 * 1024;
            let frames_per_read = (TARGET_READ_BYTES / format.block_align).max(1);
            let mut frames_remaining = frames;
            let mut bytes = vec![0_u8; frames_per_read.min(frames) * format.block_align];
            while frames_remaining > 0 {
                let frame_count = frames_remaining.min(frames_per_read);
                let byte_count = frame_count
                    .checked_mul(format.block_align)
                    .ok_or_else(|| malformed("AVI PCM read size overflowed"))?;
                file.read_exact(&mut bytes[..byte_count])
                    .map_err(decode_io)?;
                decode_pcm_chunk(&bytes[..byte_count], format, samples)?;
                frames_remaining -= frame_count;
            }
        }
    }
    Ok(())
}

fn adts_header(payload_len: usize, config: AacConfig) -> Result<[u8; 7], AudioError> {
    let frame_len = payload_len
        .checked_add(7)
        .ok_or_else(|| malformed("AVI AAC frame length overflowed"))?;
    if frame_len > 0x1fff {
        return Err(AudioError::Unsupported(format!(
            "AVI AAC access unit is {payload_len} bytes; ADTS permits at most 8184"
        )));
    }
    let frame_len = u16::try_from(frame_len)
        .map_err(|_| malformed("AVI AAC frame length cannot be represented"))?;
    let profile = config.object_type - 1;
    Ok([
        0xff,
        0xf1,
        (profile << 6) | (config.frequency_index << 2) | (config.channel_config >> 2),
        ((config.channel_config & 0x03) << 6) | ((frame_len >> 11) as u8 & 0x03),
        (frame_len >> 3) as u8,
        ((frame_len as u8 & 0x07) << 5) | 0x1f,
        0xfc,
    ])
}

fn decode_pcm_chunk(
    bytes: &[u8],
    format: WaveFormat,
    output: &mut Vec<f32>,
) -> Result<(), AudioError> {
    let bytes_per_sample = format.bits_per_sample / 8;
    for frame in bytes.chunks_exact(format.block_align) {
        let mut mono = 0.0_f64;
        for channel in 0..format.channels {
            let start = channel
                .checked_mul(bytes_per_sample)
                .ok_or_else(|| malformed("AVI PCM channel offset overflowed"))?;
            let end = start
                .checked_add(bytes_per_sample)
                .ok_or_else(|| malformed("AVI PCM sample offset overflowed"))?;
            let sample = decode_sample(
                frame
                    .get(start..end)
                    .ok_or_else(|| malformed("AVI PCM sample is truncated"))?,
                format.tag,
                format.bits_per_sample,
            )?;
            mono += f64::from(sample);
        }
        let sample = (mono / format.channels as f64) as f32;
        if !sample.is_finite() {
            return Err(AudioError::Rejected(
                "AVI audio contains a non-finite sample".to_owned(),
            ));
        }
        output.push(sample);
    }
    Ok(())
}

fn decode_sample(bytes: &[u8], tag: u16, bits: usize) -> Result<f32, AudioError> {
    match (tag, bits) {
        (WAVE_FORMAT_PCM, 8) => Ok((f32::from(bytes[0]) - 128.0) / 128.0),
        (WAVE_FORMAT_PCM, 16) => Ok(f32::from(i16::from_le_bytes([bytes[0], bytes[1]])) / 32_768.0),
        (WAVE_FORMAT_PCM, 24) => {
            let extended = [
                bytes[0],
                bytes[1],
                bytes[2],
                if bytes[2] & 0x80 == 0 { 0 } else { 0xff },
            ];
            Ok(i32::from_le_bytes(extended) as f32 / 8_388_608.0)
        }
        (WAVE_FORMAT_PCM, 32) => Ok(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
            as f32
            / 2_147_483_648.0),
        (WAVE_FORMAT_IEEE_FLOAT, 32) => {
            Ok(f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
        }
        (WAVE_FORMAT_IEEE_FLOAT, 64) => Ok(f64::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]) as f32),
        _ => Err(AudioError::Decode(
            "internal AVI sample format mismatch".to_owned(),
        )),
    }
}

fn audio_chunk_stream(id: [u8; 4]) -> Option<u8> {
    if &id[2..] != b"wb" {
        return None;
    }
    let high = hex_value(id[0])?;
    let low = hex_value(id[1])?;
    high.checked_mul(16)?.checked_add(low)
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn read_riff(
    file: &mut File,
    start: u64,
    limit: u64,
) -> Result<([u8; 4], u64, u64, u64), AudioError> {
    if limit.saturating_sub(start) < RIFF_HEADER_LEN {
        return Err(malformed("trailing bytes do not form a RIFF segment"));
    }
    let header = read_bytes(file, start, RIFF_HEADER_LEN as usize)?;
    if &header[..4] != b"RIFF" {
        return Err(malformed("AVI contains a non-RIFF top-level segment"));
    }
    let size = u64::from(le_u32(&header, 4)?);
    if size < 4 {
        return Err(malformed("AVI RIFF segment is too small"));
    }
    let end = start
        .checked_add(CHUNK_HEADER_LEN)
        .and_then(|value| value.checked_add(size))
        .ok_or_else(|| malformed("AVI RIFF size overflowed"))?;
    if end > limit {
        return Err(malformed("AVI RIFF segment exceeds the file"));
    }
    let next = align_even(end)?;
    if next > limit && end != limit {
        return Err(malformed("AVI RIFF padding exceeds the file"));
    }
    Ok((
        header[8..12]
            .try_into()
            .map_err(|_| malformed("AVI form type is truncated"))?,
        start + RIFF_HEADER_LEN,
        end,
        next.min(limit),
    ))
}

fn read_chunk(file: &mut File, start: u64, limit: u64) -> Result<Chunk, AudioError> {
    if limit.saturating_sub(start) < CHUNK_HEADER_LEN {
        return Err(malformed("AVI chunk header is truncated"));
    }
    let header = read_bytes(file, start, CHUNK_HEADER_LEN as usize)?;
    let length = u64::from(le_u32(&header, 4)?);
    let data_start = start + CHUNK_HEADER_LEN;
    let data_end = data_start
        .checked_add(length)
        .ok_or_else(|| malformed("AVI chunk size overflowed"))?;
    if data_end > limit {
        return Err(malformed("AVI chunk exceeds its containing list"));
    }
    let next = align_even(data_end)?;
    if next > limit && data_end != limit {
        return Err(malformed("AVI chunk padding exceeds its containing list"));
    }
    Ok(Chunk {
        id: header[..4]
            .try_into()
            .map_err(|_| malformed("AVI chunk ID is truncated"))?,
        data_start,
        data_end,
        next: next.min(limit),
    })
}

fn align_even(value: u64) -> Result<u64, AudioError> {
    value
        .checked_add(value & 1)
        .ok_or_else(|| malformed("AVI alignment overflowed"))
}

fn read_fourcc(file: &mut File, offset: u64) -> Result<[u8; 4], AudioError> {
    read_bytes(file, offset, 4)?
        .try_into()
        .map_err(|_| malformed("AVI FourCC is truncated"))
}

fn read_bytes(file: &mut File, offset: u64, length: usize) -> Result<Vec<u8>, AudioError> {
    file.seek(SeekFrom::Start(offset)).map_err(decode_io)?;
    let mut bytes = vec![0_u8; length];
    file.read_exact(&mut bytes).map_err(decode_io)?;
    Ok(bytes)
}

fn le_u16(bytes: &[u8], offset: usize) -> Result<u16, AudioError> {
    let value = bytes
        .get(offset..offset.saturating_add(2))
        .ok_or_else(|| malformed("AVI integer is truncated"))?;
    Ok(u16::from_le_bytes(
        value
            .try_into()
            .map_err(|_| malformed("AVI integer is truncated"))?,
    ))
}

fn le_u32(bytes: &[u8], offset: usize) -> Result<u32, AudioError> {
    let value = bytes
        .get(offset..offset.saturating_add(4))
        .ok_or_else(|| malformed("AVI integer is truncated"))?;
    Ok(u32::from_le_bytes(
        value
            .try_into()
            .map_err(|_| malformed("AVI integer is truncated"))?,
    ))
}

fn decode_io(error: std::io::Error) -> AudioError {
    AudioError::Decode(error.to_string())
}

fn malformed(message: &str) -> AudioError {
    AudioError::Rejected(format!("malformed AVI: {message}"))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn detects_content_without_avi_extension() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("recording.bin");
        fs::write(&path, pcm_avi(16_000, 1, &[[1_000_i16], [-1_000_i16]]))?;
        assert!(is_avi(&path)?);
        Ok(())
    }

    #[test]
    fn decodes_pcm_avi_to_whisper_mono() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("stereo.avi");
        let frames = [[i16::MAX, i16::MIN], [8_000, 4_000], [-4_000, -8_000]];
        fs::write(&path, pcm_avi(16_000, 2, &frames))?;
        let decoded = crate::audio::decode_audio(&path, AudioLimits::default())?;
        assert_eq!(decoded.sample_rate, 16_000);
        assert_eq!(decoded.samples.len(), 3);
        assert!(decoded.samples[0].abs() < 0.000_1);
        assert!((decoded.samples[1] - (6_000.0 / 32_768.0)).abs() < 0.000_1);
        Ok(())
    }

    #[test]
    fn finds_audio_after_video_and_inside_record_list() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("interleaved.avi");
        fs::write(&path, video_then_audio_avi())?;
        let decoded = crate::audio::decode_audio(&path, AudioLimits::default())?;
        assert_eq!(decoded.samples.len(), 2);
        assert!((decoded.samples[0] - (1_000.0 / 32_768.0)).abs() < 0.000_1);
        Ok(())
    }

    #[test]
    fn rejects_misaligned_pcm_chunk() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("bad.avi");
        let mut avi = pcm_avi(16_000, 1, &[[1_000_i16]]);
        let index = avi
            .windows(4)
            .position(|window| window == b"00wb")
            .ok_or("fixture has no audio chunk")?;
        avi[index + 4..index + 8].copy_from_slice(&1_u32.to_le_bytes());
        fs::write(&path, avi)?;
        let error = match crate::audio::decode_audio(&path, AudioLimits::default()) {
            Ok(_) => return Err("misaligned PCM must fail".into()),
            Err(error) => error,
        };
        assert!(error.to_string().contains("block-aligned"));
        Ok(())
    }

    #[test]
    fn rejects_duration_before_decoding_payload() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("long.avi");
        fs::write(&path, pcm_avi(2, 1, &[[1_i16], [2_i16], [3_i16]]))?;
        let result = crate::audio::decode_audio(
            &path,
            AudioLimits {
                max_source_bytes: 1_024 * 1_024,
                max_duration_seconds: 1,
            },
        );
        let error = match result {
            Ok(_) => return Err("duration limit must fail".into()),
            Err(error) => error,
        };
        assert!(error.to_string().contains("exceeds 1 seconds"));
        Ok(())
    }

    #[test]
    fn converts_avi_aac_metadata_to_adts() -> Result<(), Box<dyn std::error::Error>> {
        let mut format = pcm_format(48_000, 1);
        format[..2].copy_from_slice(&WAVE_FORMAT_RAW_AAC.to_le_bytes());
        format.extend_from_slice(&5_u16.to_le_bytes());
        format.extend_from_slice(&[0x11, 0x88, 0x56, 0xe5, 0x00]);
        let parsed = parse_wave_format(&format)?;
        let config = parsed.aac_config.ok_or("AAC config was not parsed")?;
        assert_eq!(config.object_type, 2);
        assert_eq!(config.frequency_index, 3);
        assert_eq!(config.channel_config, 1);
        assert_eq!(
            adts_header(100, config)?,
            [0xff, 0xf1, 0x4c, 0x40, 0x0d, 0x7f, 0xfc]
        );
        Ok(())
    }

    #[test]
    fn wave_formats_samples_and_chunk_ids_cover_supported_matrix()
    -> Result<(), Box<dyn std::error::Error>> {
        for (tag, bits, bytes, expected) in [
            (WAVE_FORMAT_PCM, 8, vec![255], 127.0 / 128.0),
            (
                WAVE_FORMAT_PCM,
                16,
                i16::MAX.to_le_bytes().to_vec(),
                i16::MAX as f32 / 32_768.0,
            ),
            (
                WAVE_FORMAT_PCM,
                24,
                vec![0xff, 0xff, 0x7f],
                8_388_607.0 / 8_388_608.0,
            ),
            (WAVE_FORMAT_PCM, 32, i32::MIN.to_le_bytes().to_vec(), -1.0),
            (
                WAVE_FORMAT_IEEE_FLOAT,
                32,
                0.25_f32.to_le_bytes().to_vec(),
                0.25,
            ),
            (
                WAVE_FORMAT_IEEE_FLOAT,
                64,
                0.5_f64.to_le_bytes().to_vec(),
                0.5,
            ),
        ] {
            assert!((decode_sample(&bytes, tag, bits)? - expected).abs() < 0.000_01);
        }
        assert!(decode_sample(&[0; 2], 99, 16).is_err());
        assert_eq!(audio_chunk_stream(*b"00wb"), Some(0));
        assert_eq!(audio_chunk_stream(*b"Afwb"), Some(175));
        assert_eq!(audio_chunk_stream(*b"ggwb"), None);
        assert_eq!(audio_chunk_stream(*b"00dc"), None);
        assert_eq!(align_even(3)?, 4);
        assert_eq!(align_even(4)?, 4);
        assert!(align_even(u64::MAX).is_err());
        assert!(le_u16(&[], 0).is_err());
        assert!(le_u32(&[0; 3], 0).is_err());

        let limits = AudioLimits::default();
        for (channels, rate, alignment, tag, bits) in [
            (0, 16_000, 2, WAVE_FORMAT_PCM, 16),
            (65, 16_000, 130, WAVE_FORMAT_PCM, 16),
            (1, 0, 2, WAVE_FORMAT_PCM, 16),
            (1, 768_001, 2, WAVE_FORMAT_PCM, 16),
            (1, 16_000, 0, WAVE_FORMAT_PCM, 16),
            (1, 16_000, 2, WAVE_FORMAT_PCM, 12),
            (1, 16_000, 4, WAVE_FORMAT_IEEE_FLOAT, 16),
            (2, 16_000, 2, WAVE_FORMAT_PCM, 16),
        ] {
            assert!(
                validate_format(
                    WaveFormat {
                        tag,
                        channels,
                        sample_rate: rate,
                        block_align: alignment,
                        bits_per_sample: bits,
                        aac_config: None,
                    },
                    limits
                )
                .is_err()
            );
        }
        Ok(())
    }

    #[test]
    fn malformed_wave_and_aac_metadata_are_bounded() -> Result<(), Box<dyn std::error::Error>> {
        assert!(parse_wave_format(&[0; 15]).is_err());
        let mut extensible = pcm_format(16_000, 1);
        extensible[..2].copy_from_slice(&WAVE_FORMAT_EXTENSIBLE.to_le_bytes());
        assert!(parse_wave_format(&extensible).is_err());
        extensible.resize(40, 0);
        extensible[16..18].copy_from_slice(&22_u16.to_le_bytes());
        assert!(parse_wave_format(&extensible).is_err());

        let mut aac = pcm_format(48_000, 1);
        aac[..2].copy_from_slice(&WAVE_FORMAT_RAW_AAC.to_le_bytes());
        assert!(parse_wave_format(&aac).is_err());
        aac.extend_from_slice(&1_u16.to_le_bytes());
        aac.push(0);
        assert!(parse_wave_format(&aac).is_err());

        let config = AacConfig {
            object_type: 2,
            frequency_index: 3,
            channel_config: 1,
        };
        assert!(adts_header(8_185, config).is_err());
        for (object_type, frequency_index, channel_config) in
            [(0_u8, 3_u8, 1_u8), (5, 3, 1), (2, 15, 1), (2, 3, 8)]
        {
            let mut format = pcm_format(48_000, 1);
            format[..2].copy_from_slice(&WAVE_FORMAT_RAW_AAC.to_le_bytes());
            format.extend_from_slice(&2_u16.to_le_bytes());
            format.push((object_type << 3) | (frequency_index >> 1));
            format.push(((frequency_index & 1) << 7) | (channel_config << 3));
            assert!(parse_wave_format(&format).is_err());
        }
        Ok(())
    }

    #[test]
    fn detection_and_container_failures_are_structured() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("fixture.bin");
        fs::write(&path, b"short")?;
        assert!(!is_avi(&path)?);
        fs::write(&path, b"RIFF\0\0\0\0WAVE")?;
        assert!(!is_avi(&path)?);
        assert!(is_avi(&directory.path().join("missing")).is_err());

        for bytes in [
            Vec::new(),
            b"not a riff!!".to_vec(),
            b"RIFF\x03\0\0\0AVI ".to_vec(),
            b"RIFF\x20\0\0\0AVI ".to_vec(),
        ] {
            fs::write(&path, bytes)?;
            assert!(decode_avi_audio(&path, AudioLimits::default()).is_err());
        }

        fs::write(&path, riff(*b"AVI ", &[]))?;
        assert!(
            decode_avi_audio(&path, AudioLimits::default())
                .is_err_and(|error| error.to_string().contains("no audio stream"))
        );
        Ok(())
    }

    fn pcm_avi<const CHANNELS: usize>(
        sample_rate: u32,
        channels: u16,
        frames: &[[i16; CHANNELS]],
    ) -> Vec<u8> {
        let strh = chunk(*b"strh", &audio_stream_header());
        let strf = chunk(*b"strf", &pcm_format(sample_rate, channels));
        let strl = list(*b"strl", &[strh, strf].concat());
        let hdrl = list(*b"hdrl", &strl);
        let mut payload = Vec::new();
        for frame in frames {
            for sample in frame {
                payload.extend_from_slice(&sample.to_le_bytes());
            }
        }
        let movi = list(*b"movi", &chunk(*b"00wb", &payload));
        riff(*b"AVI ", &[hdrl, movi].concat())
    }

    fn video_then_audio_avi() -> Vec<u8> {
        let video_strl = list(
            *b"strl",
            &[
                chunk(*b"strh", &video_stream_header()),
                chunk(*b"strf", &[]),
            ]
            .concat(),
        );
        let audio_strl = list(
            *b"strl",
            &[
                chunk(*b"strh", &audio_stream_header()),
                chunk(*b"strf", &pcm_format(16_000, 1)),
            ]
            .concat(),
        );
        let hdrl = list(*b"hdrl", &[video_strl, audio_strl].concat());
        let mut payload = Vec::new();
        payload.extend_from_slice(&1_000_i16.to_le_bytes());
        payload.extend_from_slice(&(-1_000_i16).to_le_bytes());
        let record = list(*b"rec ", &chunk(*b"01wb", &payload));
        let movi = list(*b"movi", &record);
        riff(*b"AVI ", &[hdrl, movi].concat())
    }

    fn audio_stream_header() -> Vec<u8> {
        let mut bytes = vec![0_u8; 56];
        bytes[..4].copy_from_slice(b"auds");
        bytes
    }

    fn video_stream_header() -> Vec<u8> {
        let mut bytes = vec![0_u8; 56];
        bytes[..4].copy_from_slice(b"vids");
        bytes
    }

    fn pcm_format(sample_rate: u32, channels: u16) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(16);
        bytes.extend_from_slice(&WAVE_FORMAT_PCM.to_le_bytes());
        bytes.extend_from_slice(&channels.to_le_bytes());
        bytes.extend_from_slice(&sample_rate.to_le_bytes());
        bytes.extend_from_slice(&(sample_rate * u32::from(channels) * 2).to_le_bytes());
        bytes.extend_from_slice(&(channels * 2).to_le_bytes());
        bytes.extend_from_slice(&16_u16.to_le_bytes());
        bytes
    }

    fn riff(form: [u8; 4], payload: &[u8]) -> Vec<u8> {
        let size = u32::try_from(payload.len() + 4).unwrap_or_default();
        let mut bytes = Vec::with_capacity(payload.len() + 12);
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&size.to_le_bytes());
        bytes.extend_from_slice(&form);
        bytes.extend_from_slice(payload);
        bytes
    }

    fn list(form: [u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut content = Vec::with_capacity(payload.len() + 4);
        content.extend_from_slice(&form);
        content.extend_from_slice(payload);
        chunk(*b"LIST", &content)
    }

    fn chunk(id: [u8; 4], payload: &[u8]) -> Vec<u8> {
        let size = u32::try_from(payload.len()).unwrap_or_default();
        let mut bytes = Vec::with_capacity(payload.len() + 9);
        bytes.extend_from_slice(&id);
        bytes.extend_from_slice(&size.to_le_bytes());
        bytes.extend_from_slice(payload);
        if payload.len() % 2 == 1 {
            bytes.push(0);
        }
        bytes
    }
}
