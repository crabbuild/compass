//! Whisper encoder/decoder, vendored from candle-transformers 0.9.2
//! (models/whisper/model.rs, Apache-2.0/MIT) and extended with:
//!   - incremental decoding: self-attention KV cache append + positional
//!     offset, so each step feeds only the new token (the upstream version
//!     recomputes the full sequence every step)
//!
//! Cross-attention QK capture for word timestamps lands in Phase 4.

use candle_core::{D, Device, IndexOp, Result, Tensor};
use candle_nn::{
    Conv1d, Conv1dConfig, Embedding, LayerNorm, Linear, Module, VarBuilder, embedding,
};
use candle_transformers::models::whisper::Config;

/// Quantized counterparts: QMatMul-backed linears from a GGUF file; layer
/// norms, convs and embeddings are dequantized to f32 at load time.
pub use candle_transformers::quantized_nn::Linear as QLinear;
pub use candle_transformers::quantized_var_builder::VarBuilder as QVarBuilder;

fn qlinear(in_dim: usize, out_dim: usize, vb: QVarBuilder) -> Result<QLinear> {
    candle_transformers::quantized_nn::linear(in_dim, out_dim, vb)
}

fn qlinear_no_bias(in_dim: usize, out_dim: usize, vb: QVarBuilder) -> Result<QLinear> {
    candle_transformers::quantized_nn::linear_no_bias(in_dim, out_dim, vb)
}

fn q_layer_norm(size: usize, vb: QVarBuilder) -> Result<LayerNorm> {
    let weight = vb.get(size, "weight")?.dequantize(vb.device())?;
    let bias = vb.get(size, "bias")?.dequantize(vb.device())?;
    Ok(LayerNorm::new(weight, bias, 1e-5))
}

fn q_conv1d(
    in_channels: usize,
    out_channels: usize,
    kernel_size: usize,
    config: Conv1dConfig,
    vb: QVarBuilder,
) -> Result<Conv1d> {
    let weight = vb
        .get((out_channels, in_channels, kernel_size), "weight")?
        .dequantize(vb.device())?;
    let bias = vb.get(out_channels, "bias")?.dequantize(vb.device())?;
    Ok(Conv1d::new(weight, Some(bias), config))
}

/// Dequantize a GGUF linear into a plain f32 Linear (used for the encoder,
/// where BLAS GEMMs beat qmatmul kernels by an order of magnitude).
fn qlinear_dequantized(in_dim: usize, out_dim: usize, vb: QVarBuilder) -> Result<Linear> {
    let weight = vb
        .get((out_dim, in_dim), "weight")?
        .dequantize(vb.device())?;
    let bias = vb.get(out_dim, "bias")?.dequantize(vb.device())?;
    Ok(Linear::new(weight, Some(bias)))
}

fn qlinear_no_bias_dequantized(in_dim: usize, out_dim: usize, vb: QVarBuilder) -> Result<Linear> {
    let weight = vb
        .get((out_dim, in_dim), "weight")?
        .dequantize(vb.device())?;
    Ok(Linear::new(weight, None))
}

fn linear(in_dim: usize, out_dim: usize, vb: VarBuilder) -> Result<Linear> {
    let weight = vb.get((out_dim, in_dim), "weight")?;
    let bias = vb.get(out_dim, "bias")?;
    Ok(Linear::new(weight, Some(bias)))
}

fn linear_no_bias(in_dim: usize, out_dim: usize, vb: VarBuilder) -> Result<Linear> {
    let weight = vb.get((out_dim, in_dim), "weight")?;
    Ok(Linear::new(weight, None))
}

fn conv1d(
    in_channels: usize,
    out_channels: usize,
    kernel_size: usize,
    config: Conv1dConfig,
    vb: VarBuilder,
) -> Result<Conv1d> {
    let weight = vb.get((out_channels, in_channels, kernel_size), "weight")?;
    let bias = vb.get(out_channels, "bias")?;
    Ok(Conv1d::new(weight, Some(bias), config))
}

fn layer_norm(size: usize, vb: VarBuilder) -> Result<LayerNorm> {
    let weight = vb.get(size, "weight")?;
    let bias = vb.get(size, "bias")?;
    Ok(LayerNorm::new(weight, bias, 1e-5))
}

