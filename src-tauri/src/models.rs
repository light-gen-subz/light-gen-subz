use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use futures_util::StreamExt;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, serde::Serialize)]
pub struct ModelInfo {
    pub name: &'static str,
    pub filename: &'static str,
    pub url: &'static str,
    pub approx_size_mb: u32,
}

/// Known ggml models (whisper.cpp), hosted on the official HuggingFace repo.
/// `small-q5_1` is the MVP's default model: good accuracy/speed/size tradeoff.
pub fn known_models() -> Vec<ModelInfo> {
    vec![
        ModelInfo {
            name: "base",
            filename: "ggml-base-q5_1.bin",
            url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base-q5_1.bin",
            approx_size_mb: 60,
        },
        ModelInfo {
            name: "small",
            filename: "ggml-small-q5_1.bin",
            url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small-q5_1.bin",
            approx_size_mb: 190,
        },
        ModelInfo {
            name: "medium",
            filename: "ggml-medium-q5_0.bin",
            url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-medium-q5_0.bin",
            approx_size_mb: 540,
        },
    ]
}

pub fn default_model() -> ModelInfo {
    known_models()
        .into_iter()
        .find(|m| m.name == "small")
        .expect("default model 'small' must exist in known_models()")
}

pub fn models_dir() -> Result<PathBuf> {
    let dir = crate::app_dirs::data_dir()?.join("models");
    std::fs::create_dir_all(&dir).context("creating models directory")?;
    Ok(dir)
}

pub fn local_path(model: &ModelInfo) -> Result<PathBuf> {
    Ok(models_dir()?.join(model.filename))
}

/// Downloads the model if not already present locally, reporting progress (0.0-1.0).
/// Writes a .sha256 sidecar file to detect corruption on a future re-download.
pub async fn ensure_model_downloaded(
    model: &ModelInfo,
    mut on_progress: impl FnMut(f32),
) -> Result<PathBuf> {
    let dest = local_path(model)?;
    if dest.exists() {
        return Ok(dest);
    }

    let tmp_path = dest.with_extension("part");
    let response = reqwest::get(model.url)
        .await
        .with_context(|| format!("request to {} failed", model.url))?
        .error_for_status()
        .with_context(|| format!("HTTP error response for {}", model.url))?;

    let total_size = response.content_length().unwrap_or(0);
    let mut file = tokio::fs::File::create(&tmp_path)
        .await
        .context("creating temporary download file")?;
    let mut hasher = Sha256::new();
    let mut downloaded: u64 = 0;
    let mut stream = response.bytes_stream();

    use tokio::io::AsyncWriteExt;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("error while streaming the download")?;
        hasher.update(&chunk);
        file.write_all(&chunk)
            .await
            .context("writing downloaded file")?;
        downloaded += chunk.len() as u64;
        if total_size > 0 {
            on_progress(downloaded as f32 / total_size as f32);
        }
    }
    file.flush().await.context("flushing downloaded file")?;

    tokio::fs::rename(&tmp_path, &dest)
        .await
        .context("renaming downloaded model file")?;

    let checksum = format!("{:x}", hasher.finalize());
    write_checksum_sidecar(&dest, &checksum)?;

    Ok(dest)
}

fn write_checksum_sidecar(model_path: &Path, checksum: &str) -> Result<()> {
    let sidecar = model_path.with_extension("sha256");
    std::fs::write(sidecar, checksum).context("writing checksum file")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn lists_the_three_known_models() {
        let names: Vec<_> = known_models().into_iter().map(|m| m.name).collect();
        assert_eq!(names, vec!["base", "small", "medium"]);
    }

    #[test]
    fn every_model_has_a_distinct_file_name() {
        let files: HashSet<_> = known_models().into_iter().map(|m| m.filename).collect();
        assert_eq!(files.len(), 3);
    }

    #[test]
    fn every_model_points_at_the_official_repository() {
        for model in known_models() {
            assert!(
                model
                    .url
                    .starts_with("https://huggingface.co/ggerganov/whisper.cpp/"),
                "{} is hosted elsewhere: {}",
                model.name,
                model.url
            );
            assert!(model.url.ends_with(model.filename));
        }
    }

    #[test]
    fn every_model_declares_a_plausible_size() {
        for model in known_models() {
            assert!(model.approx_size_mb > 0);
            assert!(model.approx_size_mb < 5_000);
        }
    }

    #[test]
    fn sizes_grow_with_the_model() {
        let models = known_models();
        assert!(models[0].approx_size_mb < models[1].approx_size_mb);
        assert!(models[1].approx_size_mb < models[2].approx_size_mb);
    }

    #[test]
    fn defaults_to_the_small_model() {
        assert_eq!(default_model().name, "small");
    }

    #[test]
    fn the_default_is_one_of_the_known_models() {
        let default = default_model();
        assert!(known_models().iter().any(|m| m.name == default.name));
    }

    #[test]
    fn the_local_path_sits_under_the_models_directory() {
        let model = default_model();
        let path = local_path(&model).unwrap();

        assert!(path.ends_with(model.filename));
        assert_eq!(path.parent().unwrap(), models_dir().unwrap());
    }

    #[test]
    fn the_models_directory_is_created_on_demand() {
        let dir = models_dir().unwrap();
        assert!(dir.is_dir());
        assert!(dir.ends_with("models"));
    }

    #[test]
    fn model_info_serialises_for_the_frontend() {
        let json = serde_json::to_string(&default_model()).unwrap();
        assert!(json.contains("\"name\":\"small\""));
        assert!(json.contains("\"approx_size_mb\""));
    }
}
