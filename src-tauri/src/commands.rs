use std::path::{Path, PathBuf};

use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt;

use crate::cancel;
use crate::config::{self, Settings};
use crate::errors::CommandError;
use crate::paths;
use crate::pipeline::audio_extract::{self, AudioTrack};
use crate::pipeline::subtitle_writer;
use crate::pipeline::translate::languages::LANGUAGES;
use crate::pipeline::{self, PipelineOutput, TranslationOutput};

const MEDIA_EXTENSIONS: &[&str] = &[
    "mp4", "mkv", "mov", "avi", "webm", "mp3", "wav", "m4a", "flac", "ogg",
];

/// File dialogs match extensions case-sensitively on Linux (GTK): `IMG_0001.MOV` would
/// be hidden by a lower-case-only filter, so both spellings are listed.
pub fn filter_extensions(extensions: &[&str]) -> Vec<String> {
    extensions
        .iter()
        .flat_map(|e| [e.to_string(), e.to_uppercase()])
        .collect()
}

#[tauri::command]
pub async fn pick_file(app: AppHandle, filter_name: String) -> Option<String> {
    let extensions = filter_extensions(MEDIA_EXTENSIONS);
    let refs: Vec<&str> = extensions.iter().map(String::as_str).collect();
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .add_filter(filter_name, &refs)
        .pick_file(move |file_path| {
            let _ = tx.send(file_path);
        });
    let path = rx.await.ok().flatten().and_then(|f| f.into_path().ok())?;
    // From now on the webview may ask for this file, and only for files given this way.
    paths::allow_input(&path);
    Some(paths::to_webview(&path))
}

/// The audio tracks of a media file (several for a multi-language film).
#[tauri::command]
pub async fn list_audio_tracks(path: String) -> Result<Vec<AudioTrack>, CommandError> {
    let path = paths::from_webview(&path);
    if !paths::is_allowed_input(&path) {
        return Ok(Vec::new());
    }
    let tracks =
        tauri::async_runtime::spawn_blocking(move || audio_extract::list_audio_tracks(&path))
            .await
            .map_err(|e| CommandError::from(e.to_string()))??;
    Ok(tracks)
}

#[tauri::command]
pub async fn run_pipeline(
    app: AppHandle,
    input_path: String,
    audio_track: Option<u32>,
) -> Result<PipelineOutput, CommandError> {
    Ok(pipeline::run(app, input_path, audio_track).await?)
}

#[tauri::command]
pub fn cancel_pipeline() {
    cancel::request();
}

/// The extension a subtitle `format` is saved with. Anything unknown is SRT.
pub fn format_extension(format: &str) -> &'static str {
    match format {
        "vtt" => "vtt",
        "ass" => "ass",
        _ => "srt",
    }
}

/// `srt_content` (always SRT) rendered in the requested format.
pub fn render_subtitles(srt_content: &str, format: &str) -> String {
    match format {
        "vtt" => subtitle_writer::to_vtt(&subtitle_writer::parse_srt(srt_content)),
        "ass" => subtitle_writer::to_ass(&subtitle_writer::parse_srt(srt_content)),
        _ => srt_content.to_string(),
    }
}

/// A second file saved along with the one the user names (the translation next to the
/// original).
#[derive(serde::Deserialize)]
pub struct ExtraSubtitles {
    pub srt_path: String,
    pub srt_content: String,
}

/// The name an extra file takes next to the one the user chose: what its suggested name adds
/// to the first one's (`clip.srt` → `clip.fr.srt` adds `.fr`) is added to the chosen name.
pub fn sibling_path(
    chosen: &Path,
    first_suggested: &Path,
    extra_suggested: &Path,
    ext: &str,
) -> PathBuf {
    let stem = |p: &Path| {
        p.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("subtitles")
            .to_string()
    };
    let (first, extra) = (stem(first_suggested), stem(extra_suggested));
    let suffix = extra.strip_prefix(&first).unwrap_or(&extra);
    let base = if extra.starts_with(&first) {
        stem(chosen)
    } else {
        String::new()
    };
    chosen.with_file_name(format!("{base}{suffix}.{ext}"))
}

