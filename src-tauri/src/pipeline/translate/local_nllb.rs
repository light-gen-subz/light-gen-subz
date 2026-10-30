use std::borrow::Cow;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::{Context, Result};
use ort::session::Session;
use ort::session::SessionInputValue;
use ort::value::{DynValue, Tensor, TensorRef};
use tokenizers::Tokenizer;

use super::languages::flores_code_for;
use super::TranslationEngine;
use crate::cancel;
use crate::errors::coded;

const EOS_TOKEN_ID: i64 = 2;
const PAD_TOKEN_ID: i64 = 1;
const MAX_NEW_TOKENS: usize = 200;
/// Layers, heads and head size of NLLB-200-distilled-600M: the shape of one KV tensor.
const LAYERS: usize = 12;
const HEADS: i64 = 16;
const HEAD_DIM: i64 = 64;
/// Lines translated together: a whole batch costs about one decoder step per token.
const BATCH_SIZE: usize = 8;

/// Local translation via an NLLB-200 model exported to ONNX: an encoder and a "merged"
/// decoder that runs with a key/value cache, so each step only feeds the newest token.
pub struct LocalNllbEngine {
    encoder: Mutex<Session>,
    decoder: Mutex<Session>,
    tokenizer: Tokenizer,
}

/// Source and target FLORES codes. NLLB has no automatic detection: the source language
/// must be known (it comes from the transcription).
///
/// Extracted from `translate` so it can be checked without loading ~900 MB of ONNX weights.
pub fn resolve_flores_pair(
    source_lang: Option<&str>,
    target_lang: &str,
) -> Result<(&'static str, &'static str)> {
    let tgt = flores_code_for(target_lang).ok_or_else(|| {
        coded(
            "unsupported_language",
            format!("unsupported target language: {target_lang}"),
        )
    })?;
    let src = match source_lang {
        None => {
            return Err(coded(
                "source_lang_required",
                "local translation requires a known source language",
            ))
        }
        Some(code) => flores_code_for(code).ok_or_else(|| {
            coded(
                "unsupported_language",
                format!("unsupported source language: {code}"),
            )
        })?,
    };
    Ok((src, tgt))
}

/// Encoder input sequence: `[source language] tokens… [eos]`.
///
/// The tokenizer's post-processor pins a source language, so the text is tokenised without
/// special tokens and these are added here.
pub fn encoder_input_ids(src_lang_id: i64, token_ids: &[u32]) -> Vec<i64> {
    let mut ids = Vec::with_capacity(token_ids.len() + 2);
    ids.push(src_lang_id);
    ids.extend(token_ids.iter().map(|&id| id as i64));
    ids.push(EOS_TOKEN_ID);
    ids
}

/// Decoder priming: `[eos, target language]`, NLLB's generation convention
/// (`decoder_start_token_id` + `forced_bos_token_id`).
pub fn decoder_priming(tgt_lang_id: i64) -> Vec<i64> {
    vec![EOS_TOKEN_ID, tgt_lang_id]
}

/// Logits of the last decoded position of row `row`, in the flattened
/// `[batch, dec_len, vocab]`.
pub fn last_position_logits(
    logits: &[f32],
    row: usize,
    dec_len: usize,
    vocab_size: usize,
) -> &[f32] {
    let start = (row * dec_len + dec_len - 1) * vocab_size;
    &logits[start..start + vocab_size]
}

/// Greedy decoding: the index of the largest logit.
pub fn argmax(logits: &[f32]) -> Option<i64> {
    logits
        .iter()
        .enumerate()
        .filter(|(_, v)| !v.is_nan())
        .max_by(|a, b| a.1.total_cmp(b.1))
        .map(|(i, _)| i as i64)
}

/// Upper bound on the tokens generated for an input of `input_tokens`: a translation
/// is rarely more than a few times longer than its source, and a runaway decoder must
/// not spend 200 steps on a three-word line.
pub fn max_new_tokens(input_tokens: usize) -> usize {
    (input_tokens * 3 + 10).min(MAX_NEW_TOKENS)
}

/// `true` when the generated tokens end in a pattern of 1 to 4 tokens repeated five
/// times in a row — the greedy decoder is stuck in a loop.
pub fn is_looping(generated: &[i64]) -> bool {
    loop_period(generated).is_some()
}

