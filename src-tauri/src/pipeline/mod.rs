pub mod audio_extract;
pub mod segmentation;
pub mod stt;
pub mod subtitle_writer;
pub mod translate;

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{Context, Result};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use zeroize::Zeroizing;

use crate::cancel;
use crate::config::{self, Settings, SttEngineChoice, TranslationEngineChoice};
use crate::errors::coded;
use crate::models;
use crate::paths;
use audio_extract::AudioFormat;
use stt::{
    AssemblyAiEngine, DeepgramEngine, LocalWhisperEngine, OpenAiCompatibleWhisperEngine, Segment,
    SttEngine, Transcript,
};
use translate::languages;
use translate::{
    AzureTranslateEngine, CloudDeepLEngine, GoogleTranslateEngine, LocalNllbEngine,
    OpenAiTranslateEngine, TranslationEngine,
};

#[derive(Debug, Clone, Serialize)]
pub struct PipelineProgress {
    pub stage: String,
    pub fraction: f32,
}

#[derive(Debug, Clone, Serialize)]
pub struct PipelineOutput {
    pub srt_path: String,
    pub srt_content: String,
    pub language: String,
}

/// Whether `fraction` is worth an IPC event after `last`: a download streams tens of
/// thousands of chunks, and each event re-renders the UI.
pub fn should_emit(last: Option<(&str, f32)>, stage: &str, fraction: f32) -> bool {
    match last {
        None => true,
        Some((last_stage, last_fraction)) => {
            last_stage != stage
                || fraction <= 0.0
                || fraction >= 1.0
                || (fraction - last_fraction).abs() >= 0.005
        }
    }
}

fn emit_progress(app: &AppHandle, stage: &str, fraction: f32) {
    static LAST: Mutex<Option<(String, f32)>> = Mutex::new(None);
    {
        let mut last = LAST.lock().unwrap_or_else(|p| p.into_inner());
        let previous = last.as_ref().map(|(s, f)| (s.as_str(), *f));
        if !should_emit(previous, stage, fraction) {
            return;
        }
        *last = Some((stage.to_string(), fraction));
    }
    let _ = app.emit(
        "pipeline-progress",
        PipelineProgress {
            stage: stage.to_string(),
            fraction,
        },
    );
}

fn require_api_key(key_name: &str, provider: &str) -> Result<Zeroizing<String>> {
    config::get_api_key(key_name)
        .with_context(|| format!("reading {provider} API key"))?
        .map(Zeroizing::new)
        .ok_or_else(|| {
            coded(
                "api_key_missing",
                format!("no {provider} API key configured — add one in Settings"),
            )
        })
}

/// API key required by a transcription engine: `(key constant, display name)`.
/// `None` for the local engine, which needs none.
///
/// Extracted from `run` so the table of requirements can be checked without running the
/// pipeline (which needs ffmpeg, a downloaded model and network access).
pub fn stt_key_requirement(choice: SttEngineChoice) -> Option<(&'static str, &'static str)> {
    match choice {
        SttEngineChoice::Local => None,
        SttEngineChoice::Groq => Some((config::GROQ_API_KEY, "Groq")),
        SttEngineChoice::OpenAi => Some((config::OPENAI_API_KEY, "OpenAI")),
        SttEngineChoice::Deepgram => Some((config::DEEPGRAM_API_KEY, "Deepgram")),
        SttEngineChoice::AssemblyAi => Some((config::ASSEMBLYAI_API_KEY, "AssemblyAI")),
    }
}

/// Same for the translation engines. `None` for the local engine and for "no translation".
pub fn translation_key_requirement(
    choice: TranslationEngineChoice,
) -> Option<(&'static str, &'static str)> {
    match choice {
        TranslationEngineChoice::DeepL => Some((config::DEEPL_API_KEY, "DeepL")),
        TranslationEngineChoice::OpenAi => Some((config::OPENAI_API_KEY, "OpenAI")),
        TranslationEngineChoice::Google => {
            Some((config::GOOGLE_TRANSLATE_API_KEY, "Google Translate"))
        }
        TranslationEngineChoice::Azure => Some((config::AZURE_TRANSLATOR_KEY, "Azure Translator")),
        TranslationEngineChoice::Local => None,
    }
}

