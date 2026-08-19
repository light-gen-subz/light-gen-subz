use std::path::Path;
use std::sync::Mutex;

use anyhow::{Context, Result};
use ort::session::Session;
use ort::value::Tensor;
use tokenizers::Tokenizer;

use super::languages::flores_code_for;
use super::TranslationEngine;

const EOS_TOKEN_ID: i64 = 2;
const MAX_NEW_TOKENS: usize = 200;

/// Local translation via an NLLB-200 model exported to ONNX (encoder/decoder, no KV cache —
/// each decoding step recomputes the full decoder pass, which is simple and fast enough for
/// short subtitle lines).
pub struct LocalNllbEngine {
    encoder: Mutex<Session>,
    decoder: Mutex<Session>,
    tokenizer: Tokenizer,
}

/// Codes FLORES source et cible. NLLB n'a pas de détection automatique : la langue
/// source doit être connue (elle vient de la transcription).
///
/// Extrait de `translate` pour être vérifiable sans charger ~900 Mo de poids ONNX.
pub fn resolve_flores_pair(
    source_lang: Option<&str>,
    target_lang: &str,
) -> Result<(&'static str, &'static str)> {
    let tgt = flores_code_for(target_lang)
        .with_context(|| format!("unsupported target language: {target_lang}"))?;
    let src = source_lang
        .context("local translation requires a known source language")
        .and_then(|code| {
            flores_code_for(code).with_context(|| format!("unsupported source language: {code}"))
        })?;
    Ok((src, tgt))
}

/// Séquence d'entrée de l'encodeur : `[langue source] tokens… [eos]`.
///
/// Le post-processeur du tokenizer fige une langue source ; on tokenise donc sans
/// tokens spéciaux et on pose nous-mêmes ceux-ci.
pub fn encoder_input_ids(src_lang_id: i64, token_ids: &[u32]) -> Vec<i64> {
    let mut ids = Vec::with_capacity(token_ids.len() + 2);
    ids.push(src_lang_id);
    ids.extend(token_ids.iter().map(|&id| id as i64));
    ids.push(EOS_TOKEN_ID);
    ids
}

/// Amorce du décodeur : `[eos, langue cible]`, la convention de génération de NLLB
/// (`decoder_start_token_id` + `forced_bos_token_id`).
pub fn decoder_priming(tgt_lang_id: i64) -> Vec<i64> {
    vec![EOS_TOKEN_ID, tgt_lang_id]
}

/// Logits de la dernière position décodée, dans le tenseur aplati `[1, dec_len, vocab]`.
pub fn last_position_logits(logits: &[f32], dec_len: usize, vocab_size: usize) -> &[f32] {
    let start = (dec_len - 1) * vocab_size;
    &logits[start..start + vocab_size]
}

/// Décodage glouton : l'indice du plus grand logit.
pub fn argmax(logits: &[f32]) -> Option<i64> {
    logits
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
        .map(|(i, _)| i as i64)
}

/// Tokens réellement générés : on retire l'amorce `[eos, langue cible]`.
pub fn generated_tokens(decoder_ids: &[i64]) -> Vec<u32> {
    decoder_ids[2..].iter().map(|&id| id as u32).collect()
}

