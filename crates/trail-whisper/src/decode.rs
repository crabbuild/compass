//! Token decoding for 30-second windows. Port of `whisper/decoding.py`
//! (greedy + temperature sampling; beam search lands in Phase 3).

use crate::model::WhisperModel;
use crate::tokenizer::{Task, Tokenizer};
use crate::utils::compression_ratio;
use anyhow::{Result, bail};
use candle_core::{Device, IndexOp, Tensor};
use rand::Rng;
use std::collections::HashMap;

pub const CHUNK_LENGTH: usize = 30;

#[derive(Debug, Clone)]
pub struct DecodingOptions {
    pub task: Task,
    /// None triggers per-window language detection.
    pub language: Option<String>,
    pub temperature: f64,
    /// Max tokens to sample; defaults to n_text_ctx / 2.
    pub sample_len: Option<usize>,
    /// Independent samples when temperature > 0.
    pub best_of: Option<usize>,
    /// Beam search width when temperature == 0 (not yet implemented).
    pub beam_size: Option<usize>,
    pub patience: Option<f64>,
    /// Google-NMT style length penalty alpha; None = simple length norm.
    pub length_penalty: Option<f64>,
    /// Tokens of previous-window context, injected after <|startofprev|>.
    pub prompt: Vec<u32>,
    /// Tokens forced at the start of the sample.
    pub prefix: Option<String>,
    /// Extra token ids to suppress; `default_suppress` adds the non-speech set.
    pub suppress_tokens: Vec<u32>,
    pub default_suppress: bool,
    pub suppress_blank: bool,
    pub without_timestamps: bool,
    pub max_initial_timestamp: Option<f64>,
}

impl Default for DecodingOptions {
    fn default() -> Self {
        Self {
            task: Task::Transcribe,
            language: None,
            temperature: 0.0,
            sample_len: None,
            best_of: None,
            beam_size: None,
            patience: None,
            length_penalty: None,
            prompt: Vec::new(),
            prefix: None,
            suppress_tokens: Vec::new(),
            default_suppress: true,
            suppress_blank: true,
            without_timestamps: false,
            max_initial_timestamp: Some(1.0),
        }
    }
}

#[derive(Debug, Clone)]
pub struct DecodingResult {
    pub language: String,
    pub tokens: Vec<u32>,
    pub text: String,
    pub avg_logprob: f64,
    pub no_speech_prob: f64,
    pub temperature: f64,
    pub compression_ratio: f64,
}

/// Detect the spoken language from encoded audio features.
/// Returns (language_code, probabilities over all languages).
pub fn detect_language(
    model: &mut WhisperModel,
    tokenizer: &Tokenizer,
    audio_features: &Tensor,
) -> Result<(String, HashMap<String, f32>)> {
    let device = model.device.clone();
    let n_audio = audio_features.dim(0)?;
    let sot = tokenizer.sot;
    let x = Tensor::from_vec(vec![sot; n_audio], (n_audio, 1), &device)?;
    let hidden = model.decoder_forward(&x, audio_features, true)?;
    let logits = model.logits_at(&hidden, 0)?; // (n_audio, vocab)
    let row: Vec<f32> = logits.i(0)?.to_vec1()?;

    let lang_tokens = tokenizer.all_language_tokens();
    let lang_codes = tokenizer.all_language_codes();

    // softmax restricted to language tokens (everything else masked to -inf)
    let mut max = f32::NEG_INFINITY;
    for &t in &lang_tokens {
        max = max.max(row[t as usize]);
    }
    let mut sum = 0f32;
    let mut probs: Vec<f32> = lang_tokens
        .iter()
        .map(|&t| {
            let p = (row[t as usize] - max).exp();
            sum += p;
            p
        })
        .collect();
    for p in probs.iter_mut() {
        *p /= sum;
    }

    let best = probs
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1))
        .map(|(i, _)| i)
        .unwrap_or(0);
    let map: HashMap<String, f32> = lang_codes
        .iter()
        .zip(&probs)
        .map(|(c, p)| (c.to_string(), *p))
        .collect();
    Ok((lang_codes[best].to_string(), map))
}

/// Logit filters, applied in-place per batch row. Port of decoding.py:423-505.
enum LogitFilter {
    SuppressBlank {
        tokens: Vec<u32>,
        sample_begin: usize,
    },
    SuppressTokens {
        tokens: Vec<u32>,
    },
    TimestampRules(TimestampRules),
}