/// Name of the file the transcription produces: `<source>.srt`, next to the input.
pub fn srt_output_path(input: &Path) -> PathBuf {
    input.with_extension("srt")
}

/// Name of the file the translation produces: `<source>.<language>.srt`, next to the source SRT.
pub fn translated_output_path(srt_path: &Path, target_lang: &str) -> PathBuf {
    let stem = srt_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("subtitles");
    srt_path.with_file_name(format!("{stem}.{target_lang}.srt"))
}

/// `path` if nothing is there yet, otherwise `name (2).ext`, `name (3).ext`… — an
/// existing subtitle file (hand-made, or from a previous run) is never overwritten.
pub fn unique_path(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("subtitles");
    let ext = path.extension().and_then(|e| e.to_str());
    (2..)
        .map(|n| {
            let name = match ext {
                Some(ext) => format!("{stem} ({n}).{ext}"),
                None => format!("{stem} ({n})"),
            };
            path.with_file_name(name)
        })
        .find(|candidate| !candidate.exists())
        .expect("an unused file name exists")
}

/// Puts the translated texts back on the original timings, refusing any offset: an engine
/// that merges or splits lines would put every following subtitle out of sync.
pub fn zip_translations(cues: &[Segment], translated: Vec<String>) -> Result<Vec<Segment>> {
    anyhow::ensure!(
        translated.len() == cues.len(),
        "translation engine returned {} results for {} cues",
        translated.len(),
        cues.len()
    );
    Ok(cues
        .iter()
        .zip(translated)
        .map(|(cue, text)| Segment {
            start: cue.start,
            end: cue.end,
            text,
        })
        .collect())
}

/// Builds the cloud transcription engine a setting names, or `None` for the local one.
fn build_cloud_stt(
    choice: SttEngineChoice,
    language: Option<String>,
) -> Result<Option<Box<dyn SttEngine + Send>>> {
    let Some((key, provider)) = stt_key_requirement(choice) else {
        return Ok(None);
    };
    let api_key = require_api_key(key, provider)?;
    let engine: Box<dyn SttEngine + Send> = match choice {
        SttEngineChoice::Groq => Box::new(
            OpenAiCompatibleWhisperEngine::groq(api_key.to_string()).with_language(language),
        ),
        SttEngineChoice::OpenAi => Box::new(
            OpenAiCompatibleWhisperEngine::openai(api_key.to_string()).with_language(language),
        ),
        SttEngineChoice::Deepgram => {
            Box::new(DeepgramEngine::new(api_key.to_string()).with_language(language))
        }
        SttEngineChoice::AssemblyAi => {
            Box::new(AssemblyAiEngine::new(api_key.to_string()).with_language(language))
        }
        SttEngineChoice::Local => unreachable!("the local engine needs no key"),
    };
    Ok(Some(engine))
}

/// Builds the cloud translation engine a setting names, or `None` when the choice is
/// local or disabled.
fn build_cloud_translator(
    settings: &Settings,
) -> Result<Option<Box<dyn TranslationEngine + Send>>> {
    let choice = settings.translation_engine;
    let Some((key, provider)) = translation_key_requirement(choice) else {
        return Ok(None);
    };
    let api_key = require_api_key(key, provider)?;
    let engine: Box<dyn TranslationEngine + Send> = match choice {
        TranslationEngineChoice::DeepL => Box::new(CloudDeepLEngine::new(api_key.to_string())),
        TranslationEngineChoice::OpenAi => {
            Box::new(OpenAiTranslateEngine::new(api_key.to_string()))
        }
        TranslationEngineChoice::Google => {
            Box::new(GoogleTranslateEngine::new(api_key.to_string()))
        }
        TranslationEngineChoice::Azure => {
            if settings.azure_translator_region.trim().is_empty() {
                return Err(coded(
                    "azure_region_missing",
                    "Azure Translator region not configured — add it in Settings",
                ));
            }
            Box::new(AzureTranslateEngine::new(
                api_key.to_string(),
                settings.azure_translator_region.trim(),
            ))
        }
        TranslationEngineChoice::Local => {
            unreachable!("no key requirement")
        }
    };
    Ok(Some(engine))
}

