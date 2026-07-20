//! Whisper tokenizer: tiktoken BPE with programmatically-built special tokens.
//! Port of `whisper/tokenizer.py`.

use anyhow::{Context, Result, anyhow, bail};
use rustc_hash::FxHashMap;
use std::collections::HashMap;
use tiktoken_rs::CoreBPE;

const MULTILINGUAL_TIKTOKEN: &str = include_str!("../assets/multilingual.tiktoken");
const GPT2_TIKTOKEN: &str = include_str!("../assets/gpt2.tiktoken");

/// BPE split pattern used by all Whisper vocabularies (same as GPT-2).
const PAT_STR: &str = r"'s|'t|'re|'ve|'m|'ll|'d| ?\p{L}+| ?\p{N}+| ?[^\s\p{L}\p{N}]+|\s+(?!\S)|\s+";

/// Language codes in vocabulary order. The first `num_languages` entries are
/// assigned token ids `sot + 1 + index`.
pub const LANGUAGES: &[(&str, &str)] = &[
    ("en", "english"),
    ("zh", "chinese"),
    ("de", "german"),
    ("es", "spanish"),
    ("ru", "russian"),
    ("ko", "korean"),
    ("fr", "french"),
    ("ja", "japanese"),
    ("pt", "portuguese"),
    ("tr", "turkish"),
    ("pl", "polish"),
    ("ca", "catalan"),
    ("nl", "dutch"),
    ("ar", "arabic"),
    ("sv", "swedish"),
    ("it", "italian"),
    ("id", "indonesian"),
    ("hi", "hindi"),
    ("fi", "finnish"),
    ("vi", "vietnamese"),
    ("he", "hebrew"),
    ("uk", "ukrainian"),
    ("el", "greek"),
    ("ms", "malay"),
    ("cs", "czech"),
    ("ro", "romanian"),
    ("da", "danish"),
    ("hu", "hungarian"),
    ("ta", "tamil"),
    ("no", "norwegian"),
    ("th", "thai"),
    ("ur", "urdu"),
    ("hr", "croatian"),
    ("bg", "bulgarian"),
    ("lt", "lithuanian"),
    ("la", "latin"),
    ("mi", "maori"),
    ("ml", "malayalam"),
    ("cy", "welsh"),
    ("sk", "slovak"),
    ("te", "telugu"),
    ("fa", "persian"),
    ("lv", "latvian"),
    ("bn", "bengali"),
    ("sr", "serbian"),
    ("az", "azerbaijani"),
    ("sl", "slovenian"),
    ("kn", "kannada"),
    ("et", "estonian"),
    ("mk", "macedonian"),
    ("br", "breton"),
    ("eu", "basque"),
    ("is", "icelandic"),
    ("hy", "armenian"),
    ("ne", "nepali"),
    ("mn", "mongolian"),
    ("bs", "bosnian"),
    ("kk", "kazakh"),
    ("sq", "albanian"),
    ("sw", "swahili"),
    ("gl", "galician"),
    ("mr", "marathi"),
    ("pa", "punjabi"),
    ("si", "sinhala"),
    ("km", "khmer"),
    ("sn", "shona"),
    ("yo", "yoruba"),
    ("so", "somali"),
    ("af", "afrikaans"),
    ("oc", "occitan"),
    ("ka", "georgian"),
    ("be", "belarusian"),
    ("tg", "tajik"),
    ("sd", "sindhi"),
    ("gu", "gujarati"),
    ("am", "amharic"),
    ("yi", "yiddish"),
    ("lo", "lao"),
    ("uz", "uzbek"),
    ("fo", "faroese"),
    ("ht", "haitian creole"),
    ("ps", "pashto"),
    ("tk", "turkmen"),
    ("nn", "nynorsk"),
    ("mt", "maltese"),
    ("sa", "sanskrit"),
    ("lb", "luxembourgish"),
    ("my", "myanmar"),
    ("bo", "tibetan"),
    ("tl", "tagalog"),
    ("mg", "malagasy"),
    ("as", "assamese"),
    ("tt", "tatar"),
    ("haw", "hawaiian"),
    ("ln", "lingala"),
    ("ha", "hausa"),
    ("ba", "bashkir"),
    ("jw", "javanese"),
    ("su", "sundanese"),
    ("yue", "cantonese"),
];