#[derive(Debug, Clone)]
struct MultiHeadAttention<L: Module> {
    query: L,
    key: L,
    value: L,
    out: L,
    n_head: usize,
    kv_cache: Option<(Tensor, Tensor)>,
}

impl MultiHeadAttention<Linear> {
    fn load(n_state: usize, n_head: usize, vb: VarBuilder) -> Result<Self> {
        let query = linear(n_state, n_state, vb.pp("q_proj"))?;
        let value = linear(n_state, n_state, vb.pp("v_proj"))?;
        let key = linear_no_bias(n_state, n_state, vb.pp("k_proj"))?;
        let out = linear(n_state, n_state, vb.pp("out_proj"))?;
        Ok(Self {
            query,
            key,
            value,
            out,
            n_head,
            kv_cache: None,
        })
    }
}

impl MultiHeadAttention<QLinear> {
    fn load_gguf(n_state: usize, n_head: usize, vb: QVarBuilder) -> Result<Self> {
        let query = qlinear(n_state, n_state, vb.pp("q_proj"))?;
        let value = qlinear(n_state, n_state, vb.pp("v_proj"))?;
        let key = qlinear_no_bias(n_state, n_state, vb.pp("k_proj"))?;
        let out = qlinear(n_state, n_state, vb.pp("out_proj"))?;
        Ok(Self {
            query,
            key,
            value,
            out,
            n_head,
            kv_cache: None,
        })
    }
}

impl MultiHeadAttention<Linear> {
    fn load_gguf_dequantized(n_state: usize, n_head: usize, vb: QVarBuilder) -> Result<Self> {
        let query = qlinear_dequantized(n_state, n_state, vb.pp("q_proj"))?;
        let value = qlinear_dequantized(n_state, n_state, vb.pp("v_proj"))?;
        let key = qlinear_no_bias_dequantized(n_state, n_state, vb.pp("k_proj"))?;
        let out = qlinear_dequantized(n_state, n_state, vb.pp("out_proj"))?;
        Ok(Self {
            query,
            key,
            value,
            out,
            n_head,
            kv_cache: None,
        })
    }
}

impl<L: Module> MultiHeadAttention<L> {
    /// Self-attention (xa = None): with `use_cache`, new keys/values are
    /// appended to the cache and only the suffix is treated as queries.
    /// Cross-attention (xa = Some): keys/values computed once per flush.
    ///
    /// KV caches are kept in pre-scaled head layout (batch, heads, len,
    /// head_dim) so the single-token decode path never re-lays-out the
    /// (potentially 1500-frame) cross-attention keys.
    fn forward(
        &mut self,
        x: &Tensor,
        xa: Option<&Tensor>,
        mask: Option<&Tensor>,
        flush_cache: bool,
        use_cache: bool,
        want_qk: bool,
    ) -> Result<(Tensor, Option<Tensor>)> {
        let n_state = x.dim(2)?;
        let scale = ((n_state / self.n_head) as f64).powf(-0.25);
        let q = (self.reshape_head(&self.query.forward(x)?)? * scale)?;
        let (k, v) = match xa {
            None => {
                let k_new = (self.reshape_head(&self.key.forward(x)?)? * scale)?;
                let v_new = self.reshape_head(&self.value.forward(x)?)?;
                if !use_cache {
                    (k_new, v_new)
                } else {
                    if flush_cache {
                        self.kv_cache = None;
                    }
                    let (k, v) = match &self.kv_cache {
                        Some((pk, pv)) => (
                            Tensor::cat(&[pk, &k_new], 2)?,
                            Tensor::cat(&[pv, &v_new], 2)?,
                        ),
                        None => (k_new.contiguous()?, v_new.contiguous()?),
                    };
                    self.kv_cache = Some((k.clone(), v.clone()));
                    (k, v)
                }
            }
            Some(xa) => {
                if flush_cache {
                    self.kv_cache = None;
                }
                if let Some((k, v)) = &self.kv_cache {
                    (k.clone(), v.clone())
                } else {
                    let k = (self.reshape_head(&self.key.forward(xa)?)? * scale)?.contiguous()?;
                    let v = self.reshape_head(&self.value.forward(xa)?)?.contiguous()?;
                    self.kv_cache = Some((k.clone(), v.clone()));
                    (k, v)
                }
            }
        };
        let (wv, qk) = self.attend(&q, &k, &v, mask, want_qk)?;
        let out = self.out.forward(&wv)?;
        Ok((out, qk))
    }

