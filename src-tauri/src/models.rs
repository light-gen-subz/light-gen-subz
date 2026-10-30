use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::download::{self, Expected};

#[derive(Debug, Clone)]
pub struct ModelInfo {
    pub name: &'static str,
    pub filename: &'static str,
    pub url: &'static str,
    pub size_bytes: u64,
    pub sha256: &'static str,
}

/// Known ggml models (whisper.cpp). The URLs are pinned to a commit of the official
/// HuggingFace repo, and each file to its SHA-256, so a changed or tampered file is
/// refused instead of being handed to the native parser.
pub fn known_models() -> Vec<ModelInfo> {
    vec![
        ModelInfo {
            name: "base",
            filename: "ggml-base-q5_1.bin",
            url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-base-q5_1.bin",
            size_bytes: 59_707_625,
            sha256: "422f1ae452ade6f30a004d7e5c6a43195e4433bc370bf23fac9cc591f01a8898",
        },
        ModelInfo {
            name: "small",
            filename: "ggml-small-q5_1.bin",
            url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-small-q5_1.bin",
            size_bytes: 190_085_487,
            sha256: "ae85e4a935d7a567bd102fe55afc16bb595bdb618e11b2fc7591bc08120411bb",
        },
        ModelInfo {
            name: "medium",
            filename: "ggml-medium-q5_0.bin",
            url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-medium-q5_0.bin",
            size_bytes: 539_212_467,
            sha256: "19fea4b380c3a618ec4723c3eef2eb785ffba0d0538cf43f8f235e7b3b34220f",
        },
    ]
}

/// Silero voice-activity model: lets whisper skip silence and music instead of
/// hallucinating subtitles on them.
pub fn vad_model() -> ModelInfo {
    ModelInfo {
        name: "vad",
        filename: "ggml-silero-v5.1.2.bin",
        url: "https://huggingface.co/ggml-org/whisper-vad/resolve/9ffd54a1e1ee413ddf265af9913beaf518d1639b/ggml-silero-v5.1.2.bin",
        size_bytes: 885_098,
        sha256: "29940d98d42b91fbd05ce489f3ecf7c72f0a42f027e4875919a28fb4c04ea2cf",
    }
}

pub const DEFAULT_MODEL: &str = "small";

pub fn default_model() -> ModelInfo {
    model_named(DEFAULT_MODEL).expect("default model 'small' must exist in known_models()")
}

/// The model a setting names; an unknown name falls back to the default.
pub fn model_named(name: &str) -> Option<ModelInfo> {
    known_models().into_iter().find(|m| m.name == name)
}

pub fn models_dir_in(data_dir: &Path) -> Result<PathBuf> {
    let dir = data_dir.join("models");
    std::fs::create_dir_all(&dir).context("creating models directory")?;
    Ok(dir)
}

pub fn models_dir() -> Result<PathBuf> {
    models_dir_in(&crate::app_dirs::data_dir()?)
}

pub fn local_path(model: &ModelInfo) -> Result<PathBuf> {
    Ok(models_dir()?.join(model.filename))
}

/// Removes what version 1.0 left in the models folder: a `.sha256` next to each model,
/// computed from the download itself and never read. Returns how many were removed.
pub fn remove_legacy_files() -> usize {
    let Ok(dir) = models_dir() else { return 0 };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "sha256"))
        .filter(|e| std::fs::remove_file(e.path()).is_ok())
        .count()
}

/// Downloads the model if it is not already present (and intact), reporting progress.
pub async fn ensure_model_downloaded(
    model: &ModelInfo,
    on_progress: impl FnMut(f32),
) -> Result<PathBuf> {
    let dest = local_path(model)?;
    download::ensure_file(
        &dest,
        &Expected {
            url: model.url,
            size: model.size_bytes,
            sha256: model.sha256,
        },
        on_progress,
    )
    .await?;
    Ok(dest)
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
    fn every_url_is_pinned_to_a_commit_not_a_branch() {
        for model in known_models().into_iter().chain([vad_model()]) {
            assert!(!model.url.contains("/resolve/main/"), "{}", model.url);
        }
    }

    #[test]
    fn every_model_declares_a_checksum_and_a_size() {
        for model in known_models().into_iter().chain([vad_model()]) {
            assert_eq!(model.sha256.len(), 64, "{}", model.name);
            assert!(model.sha256.chars().all(|c| c.is_ascii_hexdigit()));
            assert!(model.size_bytes > 0);
        }
    }

    #[test]
    fn sizes_grow_with_the_model() {
        let models = known_models();
        assert!(models[0].size_bytes < models[1].size_bytes);
        assert!(models[1].size_bytes < models[2].size_bytes);
    }

    #[test]
    fn defaults_to_the_small_model() {
        assert_eq!(default_model().name, "small");
    }

    #[test]
    fn a_model_is_found_by_name() {
        assert_eq!(model_named("medium").unwrap().name, "medium");
        assert!(model_named("gigantic").is_none());
    }

    #[test]
    fn the_models_directory_is_created_under_the_given_base() {
        let base = tempfile::tempdir().unwrap();

        let dir = models_dir_in(base.path()).unwrap();

        assert!(dir.is_dir());
        assert_eq!(dir, base.path().join("models"));
    }
}
