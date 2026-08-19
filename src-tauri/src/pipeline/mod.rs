pub mod audio_extract;
pub mod segmentation;
pub mod stt;
pub mod subtitle_writer;
pub mod translate;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::config::{self, SttEngineChoice, TranslationEngineChoice};
use crate::models;
use stt::{
    AssemblyAiEngine, DeepgramEngine, LocalWhisperEngine, OpenAiCompatibleWhisperEngine, Segment,
    SttEngine, Transcript,
};
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

fn emit_progress(app: &AppHandle, stage: &str, fraction: f32) {
    let _ = app.emit(
        "pipeline-progress",
        PipelineProgress {
            stage: stage.to_string(),
            fraction,
        },
    );
}

fn require_api_key(key_name: &str, provider: &str) -> Result<String> {
    config::get_api_key(key_name)
        .with_context(|| format!("reading {provider} API key"))?
        .with_context(|| format!("no {provider} API key configured — add one in Settings"))
}

/// Clé d'API requise par un moteur de transcription : `(constante de clé, nom affiché)`.
/// `None` pour le moteur local, qui n'en demande aucune.
///
/// Extrait de `run` pour que la table des exigences soit vérifiable sans exécuter le
/// pipeline (qui suppose ffmpeg, un modèle téléchargé et un accès réseau).
pub fn stt_key_requirement(choice: SttEngineChoice) -> Option<(&'static str, &'static str)> {
    match choice {
        SttEngineChoice::Local => None,
        SttEngineChoice::Groq => Some((config::GROQ_API_KEY, "Groq")),
        SttEngineChoice::OpenAi => Some((config::OPENAI_API_KEY, "OpenAI")),
        SttEngineChoice::Deepgram => Some((config::DEEPGRAM_API_KEY, "Deepgram")),
        SttEngineChoice::AssemblyAi => Some((config::ASSEMBLYAI_API_KEY, "AssemblyAI")),
    }
}

/// Idem pour les moteurs de traduction. `None` pour le moteur local et pour « aucune
/// traduction ».
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
        TranslationEngineChoice::Local | TranslationEngineChoice::None => None,
    }
}

/// Nom du fichier produit par la transcription : `<source>.srt`, à côté de l'entrée.
pub fn srt_output_path(input: &Path) -> PathBuf {
    input.with_extension("srt")
}

/// Nom du fichier produit par la traduction : `<source>.<langue>.srt`, à côté du SRT source.
pub fn translated_output_path(srt_path: &Path, target_lang: &str) -> PathBuf {
    let stem = srt_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("subtitles");
    srt_path.with_file_name(format!("{stem}.{target_lang}.srt"))
}

/// Réassocie les textes traduits aux minutages d'origine, en refusant tout décalage :
/// un moteur qui fusionne ou scinde des lignes désynchroniserait tous les sous-titres.
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

async fn run_stt_engine(
    engine: Box<dyn SttEngine + Send>,
    wav_path: PathBuf,
    app: AppHandle,
) -> Result<Transcript> {
    tauri::async_runtime::spawn_blocking(move || {
        engine.transcribe(
            &wav_path,
            Box::new(move |frac| emit_progress(&app, "transcribe", frac)),
        )
    })
    .await
    .context("transcription task")?
}

async fn run_translation_engine(
    engine: Box<dyn TranslationEngine + Send>,
    texts: Vec<String>,
    source_lang: Option<String>,
    target_lang: String,
    app: AppHandle,
) -> Result<Vec<String>> {
    tauri::async_runtime::spawn_blocking(move || {
        engine.translate(
            &texts,
            source_lang.as_deref(),
            &target_lang,
            Box::new(move |frac| emit_progress(&app, "translate", frac)),
        )
    })
    .await
    .context("translation task")?
}