    fn reshape_head(&self, x: &Tensor) -> Result<Tensor> {
        let (n_batch, n_ctx, n_state) = x.dims3()?;
        let target_dims = &[n_batch, n_ctx, self.n_head, n_state / self.n_head];
        x.reshape(target_dims)?.transpose(1, 2)
    }

    /// Attention over head-layout tensors: q (b,h,q_len,hd) pre-scaled,
    /// k (b,h,k_len,hd) pre-scaled, v (b,h,k_len,hd). Returns (weighted
    /// values flattened to (b,q_len,d), optional pre-softmax QK — the matrix
    /// `timing.py` reads via forward hooks for word-level alignment).
    fn attend(
        &self,
        q: &Tensor,
        k: &Tensor,
        v: &Tensor,
        mask: Option<&Tensor>,
        want_qk: bool,
    ) -> Result<(Tensor, Option<Tensor>)> {
        let q_ctx = q.dim(2)?;
        let k_ctx = k.dim(2)?;
        let kt = k.transpose(2, 3)?;
        // contiguous copies pay off on multi-token (prefill/encoder) passes,
        // but dominate the single-token decode step; skip them there
        let (q, kt, v) = if q_ctx > 1 {
            (q.contiguous()?, kt.contiguous()?, v.contiguous()?)
        } else {
            (q.clone(), kt, v.clone())
        };
        let mut qk = q.matmul(&kt)?;
        if let Some(mask) = mask {
            // Only needed when several queries attend causally to each other,
            // i.e. the first pass where q_ctx == k_ctx. A single incremental
            // query attends to everything (mirrors the reference, where
            // mask[:1, :1] is an all-zero add).
            if q_ctx == k_ctx {
                let mask = mask.i((0..q_ctx, 0..k_ctx))?;
                qk = qk.broadcast_add(&mask)?;
            }
        }
        let captured = if want_qk { Some(qk.clone()) } else { None };
        let w = candle_nn::ops::softmax_last_dim(&qk)?;
        let wv = w.matmul(&v)?.transpose(1, 2)?.flatten_from(2)?;
        Ok((wv, captured))
    }

    fn reset_kv_cache(&mut self) {
        self.kv_cache = None;
    }

    fn cache_len(&self) -> usize {
        // head-layout cache: (batch, heads, len, head_dim)
        self.kv_cache
            .as_ref()
            .and_then(|(k, _)| k.dims().get(2).copied())
            .unwrap_or(0)
    }
}

#[derive(Debug, Clone)]
struct ResidualAttentionBlock<L: Module> {
    attn: MultiHeadAttention<L>,
    attn_ln: LayerNorm,
    cross_attn: Option<(MultiHeadAttention<L>, LayerNorm)>,
    mlp_linear1: L,
    mlp_linear2: L,
    mlp_ln: LayerNorm,
}

impl ResidualAttentionBlock<Linear> {
    fn load(n_state: usize, n_head: usize, ca: bool, vb: VarBuilder) -> Result<Self> {
        let attn = MultiHeadAttention::load(n_state, n_head, vb.pp("self_attn"))?;
        let attn_ln = layer_norm(n_state, vb.pp("self_attn_layer_norm"))?;
        let cross_attn = if ca {
            let cross_attn = MultiHeadAttention::load(n_state, n_head, vb.pp("encoder_attn"))?;
            let cross_attn_ln = layer_norm(n_state, vb.pp("encoder_attn_layer_norm"))?;
            Some((cross_attn, cross_attn_ln))
        } else {
            None
        };
        let n_mlp = n_state * 4;
        let mlp_linear1 = linear(n_state, n_mlp, vb.pp("fc1"))?;
        let mlp_linear2 = linear(n_mlp, n_state, vb.pp("fc2"))?;
        let mlp_ln = layer_norm(n_state, vb.pp("final_layer_norm"))?;
        Ok(Self {
            attn,
            attn_ln,
            cross_attn,
            mlp_linear1,
            mlp_linear2,
            mlp_ln,
        })
    }
}