fn loop_period(generated: &[i64]) -> Option<usize> {
    (1..=4).find(|&period| {
        let window = period * 5;
        generated.len() >= window && {
            let tail = &generated[generated.len() - window..];
            tail.chunks(period).all(|chunk| chunk == &tail[..period])
        }
    })
}

/// Drops the runaway repetitions at the end, keeping the phrase once.
pub fn trim_loop(generated: &mut Vec<i64>) {
    if let Some(period) = loop_period(generated) {
        generated.truncate(generated.len() - period * 4);
    }
}

/// Pads the encoder inputs of a batch to the longest row. Returns the flat ids, the
/// attention mask (1 = real token, 0 = padding) and the padded length.
pub fn pad_batch(rows: &[Vec<i64>]) -> (Vec<i64>, Vec<i64>, usize) {
    let len = rows.iter().map(Vec::len).max().unwrap_or(0);
    let mut ids = Vec::with_capacity(rows.len() * len);
    let mut mask = Vec::with_capacity(rows.len() * len);
    for row in rows {
        ids.extend(row);
        mask.extend(std::iter::repeat_n(1, row.len()));
        let padding = len - row.len();
        ids.extend(std::iter::repeat_n(PAD_TOKEN_ID, padding));
        mask.extend(std::iter::repeat_n(0, padding));
    }
    (ids, mask, len)
}

/// Order in which to translate `lengths`: shortest first, so rows of a batch have about
/// the same size and padding is small. Returns indices into the original order.
pub fn length_order(lengths: &[usize]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..lengths.len()).collect();
    order.sort_by_key(|&i| lengths[i]);
    order
}

fn kv_name(prefix: &str, layer: usize, kind: &str) -> String {
    format!("{prefix}.{layer}.{kind}")
}

/// Cache tensors in a fixed order: for each layer, decoder key/value then encoder key/value.
fn kv_kinds() -> [&'static str; 4] {
    [
        "decoder.key",
        "decoder.value",
        "encoder.key",
        "encoder.value",
    ]
}

/// The ONNX Runtime environment, created once for the whole process.
fn environment() -> Result<ort::environment::Environment> {
    static ENV: OnceLock<ort::environment::Environment> = OnceLock::new();
    if let Some(env) = ENV.get() {
        return Ok(env.clone());
    }
    let env = ort::init()
        .with_name("light-gen-subz")
        .build()
        .context("initialising ONNX Runtime")?;
    Ok(ENV.get_or_init(|| env).clone())
}

type Cached = Option<(std::path::PathBuf, Arc<LocalNllbEngine>)>;

fn cache() -> &'static Mutex<Cached> {
    static CACHE: OnceLock<Mutex<Cached>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

/// Frees the translation model kept in memory (~900 MB of weights).
pub fn release_cache() {
    *cache().lock().unwrap_or_else(|p| p.into_inner()) = None;
}

impl LocalNllbEngine {
    pub fn load(encoder_path: &Path, decoder_path: &Path, tokenizer_path: &Path) -> Result<Self> {
        let env = environment()?;
        let encoder = Session::builder(&env)
            .context("creating encoder session builder")?
            .commit_from_file(encoder_path)
            .context("loading encoder model")?;
        let decoder = Session::builder(&env)
            .context("creating decoder session builder")?
            .commit_from_file(decoder_path)
            .context("loading decoder model")?;
        let tokenizer = Tokenizer::from_file(tokenizer_path)
            .map_err(|e| anyhow::anyhow!("loading tokenizer: {e}"))?;
        Ok(Self {
            encoder: Mutex::new(encoder),
            decoder: Mutex::new(decoder),
            tokenizer,
        })
    }

    /// The loaded engine for these files, kept for the next translation: the ~900 MB of
    /// weights take seconds to load. [`release_cache`] frees them.
    pub fn cached(files: &super::nllb_models::NllbFiles) -> Result<Arc<Self>> {
        let mut cache = cache().lock().unwrap_or_else(|p| p.into_inner());
        if let Some((path, engine)) = cache.as_ref() {
            if *path == files.encoder {
                return Ok(Arc::clone(engine));
            }
        }
        let engine = Arc::new(Self::load(
            &files.encoder,
            &files.decoder,
            &files.tokenizer,
        )?);
        *cache = Some((files.encoder.clone(), Arc::clone(&engine)));
        Ok(engine)
    }