pub struct TimestampRules {
    pub no_timestamps: u32,
    pub timestamp_begin: u32,
    pub eot: u32,
    pub sample_begin: usize,
    pub max_initial_timestamp_index: Option<usize>,
}

impl LogitFilter {
    fn apply(&self, logits: &mut [f32], ctx: &[u32]) {
        match self {
            Self::SuppressBlank {
                tokens,
                sample_begin,
            } => {
                if ctx.len() == *sample_begin {
                    for &t in tokens {
                        logits[t as usize] = f32::NEG_INFINITY;
                    }
                }
            }
            Self::SuppressTokens { tokens } => {
                for &t in tokens {
                    logits[t as usize] = f32::NEG_INFINITY;
                }
            }
            Self::TimestampRules(r) => apply_timestamp_rules(logits, ctx, r),
        }
    }
}

/// Exposed for fixture tests (timestamp_rules_goldens.npz).
pub fn apply_timestamp_rules(logits: &mut [f32], ctx: &[u32], r: &TimestampRules) {
    let ts = r.timestamp_begin as usize;
    // suppress <|notimestamps|>
    logits[r.no_timestamps as usize] = f32::NEG_INFINITY;

    let seq = &ctx[r.sample_begin.min(ctx.len())..];
    let last_was_timestamp = !seq.is_empty() && seq[seq.len() - 1] >= r.timestamp_begin;
    let penultimate_was_timestamp = seq.len() < 2 || seq[seq.len() - 2] >= r.timestamp_begin;

    if last_was_timestamp {
        if penultimate_was_timestamp {
            // has to be non-timestamp
            for v in logits[ts..].iter_mut() {
                *v = f32::NEG_INFINITY;
            }
        } else {
            // cannot be normal text tokens
            for v in logits[..r.eot as usize].iter_mut() {
                *v = f32::NEG_INFINITY;
            }
        }
    }

    // timestamps shouldn't decrease; also force nonzero-length segments
    if let Some(&last_ts) = seq.iter().rev().find(|&&t| t >= r.timestamp_begin) {
        let timestamp_last = if last_was_timestamp && !penultimate_was_timestamp {
            last_ts as usize
        } else {
            last_ts as usize + 1
        };
        for v in logits[ts..timestamp_last].iter_mut() {
            *v = f32::NEG_INFINITY;
        }
    }

    if ctx.len() == r.sample_begin {
        // suppress generating non-timestamp tokens at the beginning
        for v in logits[..ts].iter_mut() {
            *v = f32::NEG_INFINITY;
        }
        if let Some(max_idx) = r.max_initial_timestamp_index {
            let last_allowed = ts + max_idx;
            if last_allowed + 1 < logits.len() {
                for v in logits[last_allowed + 1..].iter_mut() {
                    *v = f32::NEG_INFINITY;
                }
            }
        }
    }

    // if the probability mass on timestamps exceeds any single text token,
    // sample a timestamp
    let logprobs = log_softmax(logits);
    let timestamp_logprob = logsumexp(&logprobs[ts..]);
    let max_text = logprobs[..ts]
        .iter()
        .copied()
        .fold(f32::NEG_INFINITY, f32::max);
    if timestamp_logprob > max_text {
        for v in logits[..ts].iter_mut() {
            *v = f32::NEG_INFINITY;
        }
    }
}

pub fn log_softmax(logits: &[f32]) -> Vec<f32> {
    let max = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let sum: f32 = logits.iter().map(|&x| (x - max).exp()).sum();
    let log_sum = sum.ln();
    logits.iter().map(|&x| x - max - log_sum).collect()
}

fn logsumexp(logprobs: &[f32]) -> f32 {
    let max = logprobs.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    if max == f32::NEG_INFINITY {
        return f32::NEG_INFINITY;
    }
    let sum: f32 = logprobs.iter().map(|&x| (x - max).exp()).sum();
    max + sum.ln()
}

enum DecoderKind {
    Greedy {
        temperature: f64,
    },
    /// Port of `decoding.py::BeamSearchDecoder` (beam ignores temperature).
    Beam {
        beam_size: usize,
        max_candidates: usize,
    },
}

pub struct DecodingTask<'a> {
    model: &'a mut WhisperModel,
    tokenizer: &'a Tokenizer,
    options: DecodingOptions,
    kind: DecoderKind,
    n_group: usize,
    n_ctx: usize,
    sample_len: usize,
    sot_index: usize,
    sample_begin: usize,
    initial_tokens: Vec<u32>,
    filters: Vec<LogitFilter>,
}