impl ResidualAttentionBlock<Linear> {
    fn load_gguf_dequantized(
        n_state: usize,
        n_head: usize,
        ca: bool,
        vb: QVarBuilder,
    ) -> Result<Self> {
        let attn = MultiHeadAttention::load_gguf_dequantized(n_state, n_head, vb.pp("self_attn"))?;
        let attn_ln = q_layer_norm(n_state, vb.pp("self_attn_layer_norm"))?;
        let cross_attn = if ca {
            let cross_attn =
                MultiHeadAttention::load_gguf_dequantized(n_state, n_head, vb.pp("encoder_attn"))?;
            let cross_attn_ln = q_layer_norm(n_state, vb.pp("encoder_attn_layer_norm"))?;
            Some((cross_attn, cross_attn_ln))
        } else {
            None
        };
        let n_mlp = n_state * 4;
        let mlp_linear1 = qlinear_dequantized(n_state, n_mlp, vb.pp("fc1"))?;
        let mlp_linear2 = qlinear_dequantized(n_mlp, n_state, vb.pp("fc2"))?;
        let mlp_ln = q_layer_norm(n_state, vb.pp("final_layer_norm"))?;
        Ok(Self {
            attn,
            attn_ln,
            cross_attn,
            mlp_linear1,
            mlp_linear2,
            mlp_ln,
        })
    }
}

impl ResidualAttentionBlock<QLinear> {
    fn load_gguf(n_state: usize, n_head: usize, ca: bool, vb: QVarBuilder) -> Result<Self> {
        let attn = MultiHeadAttention::load_gguf(n_state, n_head, vb.pp("self_attn"))?;
        let attn_ln = q_layer_norm(n_state, vb.pp("self_attn_layer_norm"))?;
        let cross_attn = if ca {
            let cross_attn = MultiHeadAttention::load_gguf(n_state, n_head, vb.pp("encoder_attn"))?;
            let cross_attn_ln = q_layer_norm(n_state, vb.pp("encoder_attn_layer_norm"))?;
            Some((cross_attn, cross_attn_ln))
        } else {
            None
        };
        let n_mlp = n_state * 4;
        let mlp_linear1 = qlinear(n_state, n_mlp, vb.pp("fc1"))?;
        let mlp_linear2 = qlinear(n_mlp, n_state, vb.pp("fc2"))?;
        let mlp_ln = q_layer_norm(n_state, vb.pp("final_layer_norm"))?;
        Ok(Self {
            attn,
            attn_ln,
            cross_attn,
            mlp_linear1,
            mlp_linear2,
            mlp_ln,
        })
    }
}

impl<L: Module> ResidualAttentionBlock<L> {
    /// Returns (output, optional cross-attention QK when `capture_cross_qk`).
    fn forward(
        &mut self,
        x: &Tensor,
        xa: Option<&Tensor>,
        mask: Option<&Tensor>,
        flush_kv_cache: bool,
        use_self_cache: bool,
        capture_cross_qk: bool,
    ) -> Result<(Tensor, Option<Tensor>)> {
        let (attn, _) = self.attn.forward(
            &self.attn_ln.forward(x)?,
            None,
            mask,
            flush_kv_cache,
            use_self_cache,
            false,
        )?;
        let mut x = (x + attn)?;
        let mut cross_qk = None;
        if let Some((attn, ln)) = &mut self.cross_attn {
            let (out, qk) = attn.forward(
                &ln.forward(&x)?,
                xa,
                None,
                flush_kv_cache,
                true,
                capture_cross_qk,
            )?;
            x = (&x + out)?;
            cross_qk = qk;
        }
        let mlp = self.mlp_linear2.forward(
            &self
                .mlp_linear1
                .forward(&self.mlp_ln.forward(&x)?)?
                .gelu()?,
        )?;
        Ok(((x + mlp)?, cross_qk))
    }

    fn reset_kv_cache(&mut self) {
        self.attn.reset_kv_cache();
        if let Some((attn, _)) = &mut self.cross_attn {
            attn.reset_kv_cache();
        }
    }
}