    fn lang_ids(&self, src_flores: &str, tgt_flores: &str) -> Result<(i64, i64)> {
        let id = |code: &str, what: &str| {
            self.tokenizer
                .token_to_id(code)
                .map(i64::from)
                .with_context(|| format!("unknown {what} language code: {code}"))
        };
        Ok((id(src_flores, "source")?, id(tgt_flores, "target")?))
    }

    /// Tokenizes without special tokens: the tokenizer's post-processor bakes in a fixed
    /// source language, so the source/eos tokens are added by `encoder_input_ids`.
    fn tokenize(&self, text: &str) -> Result<Vec<u32>> {
        Ok(self
            .tokenizer
            .encode(text, false)
            .map_err(|e| anyhow::anyhow!("tokenizing text: {e}"))?
            .get_ids()
            .to_vec())
    }

    /// Translates a batch of already tokenized lines with a greedy decoder and a
    /// key/value cache: after the first step only the newest token goes through the
    /// decoder, instead of the whole prefix again.
    fn translate_tokens(
        &self,
        batch: &[Vec<u32>],
        src_lang_id: i64,
        tgt_lang_id: i64,
    ) -> Result<Vec<Vec<u32>>> {
        let rows: Vec<Vec<i64>> = batch
            .iter()
            .map(|ids| encoder_input_ids(src_lang_id, ids))
            .collect();
        let (input_ids, mask, enc_len) = pad_batch(&rows);
        let n = batch.len() as i64;
        let enc_len = enc_len as i64;

        let (hidden_shape, hidden) = {
            let mut encoder = self.encoder.lock().unwrap_or_else(|p| p.into_inner());
            let outputs = encoder.run(ort::inputs![
                "input_ids" => Tensor::from_array((vec![n, enc_len], input_ids))?,
                "attention_mask" => TensorRef::from_array_view((vec![n, enc_len], &mask[..]))?,
            ])?;
            let (shape, data) = outputs["last_hidden_state"].try_extract_tensor::<f32>()?;
            (shape.iter().copied().collect::<Vec<i64>>(), data.to_vec())
        };

        let mut generated: Vec<Vec<i64>> = vec![Vec::new(); batch.len()];
        let mut finished = vec![false; batch.len()];
        let budget = batch
            .iter()
            .map(|ids| max_new_tokens(ids.len()))
            .max()
            .unwrap_or(0);

        // Placeholder cache for the first step: with `use_cache_branch = false` the model
        // ignores it, but ort cannot build a tensor with a zero-sized dimension. After that
        // the model's own output tensors are fed back as they are, without any copy.
        let mut cache: Vec<DynValue> = (0..LAYERS * 4)
            .map(|_| {
                Tensor::from_array((
                    vec![n, HEADS, 1, HEAD_DIM],
                    vec![0.0f32; (n * HEADS * HEAD_DIM) as usize],
                ))
                .map(Tensor::into_dyn)
            })
            .collect::<std::result::Result<_, _>>()?;

        let mut decoder = self.decoder.lock().unwrap_or_else(|p| p.into_inner());
        let mut step_ids: Vec<i64> = (0..batch.len())
            .flat_map(|_| decoder_priming(tgt_lang_id))
            .collect();
        let mut step_len = 2i64;

        for step in 0..=budget {
            cancel::check()?;
            let use_cache = step > 0;

            let mut inputs: Vec<(Cow<'_, str>, SessionInputValue<'_>)> = vec![
                (
                    "encoder_attention_mask".into(),
                    TensorRef::from_array_view((vec![n, enc_len], &mask[..]))?.into(),
                ),
                (
                    "input_ids".into(),
                    TensorRef::from_array_view((vec![n, step_len], &step_ids[..]))?.into(),
                ),
                (
                    "encoder_hidden_states".into(),
                    TensorRef::from_array_view((hidden_shape.clone(), &hidden[..]))?.into(),
                ),
                (
                    "use_cache_branch".into(),
                    Tensor::from_array((vec![1i64], vec![use_cache]))?.into(),
                ),
            ];
            for (i, kv) in cache.iter().enumerate() {
                let name = kv_name("past_key_values", i / 4, kv_kinds()[i % 4]);
                inputs.push((name.into(), kv.into()));
            }

            let mut outputs = decoder.run(inputs)?;
            let (logits_shape, logits) = outputs["logits"].try_extract_tensor::<f32>()?;
            let vocab = logits_shape[2] as usize;
            let dec_len = logits_shape[1] as usize;

            let mut next = vec![PAD_TOKEN_ID; batch.len()];
            for row in 0..batch.len() {
                if finished[row] {
                    continue;
                }
                let id = argmax(last_position_logits(logits, row, dec_len, vocab))
                    .context("decoder produced no logits")?;
                if id == EOS_TOKEN_ID {
                    finished[row] = true;
                } else {
                    generated[row].push(id);
                    if is_looping(&generated[row]) {
                        trim_loop(&mut generated[row]);
                        finished[row] = true;
                    } else {
                        next[row] = id;
                    }
                }
            }

            if finished.iter().all(|f| *f) || step == budget {
                break;
            }

            // The decoder part of the cache grows by one position; the encoder part
            // (cross-attention) is computed once, on the first step, and kept.
            for (i, kv) in cache.iter_mut().enumerate() {
                let kind = kv_kinds()[i % 4];
                if use_cache && kind.starts_with("encoder") {
                    continue;
                }
                *kv = outputs
                    .remove(kv_name("present", i / 4, kind))
                    .with_context(|| {
                        format!("decoder output present.{}.{kind} is missing", i / 4)
                    })?;
            }
            drop(outputs);
            step_ids = next;
            step_len = 1;
        }

        Ok(generated
            .into_iter()
            .map(|row| generated_tokens_from(&row))
            .collect())
    }