/// Runs blocking work off the async runtime's worker threads.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .context("background task")?
}

async fn run_stt_engine(
    engine: Box<dyn SttEngine + Send>,
    audio_path: PathBuf,
    app: AppHandle,
) -> Result<Transcript> {
    blocking(move || {
        engine.transcribe(
            &audio_path,
            Box::new(move |frac| emit_progress(&app, "transcribe", frac)),
        )
    })
    .await
}

async fn run_translation_engine(
    engine: Box<dyn TranslationEngine + Send>,
    texts: Vec<String>,
    source_lang: Option<String>,
    target_lang: String,
    app: AppHandle,
) -> Result<Vec<String>> {
    blocking(move || {
        engine.translate(
            &texts,
            source_lang.as_deref(),
            &target_lang,
            Box::new(move |frac| emit_progress(&app, "translate", frac)),
        )
    })
    .await
}

/// The name a result is offered under when the user exports it: the first free name derived
/// from `wanted`. Nothing is written: the subtitles live in memory until the user exports
/// them. The name is registered as one of the app's outputs, so that a translation of the
/// result is accepted.
fn suggest_output(wanted: &Path) -> PathBuf {
    let path = unique_path(wanted);
    paths::mark_output(&path);
    path
}

/// Orchestrates the full pipeline: audio extraction -> transcription -> segmentation -> SRT writing.
/// Writes `<source_name>.srt` next to the input file and returns its content.
pub async fn run(
    app: AppHandle,
    input_path: String,
    audio_track: Option<u32>,
) -> Result<PipelineOutput> {
    let _busy = cancel::Busy::enter();
    cancel::reset();
    let input = paths::from_webview(&input_path);
    // Only a file the user picked or dropped: the webview cannot name anything else.
    if !paths::is_allowed_input(&input) {
        return Err(coded(
            "path_not_allowed",
            format!("not a file chosen by the user: {input_path}"),
        ));
    }
    if !input.is_file() {
        return Err(coded(
            "file_not_found",
            format!("file not found: {input_path}"),
        ));
    }

    let settings = config::load_settings_or_default();
    let language = match settings.stt_language.as_str() {
        "auto" => None,
        code => Some(code.to_string()),
    };

    // Fail on a missing key before spending time on the audio.
    let cloud_engine = build_cloud_stt(settings.stt_engine, language.clone())?;
    // A cloud engine gets the audio compressed once, straight from the source.
    let format = cloud_engine
        .as_ref()
        .map_or(AudioFormat::Wav, |engine| engine.audio_format());

    emit_progress(&app, "extract_audio", 0.0);
    let cache_dir = app
        .path()
        .app_cache_dir()
        .context("resolving app cache directory")?;
    let source = input.clone();
    // Kept alive until the end of the run; dropping it deletes the temporary audio.
    let audio = blocking(move || {
        audio_extract::extract_audio(&source, &cache_dir, audio_track, format)
            .context("extracting audio via ffmpeg")
    })
    .await?;
    emit_progress(&app, "extract_audio", 1.0);

    let engine: Box<dyn SttEngine + Send> = match cloud_engine {
        Some(engine) => engine,
        None => {
            emit_progress(&app, "download_model", 0.0);
            let model =
                models::model_named(&settings.whisper_model).unwrap_or_else(models::default_model);
            let model_app = app.clone();
            let model_path = models::ensure_model_downloaded(&model, move |frac| {
                emit_progress(&model_app, "download_model", frac);
            })
            .await
            .context("downloading whisper model")?;

            // The voice-activity model is small and optional: without it the run still
            // works, only with more hallucinated text on silence. Offline, it is not
            // waited for.
            let vad = models::vad_model();
            let vad_path = tokio::time::timeout(
                VAD_DOWNLOAD_TIMEOUT,
                models::ensure_model_downloaded(&vad, |_| {}),
            )
            .await
            .ok()
            .and_then(Result::ok);
            emit_progress(&app, "download_model", 1.0);

            Box::new(
                LocalWhisperEngine::new(model_path)
                    .with_vad(vad_path)
                    .with_language(language),
            )
        }
    };

    emit_progress(&app, "transcribe", 0.0);
    let transcript = run_stt_engine(engine, audio.path.clone(), app.clone()).await?;
    emit_progress(&app, "transcribe", 1.0);
    drop(audio);

    emit_progress(&app, "write_subtitles", 0.0);
    let cues = segmentation::build_cues(&transcript.segments);
    let srt_content = subtitle_writer::to_srt(&cues);
    let srt_path = suggest_output(&srt_output_path(&input));
    emit_progress(&app, "write_subtitles", 1.0);

    Ok(PipelineOutput {
        srt_path: paths::to_webview(&srt_path),
        srt_content,
        language: transcript.language,
    })
}