fn sinusoids(length: usize, channels: usize, device: &Device) -> Result<Tensor> {
    let max_timescale = 10000f32;
    let log_timescale_increment = max_timescale.ln() / (channels / 2 - 1) as f32;
    let inv_timescales: Vec<_> = (0..channels / 2)
        .map(|i| (i as f32 * (-log_timescale_increment)).exp())
        .collect();
    let inv_timescales = Tensor::new(inv_timescales.as_slice(), device)?.unsqueeze(0)?;
    let arange = Tensor::arange(0, length as u32, device)?
        .to_dtype(candle_core::DType::F32)?
        .unsqueeze(1)?;
    let sh = (length, channels / 2);
    let scaled_time = (arange.broadcast_as(sh)? * inv_timescales.broadcast_as(sh)?)?;
    let sincos = Tensor::cat(&[scaled_time.sin()?, scaled_time.cos()?], 1)?;
    Ok(sincos)
}

#[derive(Debug, Clone)]
pub struct AudioEncoder<L: Module> {
    conv1: Conv1d,
    conv2: Conv1d,
    positional_embedding: Tensor,
    blocks: Vec<ResidualAttentionBlock<L>>,
    ln_post: LayerNorm,
}

impl AudioEncoder<Linear> {
    fn load(vb: VarBuilder, cfg: &Config) -> Result<Self> {
        let n_state = cfg.d_model;
        let n_head = cfg.encoder_attention_heads;
        let n_ctx = cfg.max_source_positions;
        let cfg1 = Conv1dConfig {
            padding: 1,
            stride: 1,
            groups: 1,
            dilation: 1,
            cudnn_fwd_algo: None,
        };
        let cfg2 = Conv1dConfig {
            padding: 1,
            stride: 2,
            groups: 1,
            dilation: 1,
            cudnn_fwd_algo: None,
        };
        let conv1 = conv1d(cfg.num_mel_bins, n_state, 3, cfg1, vb.pp("conv1"))?;
        let conv2 = conv1d(n_state, n_state, 3, cfg2, vb.pp("conv2"))?;
        let positional_embedding = sinusoids(n_ctx, n_state, vb.device())?;
        let blocks = (0..cfg.encoder_layers)
            .map(|i| {
                ResidualAttentionBlock::load(n_state, n_head, false, vb.pp(format!("layers.{i}")))
            })
            .collect::<Result<Vec<_>>>()?;
        let ln_post = layer_norm(n_state, vb.pp("layer_norm"))?;
        Ok(Self {
            conv1,
            conv2,
            positional_embedding,
            blocks,
            ln_post,
        })
    }
}

impl AudioEncoder<Linear> {
    fn load_gguf_dequantized(vb: QVarBuilder, cfg: &Config) -> Result<Self> {
        let n_state = cfg.d_model;
        let n_head = cfg.encoder_attention_heads;
        let n_ctx = cfg.max_source_positions;
        let cfg1 = Conv1dConfig {
            padding: 1,
            stride: 1,
            groups: 1,
            dilation: 1,
            cudnn_fwd_algo: None,
        };
        let cfg2 = Conv1dConfig {
            padding: 1,
            stride: 2,
            groups: 1,
            dilation: 1,
            cudnn_fwd_algo: None,
        };
        let conv1 = q_conv1d(cfg.num_mel_bins, n_state, 3, cfg1, vb.pp("conv1"))?;
        let conv2 = q_conv1d(n_state, n_state, 3, cfg2, vb.pp("conv2"))?;
        let positional_embedding = sinusoids(n_ctx, n_state, vb.device())?;
        let blocks = (0..cfg.encoder_layers)
            .map(|i| {
                ResidualAttentionBlock::load_gguf_dequantized(
                    n_state,
                    n_head,
                    false,
                    vb.pp(format!("layers.{i}")),
                )
            })
            .collect::<Result<Vec<_>>>()?;
        let ln_post = q_layer_norm(n_state, vb.pp("layer_norm"))?;
        Ok(Self {
            conv1,
            conv2,
            positional_embedding,
            blocks,
            ln_post,
        })
    }
}