    fn decode(&self, tokens: &[u32]) -> Result<String> {
        self.tokenizer
            .decode(tokens, true)
            .map(|s| s.trim().to_string())
            .map_err(|e| anyhow::anyhow!("decoding generated tokens: {e}"))
    }
}

/// Generated token ids as the tokenizer's `u32`.
fn generated_tokens_from(ids: &[i64]) -> Vec<u32> {
    ids.iter().map(|&id| id as u32).collect()
}

impl TranslationEngine for LocalNllbEngine {
    /// `source_lang` must be a known language code — unlike the cloud engine, NLLB has no
    /// auto-detection; callers should pass the language already detected during transcription.
    fn translate(
        &self,
        texts: &[String],
        source_lang: Option<&str>,
        target_lang: &str,
        mut on_progress: Box<dyn FnMut(f32) + Send>,
    ) -> Result<Vec<String>> {
        let (src_flores, tgt_flores) = resolve_flores_pair(source_lang, target_lang)?;

        let (src_lang_id, tgt_lang_id) = self.lang_ids(src_flores, tgt_flores)?;
        let tokens: Vec<Vec<u32>> = texts
            .iter()
            .map(|text| self.tokenize(text))
            .collect::<Result<_>>()?;

        // Shortest lines first, so the rows of a batch need little padding.
        let lengths: Vec<usize> = tokens.iter().map(Vec::len).collect();
        let order = length_order(&lengths);

        let mut results = vec![String::new(); texts.len()];
        let mut done = 0;
        on_progress(0.0);
        for group in order.chunks(BATCH_SIZE) {
            cancel::check()?;
            let batch: Vec<Vec<u32>> = group.iter().map(|&i| tokens[i].clone()).collect();
            let translated = self.translate_tokens(&batch, src_lang_id, tgt_lang_id)?;
            for (&index, ids) in group.iter().zip(translated) {
                results[index] = self.decode(&ids)?;
            }
            done += group.len();
            on_progress(done as f32 / texts.len().max(1) as f32);
        }
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── language code resolution ─────────────────────────────────────────────

    #[test]
    fn the_cached_engine_can_be_released() {
        release_cache();

        assert!(cache().lock().unwrap().is_none());
    }

    #[test]
    fn a_known_pair_resolves_to_flores_codes() {
        let (src, tgt) = resolve_flores_pair(Some("en"), "fr").unwrap();

        assert_eq!(src, flores_code_for("en").unwrap());
        assert_eq!(tgt, flores_code_for("fr").unwrap());
    }

    #[test]
    fn a_missing_source_language_is_refused() {
        let err = resolve_flores_pair(None, "fr").unwrap_err();

        assert!(
            err.to_string().contains("requires a known source language"),
            "{err}"
        );
    }

    #[test]
    fn an_unsupported_source_language_is_refused() {
        let err = resolve_flores_pair(Some("xx"), "fr").unwrap_err();

        assert!(
            err.to_string().contains("unsupported source language"),
            "{err}"
        );
    }

    #[test]
    fn an_unsupported_target_language_is_refused() {
        let err = resolve_flores_pair(Some("en"), "xx").unwrap_err();

        assert!(
            err.to_string().contains("unsupported target language"),
            "{err}"
        );
    }

    #[test]
    fn the_target_is_checked_before_the_source() {
        // Both are wrong: it is the target, chosen by the user, that gets reported.
        let err = resolve_flores_pair(Some("xx"), "yy").unwrap_err();

        assert!(err.to_string().contains("target"), "{err}");
    }

    // ── token sequences ──────────────────────────────────────────────────────

    #[test]
    fn the_encoder_input_is_wrapped_by_the_language_and_eos_tokens() {
        let ids = encoder_input_ids(256_047, &[10, 20, 30]);

        assert_eq!(ids, vec![256_047, 10, 20, 30, EOS_TOKEN_ID]);
    }

    #[test]
    fn an_empty_text_still_carries_its_markers() {
        assert_eq!(encoder_input_ids(256_047, &[]), vec![256_047, EOS_TOKEN_ID]);
    }

    #[test]
    fn the_decoder_starts_on_eos_then_the_target_language() {
        assert_eq!(decoder_priming(256_057), vec![EOS_TOKEN_ID, 256_057]);
    }

    #[test]
    fn generated_ids_become_tokenizer_ids() {
        assert_eq!(generated_tokens_from(&[11, 22, 33]), vec![11u32, 22, 33]);
        assert!(generated_tokens_from(&[]).is_empty());
    }

    // ── greedy decoding ──────────────────────────────────────────────────────

    #[test]
    fn the_argmax_picks_the_highest_logit() {
        assert_eq!(argmax(&[0.1, 0.9, 0.3]), Some(1));
    }

    #[test]
    fn the_argmax_settles_ties_on_the_last_candidate() {
        // `max_by` keeps the last of equal elements. In practice two strictly equal float
        // logits do not occur; the test only pins the behaviour.
        assert_eq!(argmax(&[0.5, 0.5]), Some(1));
    }

    #[test]
    fn each_row_of_a_batch_reads_its_own_last_position() {
        // Two rows, two decoded positions, vocabulary of two.
        let logits = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];

        assert_eq!(last_position_logits(&logits, 0, 2, 2), &[3.0, 4.0]);
        assert_eq!(last_position_logits(&logits, 1, 2, 2), &[7.0, 8.0]);
    }

