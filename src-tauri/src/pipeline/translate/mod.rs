pub mod azure_translate;
pub mod cloud_deepl;
pub mod google_translate;
pub mod languages;
pub mod local_nllb;
pub mod nllb_models;
pub mod openai_translate;

use std::ops::Range;

use anyhow::Result;

pub use azure_translate::AzureTranslateEngine;
pub use cloud_deepl::CloudDeepLEngine;
pub use google_translate::GoogleTranslateEngine;
pub use local_nllb::LocalNllbEngine;
pub use openai_translate::OpenAiTranslateEngine;

use crate::cancel;

pub trait TranslationEngine {
    /// Translates a batch of texts, in order. `source_lang` is `None` for auto-detect.
    fn translate(
        &self,
        texts: &[String],
        source_lang: Option<&str>,
        target_lang: &str,
        on_progress: Box<dyn FnMut(f32) + Send>,
    ) -> anyhow::Result<Vec<String>>;
}

/// Request size limits of a cloud provider.
#[derive(Debug, Clone, Copy)]
pub struct BatchLimits {
    pub max_items: usize,
    pub max_chars: usize,
}

/// Cuts `texts` into consecutive ranges that each respect `limits`. A single text
/// larger than `max_chars` still gets a range of its own.
pub fn batch_ranges(texts: &[String], limits: BatchLimits) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut start = 0;
    let mut chars = 0;
    for (i, text) in texts.iter().enumerate() {
        let len = text.chars().count();
        let full = i > start && (i - start >= limits.max_items || chars + len > limits.max_chars);
        if full {
            ranges.push(start..i);
            start = i;
            chars = 0;
        }
        chars += len;
    }
    if start < texts.len() {
        ranges.push(start..texts.len());
    }
    ranges
}

/// Lines of the previous batch handed to the next one as context: a sentence that runs
/// over two cues is better translated when the engine has seen the first half.
pub const CONTEXT_LINES: usize = 2;

/// Translates `texts` batch by batch, in order, reporting real progress. Every batch
/// must come back with exactly as many lines as it went out with, otherwise the cues
/// would be shifted against their timings. The closure receives the batch and the last
/// [`CONTEXT_LINES`] source lines before it (empty for the first batch).
pub fn translate_in_batches(
    texts: &[String],
    limits: BatchLimits,
    on_progress: &mut dyn FnMut(f32),
    mut translate_batch: impl FnMut(&[String], &[String]) -> Result<Vec<String>>,
) -> Result<Vec<String>> {
    on_progress(0.0);
    let mut out = Vec::with_capacity(texts.len());
    for range in batch_ranges(texts, limits) {
        cancel::check()?;
        let batch = &texts[range.clone()];
        let context = &texts[range.start.saturating_sub(CONTEXT_LINES)..range.start];
        let translated = translate_batch(batch, context)?;
        anyhow::ensure!(
            translated.len() == batch.len(),
            "translation engine returned {} results for {} lines",
            translated.len(),
            batch.len()
        );
        out.extend(translated);
        on_progress(out.len() as f32 / texts.len().max(1) as f32);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("line {i}")).collect()
    }

    const LIMITS: BatchLimits = BatchLimits {
        max_items: 50,
        max_chars: 1_000,
    };

    #[test]
    fn a_small_input_is_one_batch() {
        assert_eq!(batch_ranges(&lines(10), LIMITS), vec![0..10]);
    }

    #[test]
    fn an_input_over_the_item_limit_is_split() {
        let ranges = batch_ranges(&lines(120), LIMITS);

        assert_eq!(ranges, vec![0..50, 50..100, 100..120]);
    }

    #[test]
    fn an_input_over_the_character_limit_is_split() {
        let texts = vec!["x".repeat(400), "y".repeat(400), "z".repeat(400)];

        let ranges = batch_ranges(&texts, LIMITS);

        assert_eq!(ranges, vec![0..2, 2..3]);
    }

    #[test]
    fn a_text_larger_than_the_limit_gets_a_batch_of_its_own() {
        let texts = vec!["x".repeat(5_000), "short".to_string()];

        assert_eq!(batch_ranges(&texts, LIMITS), vec![0..1, 1..2]);
    }

    #[test]
    fn nothing_to_translate_means_no_batch() {
        assert!(batch_ranges(&[], LIMITS).is_empty());
    }

    #[test]
    fn the_batches_cover_every_line_exactly_once() {
        let texts = lines(333);

        let ranges = batch_ranges(&texts, LIMITS);

        let covered: usize = ranges.iter().map(|r| r.len()).sum();
        assert_eq!(covered, 333);
        for pair in ranges.windows(2) {
            assert_eq!(pair[0].end, pair[1].start);
        }
    }

    #[test]
    fn results_are_joined_in_order_and_progress_is_real() {
        let texts = lines(120);
        let mut seen = Vec::new();

        let out = translate_in_batches(&texts, LIMITS, &mut |p| seen.push(p), |batch, _| {
            Ok(batch.iter().map(|t| t.to_uppercase()).collect())
        })
        .unwrap();

        assert_eq!(out[0], "LINE 0");
        assert_eq!(out[119], "LINE 119");
        assert_eq!(out.len(), 120);
        assert_eq!(seen.first(), Some(&0.0));
        assert!((seen.last().unwrap() - 1.0).abs() < 1e-6);
        assert!(seen.windows(2).all(|w| w[1] >= w[0]));
        assert!(
            seen.len() > 3,
            "progress must advance batch by batch: {seen:?}"
        );
    }

    #[test]
    fn a_batch_that_lost_a_line_is_refused() {
        let err = translate_in_batches(&lines(3), LIMITS, &mut |_| {}, |_, _| Ok(vec!["a".into()]))
            .unwrap_err();

        assert!(
            err.to_string().contains("returned 1 results for 3 lines"),
            "{err}"
        );
    }

    #[test]
    fn a_failing_batch_stops_the_run() {
        let mut calls = 0;

        let result = translate_in_batches(&lines(120), LIMITS, &mut |_| {}, |_, _| {
            calls += 1;
            anyhow::bail!("quota")
        });

        assert!(result.is_err());
        assert_eq!(calls, 1);
    }

    #[test]
    fn a_batch_gets_the_last_lines_of_the_previous_one_as_context() {
        let texts = lines(120);
        let mut seen: Vec<Vec<String>> = Vec::new();

        translate_in_batches(&texts, LIMITS, &mut |_| {}, |batch, context| {
            seen.push(context.to_vec());
            Ok(batch.to_vec())
        })
        .unwrap();

        assert!(seen[0].is_empty());
        assert_eq!(seen[1], vec!["line 48".to_string(), "line 49".to_string()]);
    }
}