impl<L: Module> AudioEncoder<L> {
    pub fn forward(&mut self, x: &Tensor, flush_kv_cache: bool) -> Result<Tensor> {
        let x = self.conv1.forward(x)?.gelu()?;
        let x = self.conv2.forward(&x)?.gelu()?;
        let x = x.transpose(1, 2)?;
        let (_bsize, seq_len, _hidden) = x.dims3()?;
        let positional_embedding = self.positional_embedding.narrow(0, 0, seq_len)?;
        let mut x = x.broadcast_add(&positional_embedding)?;
        for block in self.blocks.iter_mut() {
            // encoder self-attention is full-sequence; no KV caching
            x = block
                .forward(&x, None, None, flush_kv_cache, false, false)?
                .0;
        }
        let x = self.ln_post.forward(&x)?;
        Ok(x)
    }
}

#[derive(Debug, Clone)]
pub struct TextDecoder<L: Module> {
    token_embedding: Embedding,
    positional_embedding: Tensor,
    blocks: Vec<ResidualAttentionBlock<L>>,
    ln: LayerNorm,
    mask: Tensor,
}

impl TextDecoder<Linear> {
    fn load(vb: VarBuilder, cfg: &Config) -> Result<Self> {
        let n_state = cfg.d_model;
        let n_head = cfg.decoder_attention_heads;
        let n_ctx = cfg.max_target_positions;
        let token_embedding = embedding(cfg.vocab_size, n_state, vb.pp("embed_tokens"))?;
        let positional_embedding = vb.get((n_ctx, n_state), "embed_positions.weight")?;
        let blocks = (0..cfg.decoder_layers)
            .map(|i| {
                ResidualAttentionBlock::load(n_state, n_head, true, vb.pp(format!("layers.{i}")))
            })
            .collect::<Result<Vec<_>>>()?;
        let ln = layer_norm(n_state, vb.pp("layer_norm"))?;
        let mask: Vec<_> = (0..n_ctx)
            .flat_map(|i| (0..n_ctx).map(move |j| if j > i { f32::NEG_INFINITY } else { 0f32 }))
            .collect();
        let mask = Tensor::from_vec(mask, (n_ctx, n_ctx), vb.device())?;
        Ok(Self {
            token_embedding,
            positional_embedding,
            blocks,
            ln,
            mask,
        })
    }
}

impl TextDecoder<QLinear> {
    fn load_gguf(vb: QVarBuilder, cfg: &Config) -> Result<Self> {
        let n_state = cfg.d_model;
        let n_head = cfg.decoder_attention_heads;
        let n_ctx = cfg.max_target_positions;
        let embeddings = vb
            .get((cfg.vocab_size, n_state), "embed_tokens.weight")?
            .dequantize(vb.device())?;
        let token_embedding = Embedding::new(embeddings, n_state);
        let positional_embedding = vb
            .get((n_ctx, n_state), "embed_positions.weight")?
            .dequantize(vb.device())?;
        let blocks = (0..cfg.decoder_layers)
            .map(|i| {
                ResidualAttentionBlock::load_gguf(
                    n_state,
                    n_head,
                    true,
                    vb.pp(format!("layers.{i}")),
                )
            })
            .collect::<Result<Vec<_>>>()?;
        let ln = q_layer_norm(n_state, vb.pp("layer_norm"))?;
        let mask: Vec<_> = (0..n_ctx)
            .flat_map(|i| (0..n_ctx).map(move |j| if j > i { f32::NEG_INFINITY } else { 0f32 }))
            .collect();
        let mask = Tensor::from_vec(mask, (n_ctx, n_ctx), vb.device())?;
        Ok(Self {
            token_embedding,
            positional_embedding,
            blocks,
            ln,
            mask,
        })
    }
}

impl<L: Module> TextDecoder<L> {
    /// Incremental forward: pass the full prompt with `flush = true` once,
    /// then only the newly sampled token(s) with `flush = false`. The
    /// positional embedding is offset by the cached sequence length,
    /// mirroring `whisper/model.py::TextDecoder.forward`.
    pub fn forward(&mut self, x: &Tensor, xa: &Tensor, flush_kv_cache: bool) -> Result<Tensor> {
        let offset = if flush_kv_cache { 0 } else { self.cache_len() };
        let seq_len = x.dim(D::Minus1)?;
        let token_embedding = self.token_embedding.forward(x)?;
        let positional_embedding = self.positional_embedding.narrow(0, offset, seq_len)?;
        let mut x = token_embedding.broadcast_add(&positional_embedding)?;
        for block in self.blocks.iter_mut() {
            x = block
                .forward(&x, Some(xa), Some(&self.mask), flush_kv_cache, true, false)?
                .0;
        }
        self.ln.forward(&x)
    }