    #[test]
    fn a_batch_is_padded_to_its_longest_row_with_a_matching_mask() {
        let (ids, mask, len) = pad_batch(&[vec![5, 6, 7], vec![8]]);

        assert_eq!(len, 3);
        assert_eq!(ids, vec![5, 6, 7, 8, PAD_TOKEN_ID, PAD_TOKEN_ID]);
        assert_eq!(mask, vec![1, 1, 1, 1, 0, 0]);
    }

    #[test]
    fn an_empty_batch_has_no_padding() {
        assert_eq!(pad_batch(&[]), (vec![], vec![], 0));
    }

    #[test]
    fn lines_are_ordered_shortest_first_and_every_index_is_kept() {
        let order = length_order(&[9, 2, 5, 2]);

        assert_eq!(order, vec![1, 3, 2, 0]);
    }

    #[test]
    fn a_runaway_loop_is_trimmed_to_one_occurrence() {
        let mut generated = vec![3, 8, 9, 6, 8, 9, 6, 8, 9, 6, 8, 9, 6, 8, 9, 6];

        trim_loop(&mut generated);

        assert_eq!(generated, vec![3, 8, 9, 6]);
    }

    #[test]
    fn text_without_a_loop_is_left_alone() {
        let mut generated = vec![1, 2, 3, 4];

        trim_loop(&mut generated);

        assert_eq!(generated, vec![1, 2, 3, 4]);
    }

    #[test]
    fn the_cache_has_a_decoder_and_an_encoder_pair_per_layer() {
        assert_eq!(kv_kinds().len(), 4);
        assert_eq!(
            kv_name("present", 11, "decoder.key"),
            "present.11.decoder.key"
        );
    }

    #[test]
    fn a_nan_logit_is_never_picked() {
        assert_eq!(argmax(&[0.1, f32::NAN, 0.9]), Some(2));
        assert_eq!(argmax(&[f32::NAN]), None);
    }

    #[test]
    fn the_token_budget_follows_the_input_but_is_capped() {
        assert_eq!(max_new_tokens(3), 19);
        assert_eq!(max_new_tokens(1_000), MAX_NEW_TOKENS);
    }