impl<'a> DecodingTask<'a> {
    pub fn new(
        model: &'a mut WhisperModel,
        tokenizer: &'a Tokenizer,
        options: DecodingOptions,
    ) -> Result<Self> {
        if options.beam_size.is_some() && options.best_of.is_some() {
            bail!("beam_size and best_of can't be given together");
        }
        if options.temperature == 0.0 && options.best_of.is_some() {
            bail!("best_of with greedy sampling (temperature=0) is not compatible");
        }
        if options.patience.is_some() && options.beam_size.is_none() {
            bail!("patience requires beam_size to be given");
        }
        if let Some(lp) = options.length_penalty
            && !(0.0..=1.0).contains(&lp)
        {
            bail!("length_penalty (alpha) should be between 0 and 1");
        }

        let kind = match options.beam_size {
            Some(beam_size) => {
                let patience = options.patience.unwrap_or(1.0);
                let max_candidates = (beam_size as f64 * patience).round() as usize;
                if max_candidates == 0 {
                    bail!("invalid beam size ({beam_size}) or patience ({patience})");
                }
                DecoderKind::Beam {
                    beam_size,
                    max_candidates,
                }
            }
            None => DecoderKind::Greedy {
                temperature: options.temperature,
            },
        };
        let n_group = options.beam_size.or(options.best_of).unwrap_or(1);
        let n_ctx = model.n_text_ctx();
        let sample_len = options.sample_len.unwrap_or(n_ctx / 2);

        let sot_sequence = if options.without_timestamps {
            tokenizer.sot_sequence_including_notimestamps()
        } else {
            tokenizer.sot_sequence.clone()
        };

        // _get_initial_tokens
        let mut tokens = sot_sequence.clone();
        if let Some(prefix) = &options.prefix {
            let mut prefix_tokens = tokenizer.encode(&format!(" {}", prefix.trim()));
            let max_prefix_len = n_ctx / 2 - sample_len.min(n_ctx / 2);
            if prefix_tokens.len() > max_prefix_len {
                prefix_tokens = prefix_tokens[prefix_tokens.len() - max_prefix_len..].to_vec();
            }
            tokens.extend(prefix_tokens);
        }
        if !options.prompt.is_empty() {
            let max_prompt = n_ctx / 2 - 1;
            let tail = if options.prompt.len() > max_prompt {
                &options.prompt[options.prompt.len() - max_prompt..]
            } else {
                &options.prompt[..]
            };
            let mut with_prev = vec![tokenizer.sot_prev];
            with_prev.extend_from_slice(tail);
            with_prev.extend(tokens);
            tokens = with_prev;
        }
        let initial_tokens = tokens;
        let sample_begin = initial_tokens.len();
        let sot_index = initial_tokens
            .iter()
            .position(|&t| t == tokenizer.sot)
            .ok_or_else(|| anyhow::anyhow!("start-of-transcript token missing from prompt"))?;

        let mut filters = Vec::new();
        if options.suppress_blank {
            let mut blank = tokenizer.encode(" ");
            blank.push(tokenizer.eot);
            filters.push(LogitFilter::SuppressBlank {
                tokens: blank,
                sample_begin,
            });
        }
        {
            // _get_suppress_tokens
            let mut suppress = options.suppress_tokens.clone();
            if options.default_suppress {
                suppress.extend(tokenizer.non_speech_tokens());
            }
            suppress.extend([
                tokenizer.transcribe,
                tokenizer.translate,
                tokenizer.sot,
                tokenizer.sot_prev,
                tokenizer.sot_lm,
                tokenizer.no_speech,
            ]);
            suppress.sort_unstable();
            suppress.dedup();
            filters.push(LogitFilter::SuppressTokens { tokens: suppress });
        }
        if !options.without_timestamps {
            let precision = CHUNK_LENGTH as f64 / model.n_audio_ctx() as f64; // 0.02s
            let max_initial_timestamp_index = options
                .max_initial_timestamp
                .map(|m| (m / precision).round() as usize);
            filters.push(LogitFilter::TimestampRules(TimestampRules {
                no_timestamps: tokenizer.no_timestamps,
                timestamp_begin: tokenizer.timestamp_begin,
                eot: tokenizer.eot,
                sample_begin,
                max_initial_timestamp_index,
            }));
        }

        Ok(Self {
            model,
            tokenizer,
            options,
            kind,
            n_group,
            n_ctx,
            sample_len,
            sot_index,
            sample_begin,
            initial_tokens,
            filters,
        })
    }

