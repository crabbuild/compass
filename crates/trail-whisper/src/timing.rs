//! Word-level timestamps via cross-attention alignment + DTW.
//! Port of `whisper/timing.py` (the numba/triton kernels become plain Rust).

use crate::audio::{HOP_LENGTH, SAMPLE_RATE, TOKENS_PER_SECOND};
use crate::model::WhisperModel;
use crate::tokenizer::Tokenizer;
use crate::transcribe::{Segment, Word};
use anyhow::Result;
use candle_core::{IndexOp, Tensor};

/// Median filter along the last axis of a (rows, width) matrix, reflect-padded.
/// Port of `timing.py::median_filter` (returns input unchanged when the axis
/// is too short, like the Python guard).
pub fn median_filter_rows(data: &mut [f32], n_cols: usize, filter_width: usize) {
    assert!(
        filter_width % 2 == 1 && filter_width > 0,
        "filter_width must be odd"
    );
    let pad = filter_width / 2;
    if n_cols <= pad {
        return;
    }
    let mut padded = vec![0f32; n_cols + 2 * pad];
    let mut window = vec![0f32; filter_width];
    for row in data.chunks_exact_mut(n_cols) {
        // reflect padding: [x[p], .., x[1], x[0], x[1], .., x[n-2], ..]
        for i in 0..pad {
            padded[i] = row[pad - i];
            padded[n_cols + pad + i] = row[n_cols - 2 - i];
        }
        padded[pad..pad + n_cols].copy_from_slice(row);
        for (i, out) in row.iter_mut().enumerate() {
            window.copy_from_slice(&padded[i..i + filter_width]);
            window.sort_by(f32::total_cmp);
            *out = window[pad];
        }
    }
}

/// Dynamic time warping over a cost matrix (rows = text tokens, cols = time
/// frames). Returns the alignment path as (text_indices, time_indices).
/// Port of `timing.py::dtw_cpu` + `backtrace`, including tie-breaking.
pub fn dtw(x: &[f64], n: usize, m: usize) -> (Vec<usize>, Vec<usize>) {
    let w = m + 1;
    let mut cost = vec![f64::INFINITY; (n + 1) * w];
    let mut trace = vec![-1i8; (n + 1) * w];
    cost[0] = 0.0;

    for j in 1..=m {
        for i in 1..=n {
            let c0 = cost[(i - 1) * w + (j - 1)];
            let c1 = cost[(i - 1) * w + j];
            let c2 = cost[i * w + (j - 1)];
            let (c, t) = if c0 < c1 && c0 < c2 {
                (c0, 0)
            } else if c1 < c0 && c1 < c2 {
                (c1, 1)
            } else {
                (c2, 2)
            };
            cost[i * w + j] = x[(i - 1) * m + (j - 1)] + c;
            trace[i * w + j] = t;
        }
    }

    // backtrace
    trace[..=m].fill(2);
    for i in 0..=n {
        trace[i * w] = 1;
    }
    let (mut i, mut j) = (n, m);
    let mut text_indices = Vec::new();
    let mut time_indices = Vec::new();
    while i > 0 || j > 0 {
        text_indices.push(i - 1);
        time_indices.push(j - 1);
        match trace[i * w + j] {
            0 => {
                i -= 1;
                j -= 1;
            }
            1 => i -= 1,
            2 => j -= 1,
            _ => unreachable!("invalid trace"),
        }
    }
    text_indices.reverse();
    time_indices.reverse();
    (text_indices, time_indices)
}

#[derive(Debug, Clone)]
pub struct WordTiming {
    pub word: String,
    pub tokens: Vec<u32>,
    pub start: f64,
    pub end: f64,
    pub probability: f64,
}