/// Orchestrates the full pipeline: audio extraction -> transcription -> segmentation -> SRT writing.
/// Writes `<source_name>.srt` next to the input file and returns its content.
pub async fn run(app: AppHandle, input_path: String) -> Result<PipelineOutput> {
    let input = Path::new(&input_path);
    anyhow::ensure!(input.exists(), "file not found: {input_path}");

    let settings = config::load_settings().context("loading settings")?;

    emit_progress(&app, "extract_audio", 0.0);
    let cache_dir = app
        .path()
        .app_cache_dir()
        .context("resolving app cache directory")?;
    let wav_path =
        audio_extract::extract_to_wav(input, &cache_dir).context("extracting audio via ffmpeg")?;
    emit_progress(&app, "extract_audio", 1.0);

    emit_progress(&app, "transcribe", 0.0);
    let transcribe_app = app.clone();
    let transcript: Transcript = match settings.stt_engine {
        SttEngineChoice::Local => {
            emit_progress(&app, "download_model", 0.0);
            let model = models::default_model();
            let model_app = app.clone();
            let model_path = models::ensure_model_downloaded(&model, move |frac| {
                emit_progress(&model_app, "download_model", frac);
            })
            .await
            .context("downloading whisper model")?;
            emit_progress(&app, "download_model", 1.0);

            let engine: Box<dyn SttEngine + Send> = Box::new(LocalWhisperEngine::new(model_path));
            run_stt_engine(engine, wav_path, transcribe_app).await?
        }
        SttEngineChoice::Groq => {
            let (key, provider) =
                stt_key_requirement(SttEngineChoice::Groq).expect("Groq needs a key");
            let api_key = require_api_key(key, provider)?;
            let engine: Box<dyn SttEngine + Send> =
                Box::new(OpenAiCompatibleWhisperEngine::groq(api_key));
            run_stt_engine(engine, wav_path, transcribe_app).await?
        }
        SttEngineChoice::OpenAi => {
            let (key, provider) =
                stt_key_requirement(SttEngineChoice::OpenAi).expect("OpenAI needs a key");
            let api_key = require_api_key(key, provider)?;
            let engine: Box<dyn SttEngine + Send> =
                Box::new(OpenAiCompatibleWhisperEngine::openai(api_key));
            run_stt_engine(engine, wav_path, transcribe_app).await?
        }
        SttEngineChoice::Deepgram => {
            let (key, provider) =
                stt_key_requirement(SttEngineChoice::Deepgram).expect("Deepgram needs a key");
            let api_key = require_api_key(key, provider)?;
            let engine: Box<dyn SttEngine + Send> = Box::new(DeepgramEngine::new(api_key));
            run_stt_engine(engine, wav_path, transcribe_app).await?
        }
        SttEngineChoice::AssemblyAi => {
            let (key, provider) =
                stt_key_requirement(SttEngineChoice::AssemblyAi).expect("AssemblyAI needs a key");
            let api_key = require_api_key(key, provider)?;
            let engine: Box<dyn SttEngine + Send> = Box::new(AssemblyAiEngine::new(api_key));
            run_stt_engine(engine, wav_path, transcribe_app).await?
        }
    };
    emit_progress(&app, "transcribe", 1.0);

    emit_progress(&app, "write_subtitles", 0.0);
    let cues = segmentation::build_cues(&transcript.segments);
    let srt_content = subtitle_writer::to_srt(&cues);

    let srt_path = srt_output_path(input);
    std::fs::write(&srt_path, &srt_content).context("writing .srt file")?;
    emit_progress(&app, "write_subtitles", 1.0);

    Ok(PipelineOutput {
        srt_path: srt_path.to_string_lossy().to_string(),
        srt_content,
        language: transcript.language,
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct TranslationOutput {
    pub srt_path: String,
    pub srt_content: String,
}

/// Translates an existing SRT's cue text (keeping timestamps), writing
/// `<source_name>.<target_lang>.srt` next to it.
pub async fn translate(
    app: AppHandle,
    srt_path: String,
    srt_content: String,
    source_lang: Option<String>,
    target_lang: String,
) -> Result<TranslationOutput> {
    let settings = config::load_settings().context("loading settings")?;

    let cues = subtitle_writer::parse_srt(&srt_content);
    anyhow::ensure!(!cues.is_empty(), "no subtitle cues to translate");
    let texts: Vec<String> = cues.iter().map(|c| c.text.clone()).collect();

    emit_progress(&app, "translate", 0.0);
    let translate_app = app.clone();
    let translated_texts: Vec<String> = match settings.translation_engine {
        TranslationEngineChoice::DeepL => {
            let (key, provider) = translation_key_requirement(TranslationEngineChoice::DeepL)
                .expect("DeepL needs a key");
            let api_key = require_api_key(key, provider)?;
            let engine: Box<dyn TranslationEngine + Send> =
                Box::new(CloudDeepLEngine::new(api_key));
            run_translation_engine(
                engine,
                texts,
                source_lang,
                target_lang.clone(),
                translate_app,
            )
            .await?
        }
        TranslationEngineChoice::OpenAi => {
            let (key, provider) = translation_key_requirement(TranslationEngineChoice::OpenAi)
                .expect("OpenAI needs a key");
            let api_key = require_api_key(key, provider)?;
            let engine: Box<dyn TranslationEngine + Send> =
                Box::new(OpenAiTranslateEngine::new(api_key));
            run_translation_engine(
                engine,
                texts,
                source_lang,
                target_lang.clone(),
                translate_app,
            )
            .await?
        }
        TranslationEngineChoice::Google => {
            let (key, provider) = translation_key_requirement(TranslationEngineChoice::Google)
                .expect("Google Translate needs a key");
            let api_key = require_api_key(key, provider)?;
            let engine: Box<dyn TranslationEngine + Send> =
                Box::new(GoogleTranslateEngine::new(api_key));
            run_translation_engine(
                engine,
                texts,
                source_lang,
                target_lang.clone(),
                translate_app,
            )
            .await?
        }
        TranslationEngineChoice::Azure => {
            let (key, provider) = translation_key_requirement(TranslationEngineChoice::Azure)
                .expect("Azure Translator needs a key");
            let api_key = require_api_key(key, provider)?;
            anyhow::ensure!(
                !settings.azure_translator_region.is_empty(),
                "Azure Translator region not configured — add it in Settings"
            );
            let engine: Box<dyn TranslationEngine + Send> = Box::new(AzureTranslateEngine::new(
                api_key,
                settings.azure_translator_region.clone(),
            ));
            run_translation_engine(
                engine,
                texts,
                source_lang,
                target_lang.clone(),
                translate_app,
            )
            .await?
        }
        TranslationEngineChoice::Local => {
            emit_progress(&app, "download_translation_model", 0.0);
            let model_app = app.clone();
            let files = translate::nllb_models::ensure_nllb_downloaded(move |frac| {
                emit_progress(&model_app, "download_translation_model", frac);
            })
            .await
            .context("downloading local translation model")?;
            emit_progress(&app, "download_translation_model", 1.0);

            let target_lang_inner = target_lang.clone();
            tauri::async_runtime::spawn_blocking(move || -> Result<Vec<String>> {
                let engine =
                    LocalNllbEngine::load(&files.encoder, &files.decoder, &files.tokenizer)
                        .context("loading local translation model")?;
                engine.translate(
                    &texts,
                    source_lang.as_deref(),
                    &target_lang_inner,
                    Box::new(move |frac| emit_progress(&translate_app, "translate", frac)),
                )
            })
            .await
            .context("translation task")??
        }
        TranslationEngineChoice::None => {
            anyhow::bail!("translation is disabled — enable it in Settings first");
        }
    };
    emit_progress(&app, "translate", 1.0);

    let translated_cues = zip_translations(&cues, translated_texts)?;
    let translated_srt = subtitle_writer::to_srt(&translated_cues);

    let out_path = translated_output_path(Path::new(&srt_path), &target_lang);
    std::fs::write(&out_path, &translated_srt).context("writing translated .srt file")?;

    Ok(TranslationOutput {
        srt_path: out_path.to_string_lossy().to_string(),
        srt_content: translated_srt,
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

    // ── clés d'API exigées ───────────────────────────────────────────────────

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
    fn neither_the_local_nor_the_disabled_translation_engine_needs_a_key() {
        assert!(translation_key_requirement(TranslationEngineChoice::Local).is_none());
        assert!(translation_key_requirement(TranslationEngineChoice::None).is_none());
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
        // Une seule clé OpenAI saisie doit servir aux deux usages.
        assert_eq!(
            stt_key_requirement(SttEngineChoice::OpenAi).unwrap().0,
            translation_key_requirement(TranslationEngineChoice::OpenAi)
                .unwrap()
                .0
        );
    }

    // ── chemins de sortie ────────────────────────────────────────────────────

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

    // ── réassociation des traductions ────────────────────────────────────────

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
        // Sinon tous les sous-titres suivants seraient décalés.
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

    #[test]
    fn an_empty_set_of_cues_zips_to_nothing() {
        assert!(zip_translations(&[], vec![]).unwrap().is_empty());
    }
}