    /// Decode one 30-second mel window (1, n_mels, N_FRAMES).
    pub fn run(&mut self, mel: &Tensor) -> Result<DecodingResult> {
        let device: Device = self.model.device.clone();
        let audio_features = self.model.encoder_forward(mel, true)?;

        // language detection (per-window) when not specified
        let mut initial_tokens = self.initial_tokens.clone();
        let language = match &self.options.language {
            Some(l) => l.clone(),
            None => {
                let (lang, _) = detect_language(self.model, self.tokenizer, &audio_features)?;
                let lang_token = self.tokenizer.to_language_token(&lang)?;
                initial_tokens[self.sot_index + 1] = lang_token;
                lang
            }
        };

        let n = self.n_group;
        let features = if n > 1 {
            audio_features.repeat((n, 1, 1))?
        } else {
            audio_features
        };

        let mut rows: Vec<Vec<u32>> = vec![initial_tokens.clone(); n];
        let mut sum_logprobs = vec![0f64; n];
        let mut no_speech_prob = f64::NAN;
        let eot = self.tokenizer.eot;
        let mut rng = rand::thread_rng();
        // beam search: finished sequences in python-dict insertion order
        let mut finished: Vec<(Vec<u32>, f64)> = Vec::new();

        for i in 0..self.sample_len {
            // incremental decoding: full prompt on the first pass (flushing the
            // KV cache), then only the last sampled token per step
            let (tokens_t, step_len) = if i == 0 {
                let seq_len = rows[0].len();
                let flat: Vec<u32> = rows.iter().flatten().copied().collect();
                (Tensor::from_vec(flat, (n, seq_len), &device)?, seq_len)
            } else {
                let last: Vec<u32> = rows
                    .iter()
                    .map(|row| {
                        row.last()
                            .copied()
                            .ok_or_else(|| anyhow::anyhow!("decoder produced an empty token row"))
                    })
                    .collect::<Result<_>>()?;
                (Tensor::from_vec(last, (n, 1), &device)?, 1)
            };
            let hidden = self.model.decoder_forward(&tokens_t, &features, i == 0)?;

            if i == 0 {
                // probability of <|nospeech|> at the sot position
                let logits_sot = self.model.logits_at(&hidden, self.sot_index)?;
                let row: Vec<f32> = logits_sot.i(0)?.to_vec1()?;
                let probs = softmax(&row);
                no_speech_prob = probs[self.tokenizer.no_speech as usize] as f64;
            }

            let logits_last = self.model.logits_at(&hidden, step_len - 1)?;
            let logits_rows: Vec<Vec<f32>> = logits_last.to_vec2()?;

            let mut filtered: Vec<Vec<f32>> = logits_rows;
            for (g, logits) in filtered.iter_mut().enumerate() {
                for filter in &self.filters {
                    filter.apply(logits, &rows[g]);
                }
            }

            let completed = match self.kind {
                DecoderKind::Greedy { temperature } => {
                    let mut all_eot = true;
                    for (g, logits) in filtered.iter().enumerate() {
                        let Some(&last) = rows[g].last() else {
                            bail!("decoder produced an empty token row");
                        };
                        let next = if last == eot {
                            eot
                        } else {
                            let chosen = if temperature == 0.0 {
                                argmax(logits)
                            } else {
                                sample_gumbel(logits, temperature, &mut rng)
                            };
                            let logprobs = log_softmax(logits);
                            sum_logprobs[g] += logprobs[chosen as usize] as f64;
                            chosen
                        };
                        rows[g].push(next);
                        if next != eot {
                            all_eot = false;
                        }
                    }
                    all_eot
                }
                DecoderKind::Beam {
                    beam_size,
                    max_candidates,
                } => beam_update(
                    self.model,
                    &mut rows,
                    &mut sum_logprobs,
                    &mut finished,
                    &filtered,
                    beam_size,
                    max_candidates,
                    eot,
                )?,
            };

            if completed || rows[0].len() > self.n_ctx {
                break;
            }
        }

        // final candidates: (full token sequence, cumulative logprob)
        let candidates: Vec<(Vec<u32>, f64)> = match self.kind {
            DecoderKind::Greedy { .. } => rows.into_iter().zip(sum_logprobs).collect(),
            DecoderKind::Beam { beam_size, .. } => {
                let mut cands = finished;
                if cands.len() < beam_size {
                    // not enough finished sequences: pad with the best
                    // unfinished rows, eot-terminated (decoding.py::finalize)
                    let mut order: Vec<usize> = (0..rows.len()).collect();
                    order.sort_by(|&a, &b| sum_logprobs[b].total_cmp(&sum_logprobs[a]));
                    for j in order {
                        if cands.len() >= beam_size {
                            break;
                        }
                        let mut seq = rows[j].clone();
                        seq.push(eot);
                        cands.push((seq, sum_logprobs[j]));
                    }
                }
                cands
            }
        };

        // slice sample_begin..first eot
        let sliced: Vec<Vec<u32>> = candidates
            .iter()
            .map(|(row, _)| {
                let end = row[self.sample_begin..]
                    .iter()
                    .position(|&t| t == eot)
                    .map(|p| self.sample_begin + p)
                    .unwrap_or(row.len());
                row[self.sample_begin..end].to_vec()
            })
            .collect();

        // MaximumLikelihoodRanker
        let selected = (0..candidates.len())
            .max_by(|&a, &b| {
                let score = |g: usize| {
                    let length = sliced[g].len() as f64;
                    let penalty = match self.options.length_penalty {
                        None => length,
                        Some(alpha) => ((5.0 + length) / 6.0).powf(alpha),
                    };
                    candidates[g].1 / penalty
                };
                score(a).total_cmp(&score(b))
            })
            .unwrap_or(0);

        let tokens = sliced[selected].clone();
        let text = self.tokenizer.decode(&tokens).trim().to_string();
        let avg_logprob = candidates[selected].1 / (tokens.len() as f64 + 1.0);

        Ok(DecodingResult {
            language,
            compression_ratio: compression_ratio(&text),
            text,
            tokens,
            avg_logprob,
            no_speech_prob,
            temperature: self.options.temperature,
        })
    }
}