    /// Full-sequence forward that also returns each layer's cross-attention
    /// QK matrix ((batch, heads, seq, n_audio_ctx) per layer) — the explicit
    /// replacement for `timing.py`'s forward hooks. Flushes the KV cache.
    pub fn forward_with_cross_qk(
        &mut self,
        x: &Tensor,
        xa: &Tensor,
    ) -> Result<(Tensor, Vec<Tensor>)> {
        let seq_len = x.dim(D::Minus1)?;
        let token_embedding = self.token_embedding.forward(x)?;
        let positional_embedding = self.positional_embedding.narrow(0, 0, seq_len)?;
        let mut x = token_embedding.broadcast_add(&positional_embedding)?;
        let mut cross_qks = Vec::with_capacity(self.blocks.len());
        for block in self.blocks.iter_mut() {
            let (out, qk) = block.forward(&x, Some(xa), Some(&self.mask), true, true, true)?;
            x = out;
            cross_qks.push(qk.ok_or_else(|| {
                candle_core::Error::Msg("decoder block has no cross-attention".to_owned())
            })?);
        }
        Ok((self.ln.forward(&x)?, cross_qks))
    }

    fn cache_len(&self) -> usize {
        self.blocks.first().map(|b| b.attn.cache_len()).unwrap_or(0)
    }

    /// Reorder the self-attention KV caches along the batch dimension so beam
    /// candidates continue from the right prefixes. Mirrors
    /// `decoding.py::PyTorchInference.rearrange_kv_cache` (cross-attention
    /// caches hold identical rows per beam and are left untouched).
    pub fn rearrange_kv_cache(&mut self, source_indices: &[usize]) -> Result<()> {
        if source_indices.iter().enumerate().all(|(i, &s)| i == s) {
            return Ok(());
        }
        let device = self.mask.device().clone();
        let idx: Vec<u32> = source_indices.iter().map(|&i| i as u32).collect();
        let idx = Tensor::from_vec(idx, source_indices.len(), &device)?;
        for block in self.blocks.iter_mut() {
            if let Some((k, v)) = &block.attn.kv_cache {
                let k = k.index_select(&idx, 0)?;
                let v = v.index_select(&idx, 0)?;
                block.attn.kv_cache = Some((k, v));
            }
        }
        Ok(())
    }

    pub fn final_linear(&self, x: &Tensor) -> Result<Tensor> {
        let b_size = x.dim(0)?;
        let w = self.token_embedding.embeddings().broadcast_left(b_size)?;
        let logits = x.matmul(&w.t()?)?;
        Ok(logits)
    }

    pub fn reset_kv_cache(&mut self) {
        for block in self.blocks.iter_mut() {
            block.reset_kv_cache();
        }
    }
}

#[derive(Debug, Clone)]
pub struct Whisper<E: Module, D: Module> {
    pub encoder: AudioEncoder<E>,
    pub decoder: TextDecoder<D>,
    pub config: Config,
}

impl Whisper<Linear, Linear> {
    pub fn load(vb: &VarBuilder, config: Config) -> Result<Self> {
        let encoder = AudioEncoder::load(vb.pp("model.encoder"), &config)?;
        let decoder = TextDecoder::load(vb.pp("model.decoder"), &config)?;
        Ok(Self {
            encoder,
            decoder,
            config,
        })
    }
}

/// Hybrid quantized model: the encoder is dequantized to f32 at load time
/// (its big prefill GEMMs are ~14x faster through BLAS than through qmatmul
/// kernels), while the decoder keeps QMatMul weights — the memory-bound
/// single-token steps are where quantization pays.
impl Whisper<Linear, QLinear> {
    pub fn load_gguf(vb: &QVarBuilder, config: Config) -> Result<Self> {
        let encoder = AudioEncoder::load_gguf_dequantized(vb.pp("model.encoder"), &config)?;
        let decoder = TextDecoder::load_gguf(vb.pp("model.decoder"), &config)?;
        Ok(Self {
            encoder,
            decoder,
            config,
        })
    }
}

