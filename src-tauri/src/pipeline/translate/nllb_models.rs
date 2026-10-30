use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::download::{self, Expected};

/// Pinned to a commit: the files below are checked against these digests.
const HF_BASE: &str =
    "https://huggingface.co/Xenova/nllb-200-distilled-600M/resolve/261c31d1a5732c67cdd16d80e8d6088507c7ccea";

pub struct NllbFiles {
    pub encoder: PathBuf,
    pub decoder: PathBuf,
    pub tokenizer: PathBuf,
}

/// A model file: remote path, local name, size and SHA-256.
pub struct NllbFile {
    pub remote: &'static str,
    pub local: &'static str,
    pub size: u64,
    pub sha256: &'static str,
}

pub const NLLB_FILES: [NllbFile; 3] = [
    NllbFile {
        remote: "onnx/encoder_model_quantized.onnx",
        local: "encoder.onnx",
        size: 419_120_483,
        sha256: "5cde664eacba07a62f198857ec6c06e09572b1ebb77c8137f1fa99ac604a3a28",
    },
    NllbFile {
        // The "merged" decoder runs both with and without a key/value cache.
        remote: "onnx/decoder_model_merged_quantized.onnx",
        local: "decoder.onnx",
        size: 475_505_771,
        sha256: "dd66608c2a4194e78f95548fa0e64f24302303698c5b09fa8e1f9e16ec00676b",
    },
    NllbFile {
        remote: "tokenizer.json",
        local: "tokenizer.json",
        size: 17_331_224,
        sha256: "8ac789ad7dabea44d41537822d48c516ba358374c51813e2cba78c006e150c94",
    },
];

fn nllb_dir() -> Result<PathBuf> {
    let dir = crate::app_dirs::data_dir()?
        .join("translate-models")
        .join("nllb-200-distilled-600M");
    std::fs::create_dir_all(&dir).context("creating translation model directory")?;
    Ok(dir)
}

/// Full URL of a model file on HuggingFace.
pub fn remote_url(remote: &str) -> String {
    format!("{HF_BASE}/{remote}")
}

/// Overall progress: the files weigh very differently, so each takes a slice proportional
/// to its size.
pub fn overall_progress(index: usize, fraction: f32) -> f32 {
    let total: u64 = NLLB_FILES.iter().map(|f| f.size).sum();
    let done: u64 = NLLB_FILES.iter().take(index).map(|f| f.size).sum();
    let current = NLLB_FILES.get(index).map_or(0, |f| f.size);
    (done as f32 + fraction * current as f32) / total as f32
}

/// Downloads the local NLLB translation model (encoder + decoder + tokenizer, ~900MB total)
/// if not already present, reporting overall progress (0.0-1.0) across all three files.
pub async fn ensure_nllb_downloaded(mut on_progress: impl FnMut(f32)) -> Result<NllbFiles> {
    let dir = nllb_dir()?;

    for (i, file) in NLLB_FILES.iter().enumerate() {
        let url = remote_url(file.remote);
        download::ensure_file(
            &dir.join(file.local),
            &Expected {
                url: &url,
                size: file.size,
                sha256: file.sha256,
            },
            |frac| on_progress(overall_progress(i, frac)),
        )
        .await?;
        on_progress(overall_progress(i, 1.0));
    }

    Ok(NllbFiles {
        encoder: dir.join("encoder.onnx"),
        decoder: dir.join("decoder.onnx"),
        tokenizer: dir.join("tokenizer.json"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_model_needs_an_encoder_a_decoder_and_a_tokenizer() {
        let names: Vec<_> = NLLB_FILES.iter().map(|f| f.local).collect();

        assert_eq!(names, ["encoder.onnx", "decoder.onnx", "tokenizer.json"]);
    }

    #[test]
    fn the_quantized_onnx_weights_are_the_ones_fetched() {
        assert!(NLLB_FILES[0].remote.contains("encoder_model_quantized"));
        assert!(NLLB_FILES[1]
            .remote
            .contains("decoder_model_merged_quantized"));
    }

    #[test]
    fn the_urls_are_pinned_to_a_commit_of_the_distilled_600m_repository() {
        let url = remote_url("tokenizer.json");

        assert_eq!(
            url,
            "https://huggingface.co/Xenova/nllb-200-distilled-600M/resolve/261c31d1a5732c67cdd16d80e8d6088507c7ccea/tokenizer.json"
        );
    }

    #[test]
    fn every_file_declares_a_checksum_and_a_size() {
        for file in &NLLB_FILES {
            assert_eq!(file.sha256.len(), 64, "{}", file.local);
            assert!(file.size > 0);
        }
    }

    #[test]
    fn the_progress_spans_zero_to_one_across_the_three_files() {
        assert!(overall_progress(0, 0.0).abs() < 1e-6);
        assert!((overall_progress(2, 1.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn a_big_file_weighs_more_than_the_tokenizer() {
        // The 17 MB tokenizer must not take a third of the bar.
        assert!(overall_progress(2, 0.0) > 0.9);
    }

    #[test]
    fn the_progress_never_goes_backwards() {
        let steps = [
            overall_progress(0, 0.5),
            overall_progress(0, 1.0),
            overall_progress(1, 0.0),
            overall_progress(1, 0.5),
            overall_progress(2, 1.0),
        ];

        assert!(steps.windows(2).all(|w| w[1] >= w[0]), "{steps:?}");
    }
}