/// Port of `timing.py::find_alignment`.
pub fn find_alignment(
    model: &mut WhisperModel,
    tokenizer: &Tokenizer,
    text_tokens: &[u32],
    mel: &Tensor,
    num_frames: usize,
    medfilt_width: usize,
    qk_scale: f32,
) -> Result<Vec<WordTiming>> {
    if text_tokens.is_empty() {
        return Ok(Vec::new());
    }

    let mut tokens: Vec<u32> = tokenizer.sot_sequence.clone();
    tokens.push(tokenizer.no_timestamps);
    tokens.extend_from_slice(text_tokens);
    tokens.push(tokenizer.eot);
    let seq_len = tokens.len();
    let sot_len = tokenizer.sot_sequence.len();

    let features = model.encoder_forward(mel, true)?;
    let tokens_t = Tensor::from_vec(tokens, (1, seq_len), &model.device)?;
    let (hidden, cross_qks) = model.decoder_forward_with_cross_qk(&tokens_t, &features)?;

    // per-token probabilities: position sot_len + i predicts text_tokens[i];
    // softmax restricted to non-special logits [..eot] (timing.py:198-201)
    let logits = model.decoder_final_linear(&hidden.i((.., sot_len.., ..))?)?;
    let logits_rows: Vec<Vec<f32>> = logits.i(0)?.to_vec2()?;
    let eot_idx = tokenizer.eot as usize;
    let text_token_probs: Vec<f64> = text_tokens
        .iter()
        .enumerate()
        .map(|(i, &tok)| {
            let row = &logits_rows[i][..eot_idx];
            let max = row.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            let sum: f32 = row.iter().map(|&x| (x - max).exp()).sum();
            ((row[tok as usize] - max).exp() / sum) as f64
        })
        .collect();

    // alignment weights: selected heads, sliced to the content frames
    let half = num_frames / 2;
    let heads = model.alignment_heads();
    let n_heads = heads.len();
    let mut weights: Vec<Vec<f32>> = Vec::with_capacity(n_heads); // each (seq_len * half)
    for (l, h) in heads {
        let qk = cross_qks[l].i((0, h, .., ..half))?; // (seq_len, half)
        let mut w: Vec<f32> = qk.flatten_all()?.to_vec1()?;
        // scaled softmax over frames
        for row in w.chunks_exact_mut(half) {
            let max = row
                .iter()
                .map(|v| v * qk_scale)
                .fold(f32::NEG_INFINITY, f32::max);
            let mut sum = 0f32;
            for v in row.iter_mut() {
                *v = (*v * qk_scale - max).exp();
                sum += *v;
            }
            for v in row.iter_mut() {
                *v /= sum;
            }
        }
        weights.push(w);
    }

    // normalize per (head, frame) across tokens: std_mean(dim=-2, unbiased=False)
    for w in weights.iter_mut() {
        for f in 0..half {
            let mut mean = 0f64;
            for t in 0..seq_len {
                mean += w[t * half + f] as f64;
            }
            mean /= seq_len as f64;
            let mut var = 0f64;
            for t in 0..seq_len {
                let d = w[t * half + f] as f64 - mean;
                var += d * d;
            }
            let std = (var / seq_len as f64).sqrt();
            for t in 0..seq_len {
                w[t * half + f] = ((w[t * half + f] as f64 - mean) / std) as f32;
            }
        }
        median_filter_rows(w, half, medfilt_width);
    }

    // mean over heads, then keep rows [sot_len, seq_len-1)
    let n_rows = seq_len - 1 - sot_len;
    let mut matrix = vec![0f64; n_rows * half];
    for w in &weights {
        for r in 0..n_rows {
            for f in 0..half {
                matrix[r * half + f] += w[(sot_len + r) * half + f] as f64;
            }
        }
    }
    for v in matrix.iter_mut() {
        *v = -(*v / n_heads as f64); // negate: DTW minimizes cost
    }

    let (text_indices, time_indices) = dtw(&matrix, n_rows, half);

    let mut with_eot: Vec<u32> = text_tokens.to_vec();
    with_eot.push(tokenizer.eot);
    let (words, word_tokens) = tokenizer.split_to_word_tokens(&with_eot);
    if word_tokens.len() <= 1 {
        return Ok(Vec::new());
    }

    // word boundaries in token space (excluding the trailing eot word)
    let mut word_boundaries = vec![0usize];
    let mut acc = 0usize;
    for t in &word_tokens[..word_tokens.len() - 1] {
        acc += t.len();
        word_boundaries.push(acc);
    }

    // jump_times[k] = first time index where the DTW path enters text row k
    let mut jump_times = vec![0f64; n_rows];
    let mut prev = usize::MAX;
    for (ti, fi) in text_indices.iter().zip(&time_indices) {
        if *ti != prev {
            jump_times[*ti] = *fi as f64 / TOKENS_PER_SECOND as f64;
            prev = *ti;
        }
    }

    let n_words = word_boundaries.len() - 1;
    let mut result = Vec::with_capacity(n_words);
    for k in 0..n_words {
        let (b0, b1) = (word_boundaries[k], word_boundaries[k + 1]);
        let probability = if b1 > b0 {
            text_token_probs[b0..b1.min(text_token_probs.len())]
                .iter()
                .sum::<f64>()
                / (b1.min(text_token_probs.len()) - b0).max(1) as f64
        } else {
            0.0
        };
        result.push(WordTiming {
            word: words[k].clone(),
            tokens: word_tokens[k].clone(),
            start: jump_times[b0],
            end: jump_times[b1],
            probability,
        });
    }
    Ok(result)
}

/// Port of `timing.py::merge_punctuations`.
pub fn merge_punctuations(alignment: &mut [WordTiming], prepended: &str, appended: &str) {
    // merge prepended punctuations into the following word
    if alignment.len() >= 2 {
        let mut i = alignment.len() - 2;
        let mut j = alignment.len() - 1;
        loop {
            // NB: Python's `x in str` is a substring test and is vacuously
            // true for ""; keep those semantics — they drive the index walk.
            let prev_word = alignment[i].word.clone();
            if prev_word.starts_with(' ') && prepended.contains(prev_word.trim()) {
                alignment[j].word = format!("{}{}", prev_word, alignment[j].word);
                let mut tokens = alignment[i].tokens.clone();
                tokens.extend(&alignment[j].tokens);
                alignment[j].tokens = tokens;
                alignment[i].word.clear();
                alignment[i].tokens.clear();
            } else {
                j = i;
            }
            if i == 0 {
                break;
            }
            i -= 1;
        }
    }

    // merge appended punctuations into the previous word
    let mut i = 0;
    let mut j = 1;
    while j < alignment.len() {
        let following_word = alignment[j].word.clone();
        if !alignment[i].word.ends_with(' ') && appended.contains(&following_word) {
            alignment[i].word.push_str(&following_word);
            let tokens = alignment[j].tokens.clone();
            alignment[i].tokens.extend(tokens);
            alignment[j].word.clear();
            alignment[j].tokens.clear();
        } else {
            i = j;
        }
        j += 1;
    }
}