impl<E: Module, D: Module> Whisper<E, D> {
    pub fn encoder_forward(&mut self, mel: &Tensor, flush: bool) -> Result<Tensor> {
        self.encoder.forward(mel, flush)
    }

    pub fn decoder_forward(&mut self, tokens: &Tensor, xa: &Tensor, flush: bool) -> Result<Tensor> {
        self.decoder.forward(tokens, xa, flush)
    }

    pub fn decoder_forward_with_cross_qk(
        &mut self,
        tokens: &Tensor,
        xa: &Tensor,
    ) -> Result<(Tensor, Vec<Tensor>)> {
        self.decoder.forward_with_cross_qk(tokens, xa)
    }

    pub fn rearrange_kv_cache(&mut self, source_indices: &[usize]) -> Result<()> {
        self.decoder.rearrange_kv_cache(source_indices)
    }

    pub fn reset_kv_cache(&mut self) {
        self.decoder.reset_kv_cache();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use candle_core::DType;

    fn tiny_config() -> Config {
        Config {
            num_mel_bins: 2,
            max_source_positions: 4,
            d_model: 4,
            encoder_attention_heads: 2,
            encoder_layers: 1,
            vocab_size: 16,
            max_target_positions: 8,
            decoder_attention_heads: 2,
            decoder_layers: 1,
            suppress_tokens: Vec::new(),
        }
    }

    #[test]
    fn tiny_model_exercises_prefill_incremental_and_alignment_paths() -> Result<()> {
        let device = Device::Cpu;
        let config = tiny_config();
        let builder = VarBuilder::zeros(DType::F32, &device);
        let mut model = Whisper::load(&builder, config.clone())?;

        let mel = Tensor::zeros((1, config.num_mel_bins, 8), DType::F32, &device)?;
        let audio = model.encoder_forward(&mel, true)?;
        assert_eq!(
            audio.dims(),
            &[1, config.max_source_positions, config.d_model]
        );

        let prompt = Tensor::from_vec(vec![1u32, 2, 3], (1, 3), &device)?;
        let hidden = model.decoder_forward(&prompt, &audio, true)?;
        assert_eq!(hidden.dims(), &[1, 3, config.d_model]);

        let next = Tensor::from_vec(vec![4u32], (1, 1), &device)?;
        let incremental = model.decoder_forward(&next, &audio, false)?;
        assert_eq!(incremental.dims(), &[1, 1, config.d_model]);
        assert_eq!(model.decoder.cache_len(), 4);

        let logits = model.decoder.final_linear(&incremental)?;
        assert_eq!(logits.dims(), &[1, 1, config.vocab_size]);

        let (aligned, cross_qk) = model.decoder_forward_with_cross_qk(&prompt, &audio)?;
        assert_eq!(aligned.dims(), &[1, 3, config.d_model]);
        assert_eq!(cross_qk.len(), config.decoder_layers);
        assert_eq!(
            cross_qk[0].dims(),
            &[
                1,
                config.decoder_attention_heads,
                3,
                config.max_source_positions,
            ]
        );

        model.rearrange_kv_cache(&[0])?;
        model.reset_kv_cache();
        assert_eq!(model.decoder.cache_len(), 0);
        Ok(())
    }

    #[test]
    fn cache_reordering_tracks_beam_sources() -> Result<()> {
        let device = Device::Cpu;
        let config = tiny_config();
        let builder = VarBuilder::zeros(DType::F32, &device);
        let mut model = Whisper::load(&builder, config.clone())?;

        let mel = Tensor::zeros((2, config.num_mel_bins, 8), DType::F32, &device)?;
        let audio = model.encoder_forward(&mel, true)?;
        let prompt = Tensor::from_vec(vec![1u32, 2, 3, 4], (2, 2), &device)?;
        model.decoder_forward(&prompt, &audio, true)?;
        model.rearrange_kv_cache(&[1, 0])?;
        assert_eq!(model.decoder.cache_len(), 2);
        Ok(())
    }
}