/// Asks where to save once, then writes there. The dialog lives on the Rust side so the
/// webview can never make the app write to a path the user did not choose. `extras` are
/// written next to the chosen file, never over an existing one. Returns the paths written
/// (the chosen one first), or `None` when the dialog was dismissed.
#[tauri::command]
pub async fn save_subtitle_as(
    app: AppHandle,
    default_path: String,
    filter_name: String,
    format: String,
    srt_content: String,
    extras: Option<Vec<ExtraSubtitles>>,
) -> Result<Option<Vec<String>>, CommandError> {
    let ext = format_extension(&format);
    let default = paths::from_webview(&default_path).with_extension(ext);

    let mut dialog = app.dialog().file().add_filter(filter_name, &[ext]);
    if let Some(dir) = default.parent().filter(|d| d.is_dir()) {
        dialog = dialog.set_directory(dir);
    }
    if let Some(name) = default.file_name().and_then(|n| n.to_str()) {
        dialog = dialog.set_file_name(name);
    }
    let (tx, rx) = tokio::sync::oneshot::channel();
    dialog.save_file(move |file_path| {
        let _ = tx.send(file_path);
    });

    let Some(dest) = rx.await.ok().flatten().and_then(|f| f.into_path().ok()) else {
        return Ok(None);
    };
    let dest: PathBuf = dest;
    let write = |path: &Path, content: &str| {
        std::fs::write(path, render_subtitles(content, &format))
            .map_err(|e| CommandError::from(format!("could not write {}: {e}", path.display())))
    };
    write(&dest, &srt_content)?;
    let mut saved = vec![paths::to_webview(&dest)];
    for extra in extras.unwrap_or_default() {
        let wanted = sibling_path(
            &dest,
            &paths::from_webview(&default_path),
            &paths::from_webview(&extra.srt_path),
            ext,
        );
        let path = crate::pipeline::unique_path(&wanted);
        write(&path, &extra.srt_content)?;
        saved.push(paths::to_webview(&path));
    }
    Ok(Some(saved))
}

/// Result of applying the user's edits.
#[derive(serde::Serialize)]
pub struct EditedSubtitles {
    pub srt_content: String,
}

/// Applies edited cue texts to subtitles held in memory and returns the new content.
/// Nothing is written: the user exports the result when they want it.
#[tauri::command(async)]
pub fn update_subtitles(
    srt_content: String,
    texts: Vec<String>,
) -> Result<EditedSubtitles, CommandError> {
    let cues = subtitle_writer::parse_srt(&srt_content);
    let edited = subtitle_writer::apply_text_edits(&cues, texts)?;
    Ok(EditedSubtitles {
        srt_content: subtitle_writer::to_srt(&edited),
    })
}

#[tauri::command]
pub async fn translate_subtitles(
    app: AppHandle,
    srt_path: String,
    srt_content: String,
    source_lang: Option<String>,
    target_lang: String,
) -> Result<TranslationOutput, CommandError> {
    Ok(pipeline::translate(app, srt_path, srt_content, source_lang, target_lang).await?)
}

#[tauri::command(async)]
pub fn list_languages() -> &'static [crate::pipeline::translate::languages::Language] {
    LANGUAGES
}

#[tauri::command(async)]
pub fn get_settings() -> Settings {
    config::load_settings().unwrap_or_default()
}

#[tauri::command(async)]
pub fn set_settings(settings: Settings) -> Result<(), CommandError> {
    config::save_settings(&settings)?;
    free_unused_models(&settings);
    Ok(())
}

/// A model that the chosen engines no longer use is a few hundred megabytes for nothing.
pub fn free_unused_models(settings: &Settings) {
    if settings.stt_engine != config::SttEngineChoice::Local {
        crate::pipeline::stt::local_whisper::release_cache();
    }
    if settings.translation_engine != config::TranslationEngineChoice::Local {
        crate::pipeline::translate::local_nllb::release_cache();
    }
}

// The keyring may talk to D-Bus and even prompt to unlock: never on the main thread.
#[tauri::command(async)]
pub fn set_api_key(key_name: String, value: String) -> Result<(), CommandError> {
    Ok(config::set_api_key(&key_name, &value)?)
}

#[tauri::command(async)]
pub fn delete_api_key(key_name: String) -> Result<(), CommandError> {
    Ok(config::delete_api_key(&key_name)?)
}