    #[test]
    fn a_repeated_phrase_is_detected_as_a_loop() {
        assert!(is_looping(&[9, 7, 7, 7, 7, 7]));
        assert!(is_looping(&[1, 4, 5, 4, 5, 4, 5, 4, 5, 4, 5]));
        assert!(is_looping(&[
            3, 8, 9, 6, 8, 9, 6, 8, 9, 6, 8, 9, 6, 8, 9, 6
        ]));
    }

    #[test]
    fn normal_text_is_not_a_loop() {
        assert!(!is_looping(&[]));
        assert!(!is_looping(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]));
        assert!(!is_looping(&[7, 7, 7, 7]));
    }

    #[test]
    fn the_argmax_handles_negative_logits() {
        assert_eq!(argmax(&[-3.0, -1.0, -2.0]), Some(1));
    }

    #[test]
    fn an_empty_logit_slice_yields_nothing() {
        assert_eq!(argmax(&[]), None);
    }

    #[test]
    fn the_last_decoded_position_is_the_one_read() {
        // Two positions, vocabulary of three: the second slice is wanted.
        let logits = [1.0, 2.0, 3.0, 40.0, 50.0, 60.0];

        assert_eq!(last_position_logits(&logits, 0, 2, 3), &[40.0, 50.0, 60.0]);
    }

    #[test]
    fn a_single_decoded_position_reads_the_whole_slice() {
        let logits = [1.0, 2.0, 3.0];

        assert_eq!(last_position_logits(&logits, 0, 1, 3), &logits[..]);
    }

    #[test]
    fn reading_the_last_position_then_taking_its_argmax_gives_the_next_token() {
        let logits = [0.0, 0.0, 9.0, 0.1, 7.0, 0.2];

        let next = argmax(last_position_logits(&logits, 0, 2, 3));

        assert_eq!(next, Some(1)); // 7.0 is the maximum of the second slice
    }

    /// Needs the model files: `LGS_NLLB_DIR` must hold `encoder.onnx`, `decoder.onnx` (the
    /// merged decoder) and `tokenizer.json`, and the data dir is redirected there so that
    /// nothing is downloaded. `cargo test -- --ignored real_model`.
    #[test]
    #[ignore]
    fn real_model_translates_in_batches_with_the_cache() {
        let dir = std::path::PathBuf::from(std::env::var("LGS_NLLB_DIR").expect("LGS_NLLB_DIR"));
        let files = super::super::nllb_models::NllbFiles {
            encoder: dir.join("encoder.onnx"),
            decoder: dir.join("decoder.onnx"),
            tokenizer: dir.join("tokenizer.json"),
        };
        let engine =
            LocalNllbEngine::load(&files.encoder, &files.decoder, &files.tokenizer).unwrap();
        let lines: Vec<String> = [
            "Hello, how are you?",
            "The weather is nice today.",
            "I would like a cup of coffee, please.",
            "Where is the train station?",
            "Yes.",
            "She told me that she would arrive late because of the traffic on the highway.",
            "We have to leave now.",
            "Thank you very much for your help yesterday evening.",
            "No!",
            "What time does the movie start?",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();

        let started = std::time::Instant::now();
        let batched = engine
            .translate(&lines, Some("en"), "fr", Box::new(|_| {}))
            .unwrap();
        let batched_time = started.elapsed();

        // One line at a time gives the same text, bar the odd synonym: the weights are
        // int8 with *dynamic* activation scales, which depend on what else is in the batch.
        let mut single = Vec::new();
        let started = std::time::Instant::now();
        for line in &lines {
            single.push(
                engine
                    .translate(
                        std::slice::from_ref(line),
                        Some("en"),
                        "fr",
                        Box::new(|_| {}),
                    )
                    .unwrap()
                    .remove(0),
            );
        }
        let single_time = started.elapsed();

        let mut differing = 0;
        for (b, (s, l)) in batched.iter().zip(single.iter().zip(&lines)) {
            println!("{l}  ->  {b}");
            if b != s {
                differing += 1;
                println!("   (one by one: {s})");
            }
        }
        assert!(
            differing <= 2,
            "{differing} of {} lines differ",
            lines.len()
        );
        println!("batched {batched_time:?} vs one by one {single_time:?}");
        assert_eq!(batched[0], "Bonjour, comment allez-vous ?");
        assert_eq!(batched[1], "Le temps est beau aujourd'hui.");
        assert!(batched.iter().all(|t| !t.is_empty()));
    }
}