impl LocalNllbEngine {
    pub fn load(encoder_path: &Path, decoder_path: &Path, tokenizer_path: &Path) -> Result<Self> {
        let encoder = Session::builder()
            .context("creating encoder session builder")?
            .commit_from_file(encoder_path)
            .context("loading encoder model")?;
        let decoder = Session::builder()
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

    fn translate_one(&self, text: &str, src_flores: &str, tgt_flores: &str) -> Result<String> {
        let src_lang_id = self
            .tokenizer
            .token_to_id(src_flores)
            .with_context(|| format!("unknown source language code: {src_flores}"))?
            as i64;
        let tgt_lang_id = self
            .tokenizer
            .token_to_id(tgt_flores)
            .with_context(|| format!("unknown target language code: {tgt_flores}"))?
            as i64;

        // The tokenizer's built-in post-processor bakes in a fixed source language, so we
        // tokenize without special tokens and add the source/eos tokens ourselves.
        let encoding = self
            .tokenizer
            .encode(text, false)
            .map_err(|e| anyhow::anyhow!("tokenizing text: {e}"))?;

        let input_ids = encoder_input_ids(src_lang_id, encoding.get_ids());
        let seq_len = input_ids.len();
        let attention_mask: Vec<i64> = vec![1; seq_len];

        let (enc_shape, encoder_hidden_states) = {
            let mut encoder = self.encoder.lock().unwrap();
            let input_ids_tensor =
                Tensor::from_array((vec![1i64, seq_len as i64], input_ids.clone()))?;
            let attention_mask_tensor =
                Tensor::from_array((vec![1i64, seq_len as i64], attention_mask.clone()))?;
            let outputs = encoder.run(ort::inputs![
                "input_ids" => input_ids_tensor,
                "attention_mask" => attention_mask_tensor,
            ])?;
            let (shape, data) = outputs["last_hidden_state"].try_extract_tensor::<f32>()?;
            (shape.iter().copied().collect::<Vec<i64>>(), data.to_vec())
        };

        // Greedy decode: [eos, target_lang] primes the decoder (matches NLLB's
        // decoder_start_token_id + forced_bos_token_id generation convention).
        let mut decoder_ids = decoder_priming(tgt_lang_id);
        let mut decoder = self.decoder.lock().unwrap();
        for _ in 0..MAX_NEW_TOKENS {
            let dec_len = decoder_ids.len();
            let decoder_input_tensor =
                Tensor::from_array((vec![1i64, dec_len as i64], decoder_ids.clone()))?;
            let enc_hidden_tensor =
                Tensor::from_array((enc_shape.clone(), encoder_hidden_states.clone()))?;
            let enc_mask_tensor =
                Tensor::from_array((vec![1i64, seq_len as i64], attention_mask.clone()))?;

            let outputs = decoder.run(ort::inputs![
                "encoder_attention_mask" => enc_mask_tensor,
                "input_ids" => decoder_input_tensor,
                "encoder_hidden_states" => enc_hidden_tensor,
            ])?;
            let (logits_shape, logits) = outputs["logits"].try_extract_tensor::<f32>()?;
            let vocab_size = logits_shape[2] as usize;
            let last_logits = last_position_logits(logits, dec_len, vocab_size);
            let next_id = argmax(last_logits).context("decoder produced no logits")?;

            if next_id == EOS_TOKEN_ID {
                break;
            }
            decoder_ids.push(next_id);
        }

        let generated = generated_tokens(&decoder_ids);
        self.tokenizer
            .decode(&generated, true)
            .map(|s| s.trim().to_string())
            .map_err(|e| anyhow::anyhow!("decoding generated tokens: {e}"))
    }
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

        let total = texts.len().max(1);
        let mut results = Vec::with_capacity(texts.len());
        for (i, text) in texts.iter().enumerate() {
            results.push(self.translate_one(text, src_flores, tgt_flores)?);
            on_progress((i + 1) as f32 / total as f32);
        }
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── résolution des codes de langue ───────────────────────────────────────

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
        // Les deux sont mauvais : c'est la cible, choisie par l'utilisateur, qu'on signale.
        let err = resolve_flores_pair(Some("xx"), "yy").unwrap_err();

        assert!(err.to_string().contains("target"), "{err}");
    }

    // ── séquences de tokens ──────────────────────────────────────────────────

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
    fn the_priming_is_stripped_from_the_generated_tokens() {
        let decoder_ids = vec![EOS_TOKEN_ID, 256_057, 11, 22, 33];

        assert_eq!(generated_tokens(&decoder_ids), vec![11, 22, 33]);
    }

    #[test]
    fn a_decoder_that_stopped_immediately_generated_nothing() {
        assert!(generated_tokens(&[EOS_TOKEN_ID, 256_057]).is_empty());
    }

    // ── décodage glouton ─────────────────────────────────────────────────────

    #[test]
    fn the_argmax_picks_the_highest_logit() {
        assert_eq!(argmax(&[0.1, 0.9, 0.3]), Some(1));
    }

    #[test]
    fn the_argmax_settles_ties_on_the_last_candidate() {
        // `max_by` conserve le dernier des ex æquo. En pratique deux logits flottants
        // strictement égaux n'arrivent pas ; le test fige juste le comportement.
        assert_eq!(argmax(&[0.5, 0.5]), Some(1));
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
        // Deux positions, vocabulaire de trois : on veut la seconde tranche.
        let logits = [1.0, 2.0, 3.0, 40.0, 50.0, 60.0];

        assert_eq!(last_position_logits(&logits, 2, 3), &[40.0, 50.0, 60.0]);
    }

    #[test]
    fn a_single_decoded_position_reads_the_whole_slice() {
        let logits = [1.0, 2.0, 3.0];

        assert_eq!(last_position_logits(&logits, 1, 3), &logits[..]);
    }

    #[test]
    fn reading_the_last_position_then_taking_its_argmax_gives_the_next_token() {
        let logits = [0.0, 0.0, 9.0, 0.1, 7.0, 0.2];

        let next = argmax(last_position_logits(&logits, 2, 3));

        assert_eq!(next, Some(1)); // 7.0 est le max de la seconde tranche
    }
}