const SENTENCE_END_MARKS: &[&str] = &[".", "。", "!", "！", "?", "？"];

/// Port of `timing.py::add_word_timestamps` — attaches `words` to each
/// segment and applies the boundary/duration heuristics.
#[allow(clippy::too_many_arguments)]
pub fn add_word_timestamps(
    segments: &mut [Segment],
    model: &mut WhisperModel,
    tokenizer: &Tokenizer,
    mel: &Tensor,
    num_frames: usize,
    prepend_punctuations: &str,
    append_punctuations: &str,
    last_speech_timestamp: f64,
) -> Result<()> {
    if segments.is_empty() {
        return Ok(());
    }

    let text_tokens_per_segment: Vec<Vec<u32>> = segments
        .iter()
        .map(|s| {
            s.tokens
                .iter()
                .copied()
                .filter(|&t| t < tokenizer.eot)
                .collect()
        })
        .collect();
    let text_tokens: Vec<u32> = text_tokens_per_segment.iter().flatten().copied().collect();

    let mut alignment = find_alignment(model, tokenizer, &text_tokens, mel, num_frames, 7, 1.0)?;
    let mut word_durations: Vec<f64> = alignment
        .iter()
        .map(|t| t.end - t.start)
        .filter(|d| *d != 0.0)
        .collect();
    word_durations.sort_by(f64::total_cmp);
    let median_duration = if word_durations.is_empty() {
        0.0
    } else {
        let n = word_durations.len();
        if n % 2 == 1 {
            word_durations[n / 2]
        } else {
            (word_durations[n / 2 - 1] + word_durations[n / 2]) / 2.0
        }
    };
    let median_duration = median_duration.min(0.7);
    let max_duration = median_duration * 2.0;

    // hack: truncate long words at sentence boundaries (timing.py:307-317)
    if !word_durations.is_empty() {
        for i in 1..alignment.len() {
            if alignment[i].end - alignment[i].start > max_duration {
                if SENTENCE_END_MARKS.contains(&alignment[i].word.as_str()) {
                    alignment[i].end = alignment[i].start + max_duration;
                } else if SENTENCE_END_MARKS.contains(&alignment[i - 1].word.as_str()) {
                    alignment[i].start = alignment[i].end - max_duration;
                }
            }
        }
    }

    merge_punctuations(&mut alignment, prepend_punctuations, append_punctuations);

    let time_offset = segments[0].seek as f64 * HOP_LENGTH as f64 / SAMPLE_RATE as f64;
    let mut word_index = 0usize;
    let mut last_speech_timestamp = last_speech_timestamp;

    for (segment, seg_text_tokens) in segments.iter_mut().zip(&text_tokens_per_segment) {
        let mut saved_tokens = 0usize;
        let mut words: Vec<Word> = Vec::new();

        while word_index < alignment.len() && saved_tokens < seg_text_tokens.len() {
            let timing = &alignment[word_index];
            if !timing.word.is_empty() {
                words.push(Word {
                    word: timing.word.clone(),
                    start: round2(time_offset + timing.start),
                    end: round2(time_offset + timing.end),
                    probability: timing.probability,
                });
            }
            saved_tokens += timing.tokens.len();
            word_index += 1;
        }

        if !words.is_empty() {
            // ensure words after a pause aren't absurdly long (timing.py:346-362)
            if words[0].end - last_speech_timestamp > median_duration * 4.0
                && (words[0].end - words[0].start > max_duration
                    || (words.len() > 1 && words[1].end - words[0].start > max_duration * 2.0))
            {
                if words.len() > 1 && words[1].end - words[1].start > max_duration {
                    let boundary = (words[1].end / 2.0).max(words[1].end - max_duration);
                    words[0].end = boundary;
                    words[1].start = boundary;
                }
                words[0].start = (words[0].end - max_duration).max(0.0);
            }

            // prefer segment-level start/end when the edge word is too long
            if segment.start < words[0].end && segment.start - 0.5 > words[0].start {
                words[0].start = (words[0].end - median_duration).min(segment.start).max(0.0);
            } else {
                segment.start = words[0].start;
            }
            let last = words.len() - 1;
            if segment.end > words[last].start && segment.end + 0.5 < words[last].end {
                words[last].end = (words[last].start + median_duration).max(segment.end);
            } else {
                segment.end = words[last].end;
            }

            last_speech_timestamp = segment.end;
        }
        segment.words = Some(words);
    }
    Ok(())
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}