/// One beam-search step. Port of `decoding.py::BeamSearchDecoder.update`:
/// expand each beam with its top (beam_size + 1) continuations, dedup by
/// sequence (dict semantics: first insertion position, last value), keep the
/// top beam_size unfinished, bank eot-terminated ones, and reorder the KV
/// cache to the surviving beams' source rows. Returns true when
/// `max_candidates` sequences have finished.
#[allow(clippy::too_many_arguments)]
fn beam_update(
    model: &mut WhisperModel,
    rows: &mut Vec<Vec<u32>>,
    sum_logprobs: &mut Vec<f64>,
    finished: &mut Vec<(Vec<u32>, f64)>,
    filtered_logits: &[Vec<f32>],
    beam_size: usize,
    max_candidates: usize,
    eot: u32,
) -> Result<bool> {
    // candidate pool in python-dict insertion order
    let mut order: Vec<Vec<u32>> = Vec::with_capacity(rows.len() * (beam_size + 1));
    let mut pool: HashMap<Vec<u32>, (f64, usize)> = HashMap::new();
    for (j, logits) in filtered_logits.iter().enumerate() {
        let logprobs = log_softmax(logits);
        for (token, lp) in topk(&logprobs, beam_size + 1) {
            let mut seq = rows[j].clone();
            seq.push(token);
            let score = sum_logprobs[j] + lp as f64;
            match pool.get_mut(&seq) {
                Some(entry) => *entry = (score, j), // keep position, take last value
                None => {
                    order.push(seq.clone());
                    pool.insert(seq, (score, j));
                }
            }
        }
    }
    // stable sort by score descending preserves dict order on ties
    order.sort_by(|a, b| pool[b].0.total_cmp(&pool[a].0));

    let mut new_rows: Vec<Vec<u32>> = Vec::with_capacity(beam_size);
    let mut new_lps: Vec<f64> = Vec::with_capacity(beam_size);
    let mut sources: Vec<usize> = Vec::with_capacity(beam_size);
    let mut newly_finished: Vec<(Vec<u32>, f64)> = Vec::new();
    for seq in order {
        let (score, src) = pool[&seq];
        if seq.last().copied() == Some(eot) {
            newly_finished.push((seq, score));
        } else {
            new_rows.push(seq);
            new_lps.push(score);
            sources.push(src);
            if new_rows.len() == beam_size {
                break; // python breaks the whole scan here
            }
        }
    }

    if new_rows.is_empty() {
        bail!("beam search: no unfinished candidates to continue");
    }
    model.rearrange_kv_cache(&sources)?;
    *rows = new_rows;
    *sum_logprobs = new_lps;

    for (seq, score) in newly_finished {
        if finished.len() >= max_candidates {
            break;
        }
        finished.push((seq, score));
    }
    Ok(finished.len() >= max_candidates)
}