/// How long the optional voice-activity model may take to download.
const VAD_DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Debug, Clone, Serialize)]
pub struct TranslationOutput {
    pub srt_path: String,
    pub srt_content: String,
    /// The language this translation is in.
    pub language: String,
}

/// Language codes the webview sent, checked against the supported list: they end up in
/// requests and in a file name.
pub fn validate_languages(
    source_lang: Option<&str>,
    target_lang: &str,
) -> Result<(Option<&'static str>, &'static str)> {
    let target = languages::normalize_code(target_lang).ok_or_else(|| {
        coded(
            "unsupported_language",
            format!("unsupported target language: {target_lang}"),
        )
    })?;
    let source = match source_lang {
        None => None,
        Some(code) => Some(languages::normalize_code(code).ok_or_else(|| {
            coded(
                "unsupported_language",
                format!("unsupported source language: {code}"),
            )
        })?),
    };
    Ok((source, target))
}

/// Translates an existing SRT's cue text (keeping timestamps). The result is kept in memory,
/// offered under the name `<source_name>.<target_lang>.srt`.
pub async fn translate(
    app: AppHandle,
    srt_path: String,
    srt_content: String,
    source_lang: Option<String>,
    target_lang: String,
) -> Result<TranslationOutput> {
    let _busy = cancel::Busy::enter();
    cancel::reset();
    // Only the result of a transcription made by the app.
    if !paths::is_output(&paths::from_webview(&srt_path)) {
        return Err(coded(
            "path_not_allowed",
            format!("not a subtitle file produced by the app: {srt_path}"),
        ));
    }
    let settings = config::load_settings_or_default();
    let (source_lang, target_lang) = validate_languages(source_lang.as_deref(), &target_lang)?;
    let source_lang = source_lang.map(str::to_string);
    let target_lang = target_lang.to_string();

    let cues = subtitle_writer::parse_srt(&srt_content);
    if cues.is_empty() {
        return Err(coded("no_cues", "no subtitle cues to translate"));
    }
    // The engines translate a line of text, not a wrapped one.
    let texts: Vec<String> = cues
        .iter()
        .map(|c| segmentation::flatten_lines(&c.text))
        .collect();

    // Fail on a missing key or region before downloading anything.
    let cloud_engine = build_cloud_translator(&settings)?;

    emit_progress(&app, "translate", 0.0);
    let translated_texts: Vec<String> = match cloud_engine {
        Some(engine) => {
            run_translation_engine(engine, texts, source_lang, target_lang.clone(), app.clone())
                .await?
        }
        None => {
            emit_progress(&app, "download_translation_model", 0.0);
            let model_app = app.clone();
            let files = translate::nllb_models::ensure_nllb_downloaded(move |frac| {
                emit_progress(&model_app, "download_translation_model", frac);
            })
            .await
            .context("downloading local translation model")?;
            emit_progress(&app, "download_translation_model", 1.0);

            let target = target_lang.clone();
            let progress_app = app.clone();
            blocking(move || -> Result<Vec<String>> {
                let engine =
                    LocalNllbEngine::cached(&files).context("loading local translation model")?;
                engine.translate(
                    &texts,
                    source_lang.as_deref(),
                    &target,
                    Box::new(move |frac| emit_progress(&progress_app, "translate", frac)),
                )
            })
            .await?
        }
    };
    emit_progress(&app, "translate", 1.0);

    let translated_cues = zip_translations(&cues, translated_texts)?;
    let translated_srt = subtitle_writer::to_srt(&translated_cues);

    let wanted = translated_output_path(&paths::from_webview(&srt_path), &target_lang);
    let out_path = suggest_output(&wanted);

    Ok(TranslationOutput {
        srt_path: paths::to_webview(&out_path),
        srt_content: translated_srt,
        language: target_lang,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cue(start: f64, end: f64, text: &str) -> Segment {
        Segment {
            start,
            end,
            text: text.to_string(),
        }
    }

    // ── required API keys ────────────────────────────────────────────────────

    #[test]
    fn the_local_transcription_engine_needs_no_key() {
        assert!(stt_key_requirement(SttEngineChoice::Local).is_none());
    }

    #[test]
    fn each_cloud_transcription_engine_names_its_key_and_provider() {
        let cases = [
            (SttEngineChoice::Groq, config::GROQ_API_KEY, "Groq"),
            (SttEngineChoice::OpenAi, config::OPENAI_API_KEY, "OpenAI"),
            (
                SttEngineChoice::Deepgram,
                config::DEEPGRAM_API_KEY,
                "Deepgram",
            ),
            (
                SttEngineChoice::AssemblyAi,
                config::ASSEMBLYAI_API_KEY,
                "AssemblyAI",
            ),
        ];

        for (choice, key, provider) in cases {
            assert_eq!(
                stt_key_requirement(choice),
                Some((key, provider)),
                "{choice:?}"
            );
        }
    }

    #[test]
    fn the_local_translation_engine_needs_no_key() {
        assert!(translation_key_requirement(TranslationEngineChoice::Local).is_none());
    }

    #[test]
    fn each_cloud_translation_engine_names_its_key_and_provider() {
        let cases = [
            (
                TranslationEngineChoice::DeepL,
                config::DEEPL_API_KEY,
                "DeepL",
            ),
            (
                TranslationEngineChoice::OpenAi,
                config::OPENAI_API_KEY,
                "OpenAI",
            ),
            (
                TranslationEngineChoice::Google,
                config::GOOGLE_TRANSLATE_API_KEY,
                "Google Translate",
            ),
            (
                TranslationEngineChoice::Azure,
                config::AZURE_TRANSLATOR_KEY,
                "Azure Translator",
            ),
        ];

        for (choice, key, provider) in cases {
            assert_eq!(
                translation_key_requirement(choice),
                Some((key, provider)),
                "{choice:?}"
            );
        }
    }

    #[test]
    fn transcription_and_translation_share_the_openai_key() {
        // A single OpenAI key must serve both uses.
        assert_eq!(
            stt_key_requirement(SttEngineChoice::OpenAi).unwrap().0,
            translation_key_requirement(TranslationEngineChoice::OpenAi)
                .unwrap()
                .0
        );
    }

    // ── output paths ─────────────────────────────────────────────────────────

    #[test]
    fn the_subtitles_land_next_to_the_media_file() {
        assert_eq!(
            srt_output_path(Path::new("/home/me/clip.mp4")),
            PathBuf::from("/home/me/clip.srt")
        );
    }

    #[test]
    fn a_media_file_without_an_extension_still_gets_one() {
        assert_eq!(
            srt_output_path(Path::new("/home/me/clip")),
            PathBuf::from("/home/me/clip.srt")
        );
    }

    #[test]
    fn a_dotted_media_name_only_loses_its_last_extension() {
        assert_eq!(
            srt_output_path(Path::new("/home/me/s01.e02.mkv")),
            PathBuf::from("/home/me/s01.e02.srt")
        );
    }

    #[test]
    fn the_translation_is_named_after_its_target_language() {
        assert_eq!(
            translated_output_path(Path::new("/home/me/clip.srt"), "fr"),
            PathBuf::from("/home/me/clip.fr.srt")
        );
    }

    #[test]
    fn translating_twice_does_not_overwrite_the_first_language() {
        let fr = translated_output_path(Path::new("/home/me/clip.srt"), "fr");
        let es = translated_output_path(Path::new("/home/me/clip.srt"), "es");

        assert_ne!(fr, es);
    }

    #[test]
    fn a_path_with_no_usable_stem_falls_back_to_subtitles() {
        assert_eq!(
            translated_output_path(Path::new("/"), "fr"),
            PathBuf::from("/subtitles.fr.srt")
        );
    }

    // ── putting translations back ────────────────────────────────────────────

    #[test]
    fn the_translated_text_keeps_the_original_timings() {
        let cues = vec![cue(0.0, 1.5, "Hello"), cue(1.5, 3.0, "World")];

        let out = zip_translations(&cues, vec!["Bonjour".into(), "Monde".into()]).unwrap();

        assert_eq!(out.len(), 2);
        assert_eq!(out[0].start, 0.0);
        assert_eq!(out[0].end, 1.5);
        assert_eq!(out[0].text, "Bonjour");
        assert_eq!(out[1].start, 1.5);
        assert_eq!(out[1].text, "Monde");
    }

    #[test]
    fn a_translation_that_lost_a_line_is_refused() {
        // Otherwise every following subtitle would be shifted.
        let cues = vec![cue(0.0, 1.0, "a"), cue(1.0, 2.0, "b")];

        let err = zip_translations(&cues, vec!["A".into()]).unwrap_err();

        assert!(
            err.to_string().contains("returned 1 results for 2 cues"),
            "{err}"
        );
    }

    #[test]
    fn a_translation_that_added_a_line_is_refused() {
        let cues = vec![cue(0.0, 1.0, "a")];

        assert!(zip_translations(&cues, vec!["A".into(), "B".into()]).is_err());
    }

    // ── progression ──────────────────────────────────────────────────────────

    #[test]
    fn the_first_event_is_always_sent() {
        assert!(should_emit(None, "download_model", 0.001));
    }

    #[test]
    fn tiny_steps_of_a_download_are_not_sent() {
        assert!(!should_emit(
            Some(("download_model", 0.500)),
            "download_model",
            0.502
        ));
    }

    #[test]
    fn a_visible_step_is_sent() {
        assert!(should_emit(
            Some(("download_model", 0.500)),
            "download_model",
            0.506
        ));
    }

    #[test]
    fn a_new_stage_and_the_two_ends_are_always_sent() {
        assert!(should_emit(Some(("extract_audio", 0.5)), "transcribe", 0.5));
        assert!(should_emit(Some(("transcribe", 0.5)), "transcribe", 1.0));
        assert!(should_emit(Some(("transcribe", 0.5)), "transcribe", 0.0));
    }

    // ── output files ─────────────────────────────────────────────────────────

    #[test]
    fn a_free_name_is_used_as_is() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("clip.srt");

        assert_eq!(unique_path(&path), path);
    }

    #[test]
    fn an_existing_subtitle_file_is_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("clip.srt");
        std::fs::write(&path, "hand made").unwrap();

        let next = unique_path(&path);

        assert_eq!(next, dir.path().join("clip (2).srt"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hand made");
    }

    #[test]
    fn the_numbering_goes_on_past_taken_names() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("clip.fr.srt");
        std::fs::write(&path, "a").unwrap();
        std::fs::write(dir.path().join("clip.fr (2).srt"), "b").unwrap();

        assert_eq!(unique_path(&path), dir.path().join("clip.fr (3).srt"));
    }

    #[test]
    fn a_suggested_output_is_registered_but_nothing_is_written() {
        let dir = tempfile::tempdir().unwrap();
        let wanted = dir.path().join("a.srt");

        let path = suggest_output(&wanted);

        assert_eq!(path, wanted);
        assert!(!path.exists());
        assert!(paths::is_output(&path));
    }

    #[test]
    fn a_suggested_output_avoids_a_file_that_is_already_there() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.srt"), "mine").unwrap();

        let path = suggest_output(&dir.path().join("a.srt"));

        assert_eq!(path, dir.path().join("a (2).srt"));
        assert_eq!(
            std::fs::read_to_string(dir.path().join("a.srt")).unwrap(),
            "mine"
        );
    }

    // ── language codes received from the webview ─────────────────────────────

    #[test]
    fn supported_languages_are_normalised() {
        assert_eq!(
            validate_languages(Some("EN"), "Fr").unwrap(),
            (Some("en"), "fr")
        );
        assert_eq!(validate_languages(None, "de").unwrap(), (None, "de"));
    }

    #[test]
    fn a_target_that_could_escape_the_folder_is_refused() {
        for bad in ["../x", "fr/../../etc", "fr\\x", ""] {
            assert!(validate_languages(None, bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn an_unsupported_source_is_refused() {
        let err = validate_languages(Some("xx"), "fr").unwrap_err();

        assert!(
            err.to_string().contains("unsupported source language"),
            "{err}"
        );
    }

    #[test]
    fn an_empty_set_of_cues_zips_to_nothing() {
        assert!(zip_translations(&[], vec![]).unwrap().is_empty());
    }

    /// Real run of everything but the network engines: ffmpeg, VAD + whisper, cues, SRT.
    /// `LGS_TEST_AUDIO=/path/speech.wav LGS_TEST_MODEL=/path/ggml-small-q5_1.bin
    /// cargo test real_local_run -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_local_run_produces_subtitles() {
        let audio = std::env::var("LGS_TEST_AUDIO").expect("LGS_TEST_AUDIO");
        let model = std::env::var("LGS_TEST_MODEL").expect("LGS_TEST_MODEL");
        let cache = tempfile::tempdir().unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let extracted =
            audio_extract::extract_audio(Path::new(&audio), cache.path(), None, AudioFormat::Wav)
                .unwrap();
        let vad = runtime
            .block_on(models::ensure_model_downloaded(
                &models::vad_model(),
                |_| {},
            ))
            .unwrap();
        // Tight limits force the split on the real word times.
        let transcript = LocalWhisperEngine::new(model)
            .with_vad(Some(vad))
            .with_limits(segmentation::CueLimits {
                max_secs: 3.0,
                max_chars: 84,
            })
            .transcribe(&extracted.path, Box::new(|_| {}))
            .unwrap();
        let srt = subtitle_writer::to_srt(&segmentation::build_cues(&transcript.segments));

        println!("language={} \n{srt}", transcript.language);
        assert_eq!(transcript.language, "en");
        assert!(srt.to_lowercase().contains("country"), "{srt}");
        let wav = extracted.path.clone();
        drop(extracted);
        assert!(!wav.exists(), "the temporary WAV must be deleted");
    }

    /// The VAD must keep silence and noise out of the transcript, and the cues must start
    /// where the speech does. `LGS_TEST_GAPS` is speech, 20 s of silence, speech, 20 s of
    /// silence, speech (11 s each, so speech at 0–11, 31–42 and 62–73 s), `LGS_TEST_NOISE`
    /// a recording with no speech at all.
    /// `cargo test real_vad -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_vad_keeps_silence_and_noise_out_and_cues_on_the_speech() {
        let gaps = std::env::var("LGS_TEST_GAPS").expect("LGS_TEST_GAPS");
        let noise = std::env::var("LGS_TEST_NOISE").expect("LGS_TEST_NOISE");
        let model = std::env::var("LGS_TEST_MODEL").expect("LGS_TEST_MODEL");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let vad = runtime
            .block_on(models::ensure_model_downloaded(
                &models::vad_model(),
                |_| {},
            ))
            .unwrap();
        let engine = || LocalWhisperEngine::new(model.clone()).with_vad(Some(vad.clone()));

        let nothing = engine()
            .transcribe(Path::new(&noise), Box::new(|_| {}))
            .unwrap();
        assert!(nothing.segments.is_empty(), "invented text: {nothing:?}");

        let transcript = engine()
            .transcribe(Path::new(&gaps), Box::new(|_| {}))
            .unwrap();
        let cues = segmentation::build_cues(&transcript.segments);
        println!("{}", subtitle_writer::to_srt(&cues));
        assert_eq!(transcript.language, "en");
        let speech = [(0.0, 11.0), (31.07, 42.0), (62.07, 73.0)];
        for cue in &cues {
            assert!(
                speech
                    .iter()
                    .any(|(s, e)| cue.start >= s - 0.6 && cue.start <= *e),
                "cue starts in a silence: {cue:?}"
            );
            assert!(
                speech
                    .iter()
                    .any(|(s, e)| cue.end >= *s && cue.end <= e + 2.0),
                "cue ends far from the speech: {cue:?}"
            );
        }
        for (start, _) in &speech[1..] {
            assert!(
                cues.iter().any(|c| (c.start - start).abs() < 1.0),
                "no cue starts with the speech at {start}"
            );
        }
    }
}
