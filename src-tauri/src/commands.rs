use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt;

use crate::config::{self, Settings};
use crate::models;
use crate::pipeline::translate::languages::LANGUAGES;
use crate::pipeline::{self, PipelineOutput, TranslationOutput};

const MEDIA_EXTENSIONS: &[&str] = &[
    "mp4", "mkv", "mov", "avi", "webm", "mp3", "wav", "m4a", "flac", "ogg",
];

#[tauri::command]
pub async fn pick_file(app: AppHandle) -> Option<String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .add_filter("Video / Audio", MEDIA_EXTENSIONS)
        .pick_file(move |file_path| {
            let _ = tx.send(file_path);
        });
    rx.await
        .ok()
        .flatten()
        .and_then(|f| f.into_path().ok())
        .map(|p| p.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn run_pipeline(app: AppHandle, input_path: String) -> Result<PipelineOutput, String> {
    pipeline::run(app, input_path)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_models() -> Vec<models::ModelInfo> {
    models::known_models()
}

#[tauri::command]
pub fn save_subtitle(dest_path: String, content: String) -> Result<(), String> {
    std::fs::write(&dest_path, content).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn translate_subtitles(
    app: AppHandle,
    srt_path: String,
    srt_content: String,
    source_lang: Option<String>,
    target_lang: String,
) -> Result<TranslationOutput, String> {
    pipeline::translate(app, srt_path, srt_content, source_lang, target_lang)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_languages() -> &'static [crate::pipeline::translate::languages::Language] {
    LANGUAGES
}

#[tauri::command]
pub fn get_settings() -> Settings {
    config::load_settings().unwrap_or_default()
}

#[tauri::command]
pub fn set_settings(settings: Settings) -> Result<(), String> {
    config::save_settings(&settings).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_api_key(key_name: String, value: String) -> Result<(), String> {
    config::set_api_key(&key_name, &value).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn has_api_key(key_name: String) -> bool {
    config::has_api_key(&key_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_media_filter_covers_video_and_audio() {
        for ext in ["mp4", "mkv", "mov", "webm"] {
            assert!(MEDIA_EXTENSIONS.contains(&ext), "vidéo manquante : {ext}");
        }
        for ext in ["mp3", "wav", "m4a", "flac", "ogg"] {
            assert!(MEDIA_EXTENSIONS.contains(&ext), "audio manquant : {ext}");
        }
    }

    #[test]
    fn the_media_filter_has_no_duplicate() {
        let unique: std::collections::HashSet<_> = MEDIA_EXTENSIONS.iter().collect();

        assert_eq!(unique.len(), MEDIA_EXTENSIONS.len());
    }

    #[test]
    fn the_extensions_are_lower_case_and_dotless() {
        for ext in MEDIA_EXTENSIONS {
            assert!(!ext.starts_with('.'), "{ext} ne doit pas porter de point");
            assert_eq!(*ext, ext.to_lowercase(), "{ext} doit être en minuscules");
        }
    }

    #[test]
    fn the_model_list_is_the_one_the_pipeline_knows() {
        assert_eq!(list_models().len(), models::known_models().len());
        assert!(list_models().iter().any(|m| m.name == "small"));
    }

    #[test]
    fn the_language_list_is_exposed_as_is() {
        assert_eq!(list_languages().len(), LANGUAGES.len());
        assert!(!list_languages().is_empty());
    }

    #[test]
    fn saving_a_subtitle_writes_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("clip.srt");

        save_subtitle(
            dest.to_string_lossy().to_string(),
            "1\n00:00:00,000 --> 00:00:01,000\nBonjour\n\n".to_string(),
        )
        .unwrap();

        assert!(std::fs::read_to_string(&dest).unwrap().contains("Bonjour"));
    }

    #[test]
    fn saving_overwrites_an_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("clip.srt");
        std::fs::write(&dest, "ancien").unwrap();

        save_subtitle(dest.to_string_lossy().to_string(), "nouveau".to_string()).unwrap();

        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "nouveau");
    }

    #[test]
    fn saving_to_an_unwritable_path_is_reported() {
        let err =
            save_subtitle("/nowhere-at-all/clip.srt".to_string(), "x".to_string()).unwrap_err();

        assert!(!err.is_empty());
    }

    #[test]
    fn an_absent_api_key_is_reported_as_missing() {
        assert!(!has_api_key("light_gen_subz_test_absent_key".to_string()));
    }
}