/// Aliases accepted for `--language` in addition to names in [`LANGUAGES`].
pub const LANGUAGE_ALIASES: &[(&str, &str)] = &[
    ("burmese", "my"),
    ("valencian", "ca"),
    ("flemish", "nl"),
    ("haitian", "ht"),
    ("letzeburgesch", "lb"),
    ("pushto", "ps"),
    ("panjabi", "pa"),
    ("moldavian", "ro"),
    ("moldovan", "ro"),
    ("sinhalese", "si"),
    ("castilian", "es"),
    ("mandarin", "zh"),
];

/// Normalize a user-supplied language (code or name) to a code in [`LANGUAGES`].
pub fn normalize_language(language: &str) -> Result<String> {
    let lower = language.to_lowercase();
    if LANGUAGES.iter().any(|(code, _)| *code == lower) {
        return Ok(lower);
    }
    if let Some((_, code)) = LANGUAGES.iter().find(|(_, name)| *name == lower) {
        return Ok(code.to_string());
    }
    if let Some((_, code)) = LANGUAGE_ALIASES.iter().find(|(alias, _)| *alias == lower) {
        return Ok(code.to_string());
    }
    bail!("unsupported language: {language}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Task {
    Transcribe,
    Translate,
}

pub struct Tokenizer {
    bpe: CoreBPE,
    /// id -> bytes for regular tokens; specials rendered from `special_strings`.
    decoder: HashMap<u32, Vec<u8>>,
    special_strings: HashMap<u32, String>,
    special_tokens: HashMap<String, u32>,
    pub num_languages: usize,
    pub language: Option<String>,
    pub task: Option<Task>,
    pub sot_sequence: Vec<u32>,
    n_vocab_base: u32,
    // cached special ids
    pub eot: u32,
    pub sot: u32,
    pub sot_prev: u32,
    pub sot_lm: u32,
    pub no_speech: u32,
    pub no_timestamps: u32,
    pub timestamp_begin: u32,
    pub transcribe: u32,
    pub translate: u32,
}

impl Tokenizer {
    pub fn new(
        multilingual: bool,
        num_languages: usize,
        language: Option<&str>,
        task: Option<Task>,
    ) -> Result<Self> {
        let (vocab_src, language, task) = if multilingual {
            let lang = normalize_language(language.unwrap_or("en"))?;
            (
                MULTILINGUAL_TIKTOKEN,
                Some(lang),
                Some(task.unwrap_or(Task::Transcribe)),
            )
        } else {
            (GPT2_TIKTOKEN, None, None)
        };

        let mut encoder: FxHashMap<Vec<u8>, u32> = FxHashMap::default();
        let mut decoder: HashMap<u32, Vec<u8>> = HashMap::new();
        for line in vocab_src.lines().filter(|l| !l.is_empty()) {
            let (b64, rank) = line
                .split_once(' ')
                .ok_or_else(|| anyhow!("malformed vocab line"))?;
            let bytes = base64_decode(b64)?;
            let rank: u32 = rank.parse()?;
            encoder.insert(bytes.clone(), rank);
            decoder.insert(rank, bytes);
        }
        let n_vocab_base = encoder.len() as u32;

        // Special tokens, in the exact order of tokenizer.py::get_encoding
        let mut specials: Vec<String> =
            vec!["<|endoftext|>".into(), "<|startoftranscript|>".into()];
        for (code, _) in LANGUAGES.iter().take(num_languages) {
            specials.push(format!("<|{code}|>"));
        }
        for s in [
            "<|translate|>",
            "<|transcribe|>",
            "<|startoflm|>",
            "<|startofprev|>",
            "<|nospeech|>",
            "<|notimestamps|>",
        ] {
            specials.push(s.into());
        }
        for i in 0..1501 {
            specials.push(format!("<|{:.2}|>", i as f64 * 0.02));
        }

        let mut special_tokens: HashMap<String, u32> = HashMap::new();
        let mut special_strings: HashMap<u32, String> = HashMap::new();
        let mut special_encoder: FxHashMap<String, u32> = FxHashMap::default();
        for (i, s) in specials.iter().enumerate() {
            let id = n_vocab_base + i as u32;
            special_tokens.insert(s.clone(), id);
            special_strings.insert(id, s.clone());
            special_encoder.insert(s.clone(), id);
        }

        let bpe = CoreBPE::new(encoder, special_encoder, PAT_STR)
            .map_err(|e| anyhow!("failed to build BPE: {e}"))?;

        let get = |name: &str| -> Result<u32> {
            special_tokens
                .get(name)
                .copied()
                .with_context(|| format!("missing special token {name}"))
        };
        let eot = get("<|endoftext|>")?;
        let sot = get("<|startoftranscript|>")?;
        let transcribe = get("<|transcribe|>")?;
        let translate = get("<|translate|>")?;

        let mut sot_sequence = vec![sot];
        if let Some(lang) = &language {
            let index = LANGUAGES
                .iter()
                .take(num_languages)
                .position(|(code, _)| code == lang)
                .with_context(|| format!("language {lang} not in vocabulary"))?;
            sot_sequence.push(sot + 1 + index as u32);
        }
        if let Some(task) = task {
            sot_sequence.push(match task {
                Task::Transcribe => transcribe,
                Task::Translate => translate,
            });
        }

        Ok(Self {
            eot,
            sot,
            sot_prev: get("<|startofprev|>")?,
            sot_lm: get("<|startoflm|>")?,
            no_speech: get("<|nospeech|>")?,
            no_timestamps: get("<|notimestamps|>")?,
            timestamp_begin: get("<|0.00|>")?,
            transcribe,
            translate,
            bpe,
            decoder,
            special_strings,
            special_tokens,
            num_languages,
            language,
            task,
            sot_sequence,
            n_vocab_base,
        })
    }

    pub fn encode(&self, text: &str) -> Vec<u32> {
        self.bpe.encode_ordinary(text)
    }

    /// Decode, skipping timestamp tokens (mirrors `tokenizer.py::decode`,
    /// which drops ids >= timestamp_begin). Invalid UTF-8 becomes U+FFFD,
    /// matching tiktoken's `errors="replace"` default.
    pub fn decode(&self, tokens: &[u32]) -> String {
        self.decode_bytes(tokens.iter().copied().filter(|t| *t < self.timestamp_begin))
    }

    /// Decode including timestamp/special tokens rendered as `<|1.08|>` etc.
    pub fn decode_with_timestamps(&self, tokens: &[u32]) -> String {
        self.decode_bytes(tokens.iter().copied())
    }

    fn decode_bytes(&self, tokens: impl Iterator<Item = u32>) -> String {
        let mut bytes = Vec::new();
        for t in tokens {
            if let Some(b) = self.decoder.get(&t) {
                bytes.extend_from_slice(b);
            } else if let Some(s) = self.special_strings.get(&t) {
                bytes.extend_from_slice(s.as_bytes());
            }
        }
        String::from_utf8_lossy(&bytes).into_owned()
    }

    pub fn special_token(&self, name: &str) -> Option<u32> {
        self.special_tokens.get(name).copied()
    }

    pub fn language_token(&self) -> Result<u32> {
        let lang = self.language.as_deref().context("no language configured")?;
        self.to_language_token(lang)
    }

    pub fn to_language_token(&self, language: &str) -> Result<u32> {
        self.special_token(&format!("<|{language}|>"))
            .with_context(|| format!("language {language} not found in tokenizer"))
    }

    pub fn all_language_tokens(&self) -> Vec<u32> {
        (0..self.num_languages as u32)
            .map(|i| self.sot + 1 + i)
            .collect()
    }

    pub fn all_language_codes(&self) -> Vec<&'static str> {
        LANGUAGES
            .iter()
            .take(self.num_languages)
            .map(|(c, _)| *c)
            .collect()
    }

    pub fn sot_sequence_including_notimestamps(&self) -> Vec<u32> {
        let mut seq = self.sot_sequence.clone();
        seq.push(self.no_timestamps);
        seq
    }

    /// Tokens to suppress to avoid speaker tags / non-speech annotations.
    /// Port of `tokenizer.py::non_speech_tokens`.
    pub fn non_speech_tokens(&self) -> Vec<u32> {
        let mut symbols: Vec<String> = "\"#()*+/:;<=>@[\\]^_`{|}~「」『』"
            .chars()
            .map(|c| c.to_string())
            .collect();
        symbols.extend(
            "<< >> <<< >>> -- --- -( -[ (' (\" (( )) ((( ))) [[ ]] {{ }} ♪♪ ♪♪♪"
                .split(' ')
                .map(str::to_string),
        );
        let miscellaneous: Vec<String> = "♩♪♫♬♭♮♯".chars().map(|c| c.to_string()).collect();

        let mut result: Vec<u32> = vec![self.encode(" -")[0], self.encode(" '")[0]];
        for symbol in symbols.iter().chain(miscellaneous.iter()) {
            let is_misc = miscellaneous.contains(symbol);
            for tokens in [self.encode(symbol), self.encode(&format!(" {symbol}"))] {
                if tokens.len() == 1 || is_misc {
                    result.push(tokens[0]);
                }
            }
        }
        result.sort_unstable();
        result.dedup();
        result
    }

    /// Split tokens into words. Port of `tokenizer.py::split_to_word_tokens`.
    pub fn split_to_word_tokens(&self, tokens: &[u32]) -> (Vec<String>, Vec<Vec<u32>>) {
        if matches!(
            self.language.as_deref(),
            Some("zh") | Some("ja") | Some("th") | Some("lo") | Some("my") | Some("yue")
        ) {
            self.split_tokens_on_unicode(tokens)
        } else {
            self.split_tokens_on_spaces(tokens)
        }
    }

    fn split_tokens_on_unicode(&self, tokens: &[u32]) -> (Vec<String>, Vec<Vec<u32>>) {
        let decoded_full: Vec<char> = self.decode_with_timestamps(tokens).chars().collect();
        const REPLACEMENT: char = '\u{fffd}';

        let mut words = Vec::new();
        let mut word_tokens: Vec<Vec<u32>> = Vec::new();
        let mut current: Vec<u32> = Vec::new();
        let mut unicode_offset = 0usize; // in chars, matching Python str indexing

        for &token in tokens {
            current.push(token);
            let decoded = self.decode_with_timestamps(&current);
            let chars: Vec<char> = decoded.chars().collect();
            let rep_idx = chars.iter().position(|c| *c == REPLACEMENT);
            let complete = match rep_idx {
                None => true,
                Some(i) => decoded_full.get(unicode_offset + i) == Some(&REPLACEMENT),
            };
            if complete {
                unicode_offset += chars.len();
                words.push(decoded);
                word_tokens.push(std::mem::take(&mut current));
            }
        }
        (words, word_tokens)
    }

    fn split_tokens_on_spaces(&self, tokens: &[u32]) -> (Vec<String>, Vec<Vec<u32>>) {
        let (subwords, subword_tokens_list) = self.split_tokens_on_unicode(tokens);
        let mut words: Vec<String> = Vec::new();
        let mut word_tokens: Vec<Vec<u32>> = Vec::new();

        for (subword, subword_tokens) in subwords.into_iter().zip(subword_tokens_list) {
            let special = subword_tokens[0] >= self.eot;
            let with_space = subword.starts_with(' ');
            // Python: `subword.strip() in string.punctuation` — a *substring* check
            // (and vacuously true for the empty string); replicate exactly.
            const PUNCTUATION: &str = "!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~";
            let punctuation = PUNCTUATION.contains(subword.trim());
            if special || with_space || punctuation || words.is_empty() {
                words.push(subword);
                word_tokens.push(subword_tokens);
            } else {
                let last = words.len() - 1;
                words[last].push_str(&subword);
                word_tokens[last].extend(subword_tokens);
            }
        }
        (words, word_tokens)
    }

    pub fn n_vocab(&self) -> u32 {
        self.n_vocab_base + self.special_strings.len() as u32
    }
}

/// Minimal standard-alphabet base64 decoder (vocab files only use it for token bytes).
fn base64_decode(s: &str) -> Result<Vec<u8>> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut table = [255u8; 256];
    for (i, &c) in ALPHABET.iter().enumerate() {
        table[c as usize] = i as u8;
    }
    let bytes: Vec<u8> = s.bytes().filter(|b| *b != b'=').collect();
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    let mut acc: u32 = 0;
    let mut nbits = 0;
    for b in bytes {
        let v = table[b as usize];
        if v == 255 {
            bail!("invalid base64 character {b:#x}");
        }
        acc = (acc << 6) | v as u32;
        nbits += 6;
        if nbits >= 8 {
            nbits -= 8;
            out.push((acc >> nbits) as u8);
        }
    }
    Ok(out)
}

/// Build the tokenizer appropriate for a model, mirroring `get_tokenizer`.
pub fn get_tokenizer(
    multilingual: bool,
    num_languages: usize,
    language: Option<&str>,
    task: Option<Task>,
) -> Result<Tokenizer> {
    Tokenizer::new(multilingual, num_languages, language, task)
}