/// Top-k (value, index) selection, descending, ties by lower index first
/// (matching torch.topk ordering).
fn topk(values: &[f32], k: usize) -> Vec<(u32, f32)> {
    let mut top: Vec<(u32, f32)> = Vec::with_capacity(k + 1);
    for (i, &v) in values.iter().enumerate() {
        if top.len() < k || v > top[top.len() - 1].1 {
            let pos = top.iter().position(|&(_, tv)| v > tv).unwrap_or(top.len());
            top.insert(pos, (i as u32, v));
            if top.len() > k {
                top.pop();
            }
        }
    }
    top
}

fn softmax(logits: &[f32]) -> Vec<f32> {
    let max = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let exps: Vec<f32> = logits.iter().map(|&x| (x - max).exp()).collect();
    let sum: f32 = exps.iter().sum();
    exps.into_iter().map(|e| e / sum).collect()
}

fn argmax(logits: &[f32]) -> u32 {
    logits
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Less))
        .map(|(i, _)| i as u32)
        .unwrap_or(0)
}

/// Categorical sampling via the Gumbel-max trick (distributionally equivalent
/// to torch.distributions.Categorical on logits/T; not RNG-compatible).
fn sample_gumbel(logits: &[f32], temperature: f64, rng: &mut impl Rng) -> u32 {
    let mut best = 0usize;
    let mut best_v = f32::NEG_INFINITY;
    for (i, &l) in logits.iter().enumerate() {
        if l == f32::NEG_INFINITY {
            continue;
        }
        let u: f32 = rng.gen_range(1e-9..1.0);
        let g = -(-u.ln()).ln();
        let v = l / temperature as f32 + g;
        if v > best_v {
            best_v = v;
            best = i;
        }
    }
    best as u32
}

/// Convenience one-shot decode of a single mel window.
pub fn decode(
    model: &mut WhisperModel,
    tokenizer: &Tokenizer,
    mel: &Tensor,
    options: DecodingOptions,
) -> Result<DecodingResult> {
    DecodingTask::new(model, tokenizer, options)?.run(mel)
}

#[cfg(test)]
mod tests {
    use super::*;
    use candle_core::{DType, Device};
    use candle_transformers::models::whisper::Config;

    fn tiny_config(multilingual: bool) -> Config {
        Config {
            num_mel_bins: 2,
            max_source_positions: 4,
            d_model: 4,
            encoder_attention_heads: 2,
            encoder_layers: 1,
            vocab_size: if multilingual { 51_865 } else { 51_864 },
            max_target_positions: 16,
            decoder_attention_heads: 2,
            decoder_layers: 1,
            suppress_tokens: Vec::new(),
        }
    }

    fn english_tokenizer() -> Result<Tokenizer> {
        crate::tokenizer::get_tokenizer(false, 99, Some("en"), Some(Task::Transcribe))
    }

    #[test]
    fn decoding_option_validation_rejects_incompatible_search_modes() -> Result<()> {
        let tokenizer = english_tokenizer()?;
        let mut model = WhisperModel::for_test(tiny_config(false))?;

        let both = DecodingOptions {
            beam_size: Some(2),
            best_of: Some(2),
            ..DecodingOptions::default()
        };
        assert!(DecodingTask::new(&mut model, &tokenizer, both).is_err());

        let greedy_best_of = DecodingOptions {
            best_of: Some(2),
            ..DecodingOptions::default()
        };
        assert!(DecodingTask::new(&mut model, &tokenizer, greedy_best_of).is_err());

        let patience_without_beam = DecodingOptions {
            patience: Some(1.5),
            ..DecodingOptions::default()
        };
        assert!(DecodingTask::new(&mut model, &tokenizer, patience_without_beam).is_err());

        let bad_length_penalty = DecodingOptions {
            length_penalty: Some(1.1),
            ..DecodingOptions::default()
        };
        assert!(DecodingTask::new(&mut model, &tokenizer, bad_length_penalty).is_err());

        let zero_candidates = DecodingOptions {
            beam_size: Some(2),
            patience: Some(0.0),
            ..DecodingOptions::default()
        };
        assert!(DecodingTask::new(&mut model, &tokenizer, zero_candidates).is_err());
        Ok(())
    }

