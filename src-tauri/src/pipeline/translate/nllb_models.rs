use std::path::PathBuf;

use anyhow::{Context, Result};
use futures_util::StreamExt;

const HF_BASE: &str = "https://huggingface.co/Xenova/nllb-200-distilled-600M/resolve/main";

pub struct NllbFiles {
    pub encoder: PathBuf,
    pub decoder: PathBuf,
    pub tokenizer: PathBuf,
}

fn nllb_dir() -> Result<PathBuf> {
    let dir = crate::app_dirs::data_dir()?
        .join("translate-models")
        .join("nllb-200-distilled-600M");
    std::fs::create_dir_all(&dir).context("creating translation model directory")?;
    Ok(dir)
}

/// Les trois fichiers du modèle : `(chemin distant, destination locale)`.
/// Séparé de `ensure_nllb_downloaded` pour être vérifiable sans télécharger ~900 Mo.
pub fn nllb_file_map(dir: &std::path::Path) -> [(&'static str, PathBuf); 3] {
    [
        (
            "onnx/encoder_model_quantized.onnx",
            dir.join("encoder.onnx"),
        ),
        (
            "onnx/decoder_model_quantized.onnx",
            dir.join("decoder.onnx"),
        ),
        ("tokenizer.json", dir.join("tokenizer.json")),
    ]
}

/// URL complète d'un fichier du modèle sur HuggingFace.
pub fn remote_url(remote: &str) -> String {
    format!("{HF_BASE}/{remote}")
}

/// Progression globale : chaque fichier occupe une tranche égale de la barre.
pub fn overall_progress(index: usize, fraction: f32, total_files: usize) -> f32 {
    (index as f32 + fraction) / total_files as f32
}

/// Downloads the local NLLB translation model (encoder + decoder + tokenizer, ~900MB total)
/// if not already present, reporting overall progress (0.0-1.0) across all three files.
pub async fn ensure_nllb_downloaded(mut on_progress: impl FnMut(f32)) -> Result<NllbFiles> {
    let dir = nllb_dir()?;
    let files = nllb_file_map(&dir);

    let total_files = files.len();
    for (i, (remote, dest)) in files.iter().enumerate() {
        if !dest.exists() {
            download_file(&remote_url(remote), dest, |frac| {
                on_progress(overall_progress(i, frac, total_files));
            })
            .await?;
        }
        on_progress(overall_progress(i, 1.0, total_files));
    }

    Ok(NllbFiles {
        encoder: dir.join("encoder.onnx"),
        decoder: dir.join("decoder.onnx"),
        tokenizer: dir.join("tokenizer.json"),
    })
}

async fn download_file(
    url: &str,
    dest: &std::path::Path,
    mut on_progress: impl FnMut(f32),
) -> Result<()> {
    let tmp_path = dest.with_extension("part");
    let response = reqwest::get(url)
        .await
        .with_context(|| format!("request to {url} failed"))?
        .error_for_status()
        .with_context(|| format!("HTTP error response for {url}"))?;

    let total_size = response.content_length().unwrap_or(0);
    let mut file = tokio::fs::File::create(&tmp_path)
        .await
        .context("creating temporary download file")?;
    let mut downloaded: u64 = 0;
    let mut stream = response.bytes_stream();

    use tokio::io::AsyncWriteExt;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("error while streaming the download")?;
        file.write_all(&chunk)
            .await
            .context("writing downloaded file")?;
        downloaded += chunk.len() as u64;
        if total_size > 0 {
            on_progress(downloaded as f32 / total_size as f32);
        }
    }
    file.flush().await.context("flushing downloaded file")?;

    tokio::fs::rename(&tmp_path, dest)
        .await
        .context("renaming downloaded file")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn the_model_needs_an_encoder_a_decoder_and_a_tokenizer() {
        let files = nllb_file_map(Path::new("/models/nllb"));

        let names: Vec<_> = files
            .iter()
            .map(|(_, dest)| dest.file_name().unwrap().to_str().unwrap())
            .collect();
        assert_eq!(names, ["encoder.onnx", "decoder.onnx", "tokenizer.json"]);
    }

    #[test]
    fn the_quantized_onnx_weights_are_the_ones_fetched() {
        let files = nllb_file_map(Path::new("/models/nllb"));

        assert!(files[0].0.contains("encoder_model_quantized"));
        assert!(files[1].0.contains("decoder_model_quantized"));
    }

    #[test]
    fn every_file_lands_in_the_given_directory() {
        let dir = Path::new("/models/nllb");

        for (_, dest) in nllb_file_map(dir) {
            assert_eq!(dest.parent().unwrap(), dir);
        }
    }

    #[test]
    fn the_urls_point_at_the_distilled_600m_repository() {
        let url = remote_url("tokenizer.json");

        assert_eq!(
            url,
            "https://huggingface.co/Xenova/nllb-200-distilled-600M/resolve/main/tokenizer.json"
        );
    }

    #[test]
    fn the_progress_spans_zero_to_one_across_the_three_files() {
        assert!((overall_progress(0, 0.0, 3) - 0.0).abs() < 1e-6);
        assert!((overall_progress(2, 1.0, 3) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn each_file_owns_an_equal_slice_of_the_progress() {
        // Le premier fichier terminé = un tiers de la barre.
        assert!((overall_progress(0, 1.0, 3) - 1.0 / 3.0).abs() < 1e-6);
        assert!((overall_progress(1, 0.0, 3) - 1.0 / 3.0).abs() < 1e-6);
    }

    #[test]
    fn the_progress_never_goes_backwards() {
        let steps = [
            overall_progress(0, 0.5, 3),
            overall_progress(1, 0.0, 3),
            overall_progress(1, 0.5, 3),
            overall_progress(2, 1.0, 3),
        ];

        assert!(steps.windows(2).all(|w| w[1] >= w[0]), "{steps:?}");
    }
}
