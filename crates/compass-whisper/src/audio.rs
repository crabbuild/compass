//! Audio loading and log-Mel spectrogram. Port of `whisper/audio.py`.
//!
//! The STFT reproduces `torch.stft(..., center=True, pad_mode="reflect")`
//! exactly (Hann window, reflect padding of n_fft/2 on both sides, last
//! frame dropped), unlike the whisper.cpp-style approximation.

use anyhow::{Result, bail};
use realfft::RealFftPlanner;

pub const SAMPLE_RATE: usize = 16000;
pub const N_FFT: usize = 400;
pub const HOP_LENGTH: usize = 160;
pub const CHUNK_LENGTH: usize = 30;
/// 480000 samples in a 30-second chunk
pub const N_SAMPLES: usize = CHUNK_LENGTH * SAMPLE_RATE;
/// 3000 frames in a mel spectrogram input
pub const N_FRAMES: usize = N_SAMPLES / HOP_LENGTH;
/// 10ms per audio frame
pub const FRAMES_PER_SECOND: usize = SAMPLE_RATE / HOP_LENGTH;
/// 20ms per audio token
pub const TOKENS_PER_SECOND: usize = SAMPLE_RATE / (HOP_LENGTH * 2);

const MEL_80_BYTES: &[u8] = include_bytes!("../assets/mel_80.bytes");
const MEL_128_BYTES: &[u8] = include_bytes!("../assets/mel_128.bytes");
const N_FREQS: usize = N_FFT / 2 + 1; // 201

/// Mel filterbank matrix, row-major (n_mels, 201). Same values as
/// `librosa.filters.mel(sr=16000, n_fft=400, n_mels=..)` shipped with whisper.
pub fn mel_filters(n_mels: usize) -> Result<Vec<f32>> {
    let bytes = match n_mels {
        80 => MEL_80_BYTES,
        128 => MEL_128_BYTES,
        _ => bail!("unsupported n_mels: {n_mels}"),
    };
    Ok(bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect())
}

/// Log-Mel spectrogram, row-major (n_mels, n_frames).
/// `padding` zero samples are appended before the STFT (transcribe passes
/// N_SAMPLES so the last window can always be sliced).
pub struct MelSpectrogram {
    pub data: Vec<f32>,
    pub n_mels: usize,
    pub n_frames: usize,
}

impl MelSpectrogram {
    /// Slice frames [seek, seek+len) and zero-pad on the right to `target`
    /// frames — the combination of mel slicing + `pad_or_trim` in transcribe.
    /// Returns row-major (n_mels, target).
    pub fn window(&self, seek: usize, len: usize, target: usize) -> Vec<f32> {
        let len = len.min(self.n_frames.saturating_sub(seek)).min(target);
        let mut out = vec![0f32; self.n_mels * target];
        for m in 0..self.n_mels {
            let src = &self.data[m * self.n_frames + seek..m * self.n_frames + seek + len];
            out[m * target..m * target + len].copy_from_slice(src);
        }
        out
    }
}

pub fn log_mel_spectrogram(audio: &[f32], n_mels: usize, padding: usize) -> Result<MelSpectrogram> {
    let filters = mel_filters(n_mels)?;

    let mut samples = Vec::with_capacity(audio.len() + padding);
    samples.extend_from_slice(audio);
    samples.resize(audio.len() + padding, 0.0);

    // torch.stft(center=True): reflect-pad n_fft/2 on both sides
    let half = N_FFT / 2;
    if samples.len() <= half {
        bail!("audio too short: {} samples", samples.len());
    }
    let mut padded = Vec::with_capacity(samples.len() + N_FFT);
    for i in (1..=half).rev() {
        padded.push(samples[i]);
    }
    padded.extend_from_slice(&samples);
    for i in (samples.len().saturating_sub(half + 1)..samples.len() - 1).rev() {
        padded.push(samples[i]);
    }

    // periodic Hann window, matching torch.hann_window(N_FFT)
    let hann: Vec<f32> = (0..N_FFT)
        .map(|i| 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / N_FFT as f32).cos()))
        .collect();

    // torch yields 1 + len/hop frames; whisper drops the last one
    let n_frames_total = samples.len() / HOP_LENGTH + 1;
    let n_frames = n_frames_total - 1;

    let mut planner = RealFftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(N_FFT);
    let mut fft_in = fft.make_input_vec();
    let mut fft_out = fft.make_output_vec();

    // power spectrum, column-major staging: magnitudes[frame][freq]
    let mut magnitudes = vec![0f32; n_frames * N_FREQS];
    for t in 0..n_frames {
        let start = t * HOP_LENGTH;
        for i in 0..N_FFT {
            fft_in[i] = padded[start + i] * hann[i];
        }
        fft.process(&mut fft_in, &mut fft_out)
            .map_err(|e| anyhow::anyhow!("fft failed: {e:?}"))?;
        for (f, c) in fft_out.iter().enumerate() {
            magnitudes[t * N_FREQS + f] = c.re * c.re + c.im * c.im;
        }
    }

    // mel = filters (n_mels, 201) @ magnitudes^T (201, n_frames)
    let mut mel = vec![0f32; n_mels * n_frames];
    for m in 0..n_mels {
        let filt = &filters[m * N_FREQS..(m + 1) * N_FREQS];
        for t in 0..n_frames {
            let mag = &magnitudes[t * N_FREQS..(t + 1) * N_FREQS];
            let mut sum = 0f32;
            for f in 0..N_FREQS {
                sum += filt[f] * mag[f];
            }
            mel[m * n_frames + t] = sum.max(1e-10).log10();
        }
    }

    let max = mel.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    for v in mel.iter_mut() {
        *v = (v.max(max - 8.0) + 4.0) / 4.0;
    }

    Ok(MelSpectrogram {
        data: mel,
        n_mels,
        n_frames,
    })
}