    #[test]
    fn tiny_model_runs_greedy_sampling_and_beam_decoders() -> Result<()> {
        let device = Device::Cpu;
        let tokenizer = english_tokenizer()?;
        let mel = Tensor::zeros((1, 2, 8), DType::F32, &device)?;

        let mut greedy_model = WhisperModel::for_test(tiny_config(false))?;
        let greedy = decode(
            &mut greedy_model,
            &tokenizer,
            &mel,
            DecodingOptions {
                language: Some("en".to_owned()),
                sample_len: Some(2),
                without_timestamps: true,
                suppress_blank: false,
                default_suppress: false,
                ..DecodingOptions::default()
            },
        )?;
        assert_eq!(greedy.language, "en");
        assert_eq!(greedy.temperature, 0.0);

        let mut sampling_model = WhisperModel::for_test(tiny_config(false))?;
        let sampled = decode(
            &mut sampling_model,
            &tokenizer,
            &mel,
            DecodingOptions {
                language: Some("en".to_owned()),
                temperature: 0.5,
                best_of: Some(2),
                sample_len: Some(1),
                without_timestamps: true,
                suppress_blank: false,
                default_suppress: false,
                ..DecodingOptions::default()
            },
        )?;
        assert_eq!(sampled.language, "en");
        assert_eq!(sampled.temperature, 0.5);

        let mut beam_model = WhisperModel::for_test(tiny_config(false))?;
        let beam = decode(
            &mut beam_model,
            &tokenizer,
            &mel,
            DecodingOptions {
                language: Some("en".to_owned()),
                beam_size: Some(2),
                sample_len: Some(2),
                without_timestamps: true,
                suppress_blank: false,
                default_suppress: false,
                ..DecodingOptions::default()
            },
        )?;
        assert_eq!(beam.language, "en");
        Ok(())
    }

    #[test]
    fn language_detection_scores_every_supported_language() -> Result<()> {
        let device = Device::Cpu;
        let mut model = WhisperModel::for_test(tiny_config(true))?;
        let tokenizer = crate::tokenizer::get_tokenizer(
            true,
            model.num_languages(),
            Some("en"),
            Some(Task::Transcribe),
        )?;
        let mel = Tensor::zeros((1, 2, 8), DType::F32, &device)?;
        let features = model.encoder_forward(&mel, true)?;
        let (language, probabilities) = detect_language(&mut model, &tokenizer, &features)?;
        assert_eq!(probabilities.len(), model.num_languages());
        assert!(probabilities.contains_key(&language));
        Ok(())
    }

    #[test]
    fn timestamp_filters_and_numeric_helpers_cover_boundary_rules() {
        let rules = TimestampRules {
            no_timestamps: 2,
            timestamp_begin: 6,
            eot: 1,
            sample_begin: 2,
            max_initial_timestamp_index: Some(1),
        };

        let mut initial = vec![0.0; 10];
        apply_timestamp_rules(&mut initial, &[3, 4], &rules);
        assert!(initial[..6].iter().all(|value| value.is_infinite()));
        assert!(initial[8..].iter().all(|value| value.is_infinite()));

        let mut after_pair = vec![0.0; 10];
        apply_timestamp_rules(&mut after_pair, &[3, 4, 6, 7], &rules);
        assert!(after_pair[6..].iter().all(|value| value.is_infinite()));

        let mut after_text = vec![0.0; 10];
        apply_timestamp_rules(&mut after_text, &[3, 4, 5, 7], &rules);
        assert!(after_text[..1].iter().all(|value| value.is_infinite()));

        let normalized = log_softmax(&[0.0, 0.0]);
        assert!((normalized[0].exp() - 0.5).abs() < 1e-6);
        assert_eq!(topk(&[1.0, 3.0, 2.0], 2), vec![(1, 3.0), (2, 2.0)]);
        assert_eq!(argmax(&[1.0, 4.0, 2.0]), 1);
        assert_eq!(argmax(&[]), 0);

        let mut rng = rand::thread_rng();
        assert_eq!(sample_gumbel(&[f32::NEG_INFINITY, 0.0], 0.5, &mut rng), 1);
    }
}