#[tauri::command(async)]
pub fn has_api_key(key_name: String) -> bool {
    config::has_api_key(&key_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_media_filter_covers_video_and_audio() {
        for ext in ["mp4", "mkv", "mov", "webm"] {
            assert!(
                MEDIA_EXTENSIONS.contains(&ext),
                "missing video extension: {ext}"
            );
        }
        for ext in ["mp3", "wav", "m4a", "flac", "ogg"] {
            assert!(
                MEDIA_EXTENSIONS.contains(&ext),
                "missing audio extension: {ext}"
            );
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
            assert!(!ext.starts_with('.'), "{ext} must not carry a dot");
            assert_eq!(*ext, ext.to_lowercase(), "{ext} must be lower case");
        }
    }

    #[test]
    fn the_dialog_filter_lists_both_spellings_of_every_extension() {
        let all = filter_extensions(&["mov", "mp4"]);

        for expected in ["mov", "MOV", "mp4", "MP4"] {
            assert!(all.contains(&expected.to_string()), "{expected}");
        }
        assert_eq!(all.len(), 4);
    }

    #[test]
    fn the_language_list_is_exposed_as_is() {
        assert_eq!(list_languages().len(), LANGUAGES.len());
        assert!(!list_languages().is_empty());
    }

    #[test]
    fn the_ass_format_has_its_own_extension_and_renderer() {
        assert_eq!(format_extension("ass"), "ass");
        let ass = render_subtitles("1\n00:00:00,000 --> 00:00:01,000\nHi\n\n", "ass");
        assert!(ass.starts_with("[Script Info]"));
    }

    #[test]
    fn an_extra_file_takes_the_chosen_name_plus_what_its_own_name_adds() {
        let first = Path::new("/v/clip.srt");
        let extra = Path::new("/v/clip.fr.srt");

        assert_eq!(
            sibling_path(Path::new("/out/episode.srt"), first, extra, "srt"),
            PathBuf::from("/out/episode.fr.srt")
        );
        assert_eq!(
            sibling_path(Path::new("/out/clip.vtt"), first, extra, "vtt"),
            PathBuf::from("/out/clip.fr.vtt")
        );
    }

    #[test]
    fn the_extra_files_are_read_with_the_field_names_the_frontend_sends() {
        // Tauri renames the arguments of a command, not the fields of a struct inside one.
        let ok: Vec<ExtraSubtitles> =
            serde_json::from_str(r#"[{"srt_path":"/a/clip.fr.srt","srt_content":"x"}]"#).unwrap();
        assert_eq!(ok[0].srt_path, "/a/clip.fr.srt");
        assert!(serde_json::from_str::<Vec<ExtraSubtitles>>(
            r#"[{"srtPath":"/a/clip.fr.srt","srtContent":"x"}]"#
        )
        .is_err());
    }

    #[test]
    fn an_extra_file_with_an_unrelated_name_keeps_its_own() {
        assert_eq!(
            sibling_path(
                Path::new("/out/a.srt"),
                Path::new("/v/clip.srt"),
                Path::new("/v/other.srt"),
                "srt"
            ),
            PathBuf::from("/out/other.srt")
        );
    }

    #[test]
    fn editing_returns_the_new_content_and_touches_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let before = std::fs::read_dir(dir.path()).unwrap().count();
        let srt = "1\n00:00:00,000 --> 00:00:01,000\nHi\n\n".to_string();

        let out = update_subtitles(srt, vec!["Edited".into()]).unwrap();

        assert!(out.srt_content.contains("Edited"));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), before);
    }

    #[test]
    fn only_files_the_user_chose_have_their_tracks_listed() {
        let tracks = tauri::async_runtime::block_on(list_audio_tracks(
            "http://example.com/x.mp4".to_string(),
        ))
        .unwrap();

        assert!(tracks.is_empty());
    }

    #[test]
    fn a_cloud_engine_frees_the_local_models() {
        let settings = Settings {
            stt_engine: config::SttEngineChoice::Groq,
            translation_engine: config::TranslationEngineChoice::DeepL,
            ..Settings::default()
        };

        free_unused_models(&settings);
    }

    #[test]
    fn an_edit_that_changes_the_number_of_cues_is_refused() {
        let srt = "1\n00:00:00,000 --> 00:00:01,000\nHi\n\n".to_string();

        assert!(update_subtitles(srt, vec![]).is_err());
    }

    #[test]
    fn an_unknown_format_is_saved_as_srt() {
        assert_eq!(format_extension("vtt"), "vtt");
        assert_eq!(format_extension("srt"), "srt");
        assert_eq!(format_extension("../../x"), "srt");
    }

    #[test]
    fn srt_content_is_saved_untouched() {
        let srt = "1\n00:00:00,000 --> 00:00:01,000\nHi\n\n";

        assert_eq!(render_subtitles(srt, "srt"), srt);
    }

    #[test]
    fn webvtt_is_rendered_from_the_srt() {
        let srt = "1\n00:00:00,000 --> 00:00:01,500\nHi\n\n";

        let vtt = render_subtitles(srt, "vtt");

        assert!(vtt.starts_with("WEBVTT"));
        assert!(vtt.contains("00:00:00.000 --> 00:00:01.500"));
    }

    #[test]
    fn the_webview_cannot_touch_an_arbitrary_keyring_entry() {
        assert!(!has_api_key("light_gen_subz_test_absent_key".to_string()));
        assert!(set_api_key("../../x".to_string(), "v".to_string()).is_err());
        assert!(delete_api_key("anything".to_string()).is_err());
    }

    #[test]
    fn a_failure_reaches_the_webview_with_its_code_and_its_whole_chain() {
        let err = set_api_key("nope".to_string(), "v".to_string()).unwrap_err();

        assert_eq!(err.code, Some("unknown_key"));
        assert!(!err.message.is_empty());
    }
}
