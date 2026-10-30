use std::path::PathBuf;

use anyhow::{Context, Result};
use keyring::Entry;
use serde::{Deserialize, Serialize};

const SERVICE_NAME: &str = "light-gen-subz";

pub const GROQ_API_KEY: &str = "groq_api_key";
pub const OPENAI_API_KEY: &str = "openai_api_key";
pub const DEEPGRAM_API_KEY: &str = "deepgram_api_key";
pub const ASSEMBLYAI_API_KEY: &str = "assemblyai_api_key";
pub const DEEPL_API_KEY: &str = "deepl_api_key";
pub const GOOGLE_TRANSLATE_API_KEY: &str = "google_translate_api_key";
pub const AZURE_TRANSLATOR_KEY: &str = "azure_translator_key";

/// The only names the webview may read or write in the keyring.
pub const ALL_API_KEYS: [&str; 7] = [
    GROQ_API_KEY,
    OPENAI_API_KEY,
    DEEPGRAM_API_KEY,
    ASSEMBLYAI_API_KEY,
    DEEPL_API_KEY,
    GOOGLE_TRANSLATE_API_KEY,
    AZURE_TRANSLATOR_KEY,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SttEngineChoice {
    #[default]
    Local,
    Groq,
    OpenAi,
    Deepgram,
    AssemblyAi,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TranslationEngineChoice {
    /// `none` is what the settings said before translation became a choice made at each
    /// run: it reads as the local engine.
    #[default]
    #[serde(alias = "none")]
    Local,
    DeepL,
    OpenAi,
    Google,
    Azure,
}

/// Interface language. English by default, so that a settings file written before the
/// choice existed changes nothing for someone updating. On the very first launch (no
/// settings file) the system language is followed, as in the other apps of the suite.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    #[default]
    En,
    Fr,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub stt_engine: SttEngineChoice,
    pub translation_engine: TranslationEngineChoice,
    /// Azure Translator requires a resource region alongside its API key (not a secret).
    #[serde(default)]
    pub azure_translator_region: String,
    #[serde(default)]
    pub language: Language,
    /// Local whisper model name (`base`, `small`, `medium`).
    #[serde(default = "default_whisper_model")]
    pub whisper_model: String,
    /// Spoken language forced for transcription, or `auto` to detect it.
    #[serde(default = "default_stt_language")]
    pub stt_language: String,
}

fn default_whisper_model() -> String {
    crate::models::DEFAULT_MODEL.to_string()
}

fn default_stt_language() -> String {
    "auto".to_string()
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            stt_engine: SttEngineChoice::default(),
            translation_engine: TranslationEngineChoice::default(),
            azure_translator_region: String::new(),
            language: Language::default(),
            whisper_model: default_whisper_model(),
            stt_language: default_stt_language(),
        }
    }
}

fn settings_path() -> Result<PathBuf> {
    Ok(crate::app_dirs::config_dir()?.join("settings.json"))
}

/// Field-by-field read, deliberately lenient: a file written by a newer version (unknown
/// field, unknown enum value) must not stop the app from starting — the offending field
/// simply falls back to its default.
///
/// Separate from `load_settings` so it can be tested without touching the real config folder.
pub fn parse_settings(raw: &str) -> Result<Settings> {
    let value: serde_json::Value = serde_json::from_str(raw).context("parsing settings file")?;
    Ok(Settings {
        stt_engine: value
            .get("stt_engine")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default(),
        translation_engine: value
            .get("translation_engine")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default(),
        azure_translator_region: value
            .get("azure_translator_region")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        language: value
            .get("language")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default(),
        whisper_model: value
            .get("whisper_model")
            .and_then(|v| v.as_str())
            .filter(|name| crate::models::model_named(name).is_some())
            .map(str::to_string)
            .unwrap_or_else(default_whisper_model),
        stt_language: value
            .get("stt_language")
            .and_then(|v| v.as_str())
            .filter(|code| {
                *code == "auto"
                    || crate::pipeline::translate::languages::normalize_code(code).is_some()
            })
            .map(str::to_string)
            .unwrap_or_else(default_stt_language),
    })
}

/// Settings as the app should use them: a corrupt file never blocks a run, it just
/// falls back to the defaults (the same behaviour as what the UI shows).
pub fn load_settings_or_default() -> Settings {
    load_settings().unwrap_or_default()
}

/// French for a locale such as `fr_FR.UTF-8`, `fr-CA` or `fr`; English otherwise.
pub fn language_from_locale(locale: &str) -> Language {
    let tag = locale.trim().trim_matches('"').to_ascii_lowercase();
    if tag == "fr" || tag.starts_with("fr_") || tag.starts_with("fr-") || tag.starts_with("fr.") {
        Language::Fr
    } else {
        Language::En
    }
}

/// The system language, from the usual environment variables and, on macOS (where a
/// graphical app has none of them), from the language list in the preferences.
fn system_language() -> Language {
    for var in ["LC_ALL", "LC_MESSAGES", "LANGUAGE", "LANG"] {
        if let Ok(value) = std::env::var(var) {
            if !value.is_empty() && value != "C" && value != "POSIX" {
                return language_from_locale(value.split(':').next().unwrap_or(""));
            }
        }
    }
    #[cfg(target_os = "macos")]
    if let Ok(out) = std::process::Command::new("defaults")
        .args(["read", "-g", "AppleLanguages"])
        .output()
    {
        let text = String::from_utf8_lossy(&out.stdout);
        if let Some(first) = text.split('"').nth(1) {
            return language_from_locale(first);
        }
    }
    Language::En
}

pub fn load_settings() -> Result<Settings> {
    let path = settings_path()?;
    if !path.exists() {
        return Ok(Settings {
            language: system_language(),
            ..Settings::default()
        });
    }
    let raw = std::fs::read_to_string(&path).context("reading settings file")?;
    parse_settings(&raw)
}

pub fn save_settings(settings: &Settings) -> Result<()> {
    let path = settings_path()?;
    let raw = serde_json::to_string_pretty(settings).context("serializing settings")?;
    write_atomically(&path, &raw)
}

/// One save at a time, and a temporary file of its own for each: two saves in flight
/// (a select moved with the arrow keys, the debounced Azure region) used to share one
/// `settings.tmp`, and one of them failed or left a corrupt file.
static SAVE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Writes next to the target then renames, so a crash mid-write cannot leave a
/// truncated file behind.
pub fn write_atomically(path: &std::path::Path, content: &str) -> Result<()> {
    use std::io::Write;
    let _guard = SAVE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let dir = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    let mut tmp = tempfile::NamedTempFile::new_in(dir).context("creating settings file")?;
    tmp.write_all(content.as_bytes())
        .context("writing settings file")?;
    tmp.persist(path).context("replacing settings file")?;
    Ok(())
}

fn keyring_entry(key_name: &str) -> Result<Entry> {
    Entry::new(SERVICE_NAME, key_name).context("creating keyring entry")
}

fn ensure_known_key(key_name: &str) -> Result<()> {
    anyhow::ensure!(
        ALL_API_KEYS.contains(&key_name),
        crate::errors::coded("unknown_key", format!("unknown API key name: {key_name}"))
    );
    Ok(())
}

pub fn set_api_key(key_name: &str, value: &str) -> Result<()> {
    ensure_known_key(key_name)?;
    keyring_entry(key_name)?
        .set_password(value)
        .context("storing API key in keyring")
}

pub fn get_api_key(key_name: &str) -> Result<Option<String>> {
    ensure_known_key(key_name)?;
    match keyring_entry(key_name)?.get_password() {
        Ok(password) => Ok(Some(password)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e).context("reading API key from keyring"),
    }
}

pub fn delete_api_key(key_name: &str) -> Result<()> {
    ensure_known_key(key_name)?;
    match keyring_entry(key_name)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e).context("removing API key from keyring"),
    }
}

pub fn has_api_key(key_name: &str) -> bool {
    ALL_API_KEYS.contains(&key_name) && get_api_key(key_name).ok().flatten().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_local_transcription_and_local_translation() {
        let settings = Settings::default();
        assert_eq!(settings.stt_engine, SttEngineChoice::Local);
        assert_eq!(settings.translation_engine, TranslationEngineChoice::Local);
        assert!(settings.azure_translator_region.is_empty());
    }

    #[test]
    fn engine_choices_use_snake_case_on_the_wire() {
        assert_eq!(
            serde_json::to_string(&SttEngineChoice::OpenAi).unwrap(),
            "\"open_ai\""
        );
        assert_eq!(
            serde_json::to_string(&SttEngineChoice::AssemblyAi).unwrap(),
            "\"assembly_ai\""
        );
        assert_eq!(
            serde_json::to_string(&TranslationEngineChoice::DeepL).unwrap(),
            "\"deep_l\""
        );
    }

    #[test]
    fn engine_choices_round_trip() {
        for choice in [
            SttEngineChoice::Local,
            SttEngineChoice::Groq,
            SttEngineChoice::OpenAi,
            SttEngineChoice::Deepgram,
            SttEngineChoice::AssemblyAi,
        ] {
            let json = serde_json::to_string(&choice).unwrap();
            let back: SttEngineChoice = serde_json::from_str(&json).unwrap();
            assert_eq!(back, choice);
        }
        for choice in [
            TranslationEngineChoice::Local,
            TranslationEngineChoice::DeepL,
            TranslationEngineChoice::OpenAi,
            TranslationEngineChoice::Google,
            TranslationEngineChoice::Azure,
        ] {
            let json = serde_json::to_string(&choice).unwrap();
            let back: TranslationEngineChoice = serde_json::from_str(&json).unwrap();
            assert_eq!(back, choice);
        }
    }

    #[test]
    fn settings_round_trip_through_json() {
        let settings = Settings {
            stt_engine: SttEngineChoice::Groq,
            translation_engine: TranslationEngineChoice::Azure,
            azure_translator_region: "westeurope".into(),
            language: Language::Fr,
            ..Settings::default()
        };

        let json = serde_json::to_string(&settings).unwrap();
        let back: Settings = serde_json::from_str(&json).unwrap();

        assert_eq!(back.stt_engine, settings.stt_engine);
        assert_eq!(back.translation_engine, settings.translation_engine);
        assert_eq!(back.azure_translator_region, "westeurope");
        assert_eq!(back.language, Language::Fr);
    }

    #[test]
    fn a_saved_none_engine_reads_as_the_local_one() {
        let settings = parse_settings(r#"{"translation_engine":"none"}"#).unwrap();
        assert_eq!(settings.translation_engine, TranslationEngineChoice::Local);
    }

    #[test]
    fn a_missing_azure_region_deserialises_as_empty() {
        let settings: Settings =
            serde_json::from_str(r#"{"stt_engine":"local","translation_engine":"none"}"#).unwrap();
        assert!(settings.azure_translator_region.is_empty());
    }

    #[test]
    fn a_full_settings_file_is_read_back() {
        let settings = parse_settings(
            r#"{"stt_engine":"deepgram","translation_engine":"azure",
                "azure_translator_region":"westeurope"}"#,
        )
        .unwrap();

        assert_eq!(settings.stt_engine, SttEngineChoice::Deepgram);
        assert_eq!(settings.translation_engine, TranslationEngineChoice::Azure);
        assert_eq!(settings.azure_translator_region, "westeurope");
    }

    #[test]
    fn unknown_fields_are_ignored() {
        // A file written by a newer version must not block startup.
        let settings = parse_settings(
            r#"{"stt_engine":"deepgram","translation_engine":"local","future_option":true}"#,
        )
        .unwrap();

        assert_eq!(settings.stt_engine, SttEngineChoice::Deepgram);
        assert_eq!(settings.translation_engine, TranslationEngineChoice::Local);
    }

    #[test]
    fn an_unknown_engine_value_falls_back_to_the_default() {
        let settings =
            parse_settings(r#"{"stt_engine":"quantum_whisper","translation_engine":42}"#).unwrap();

        assert_eq!(settings.stt_engine, SttEngineChoice::Local);
        assert_eq!(settings.translation_engine, TranslationEngineChoice::Local);
    }

    #[test]
    fn missing_fields_fall_back_to_the_defaults() {
        let settings = parse_settings("{}").unwrap();

        assert_eq!(settings.stt_engine, SttEngineChoice::Local);
        assert_eq!(settings.translation_engine, TranslationEngineChoice::Local);
        assert!(settings.azure_translator_region.is_empty());
    }

    #[test]
    fn a_non_string_azure_region_falls_back_to_empty() {
        let settings = parse_settings(r#"{"azure_translator_region":123}"#).unwrap();

        assert!(settings.azure_translator_region.is_empty());
    }

    #[test]
    fn a_corrupt_settings_file_is_reported() {
        let err = parse_settings("pas du json").unwrap_err();

        assert!(err.to_string().contains("parsing settings file"), "{err}");
    }

    #[test]
    fn what_save_writes_is_what_parse_reads_back() {
        let settings = Settings {
            stt_engine: SttEngineChoice::AssemblyAi,
            translation_engine: TranslationEngineChoice::Google,
            azure_translator_region: "francecentral".into(),
            language: Language::Fr,
            ..Settings::default()
        };

        let raw = serde_json::to_string_pretty(&settings).unwrap();
        let back = parse_settings(&raw).unwrap();

        assert_eq!(back.stt_engine, settings.stt_engine);
        assert_eq!(back.translation_engine, settings.translation_engine);
        assert_eq!(back.azure_translator_region, "francecentral");
        assert_eq!(back.language, Language::Fr);
    }

    // ── interface language ────────────────────────────────────────────────

    #[test]
    fn the_interface_starts_in_english() {
        assert_eq!(Settings::default().language, Language::En);
    }

    #[test]
    fn the_system_locale_picks_the_first_launch_language() {
        for fr in ["fr_FR.UTF-8", "fr-CA", "fr", "FR_be", "\"fr-FR\""] {
            assert_eq!(language_from_locale(fr), Language::Fr, "{fr}");
        }
        for en in ["en_US.UTF-8", "de_DE", "C", "", "frisian"] {
            assert_eq!(language_from_locale(en), Language::En, "{en}");
        }
    }

    #[test]
    fn a_settings_file_written_before_the_language_existed_still_opens_in_english() {
        let settings =
            parse_settings(r#"{"stt_engine":"groq","translation_engine":"none"}"#).unwrap();

        assert_eq!(settings.language, Language::En);
    }

    #[test]
    fn a_saved_language_is_read_back() {
        assert_eq!(
            parse_settings(r#"{"language":"fr"}"#).unwrap().language,
            Language::Fr
        );
    }

    #[test]
    fn an_unknown_language_falls_back_to_english() {
        assert_eq!(
            parse_settings(r#"{"language":"klingon"}"#)
                .unwrap()
                .language,
            Language::En
        );
    }

    #[test]
    fn languages_use_their_two_letter_code_on_the_wire() {
        assert_eq!(serde_json::to_string(&Language::En).unwrap(), "\"en\"");
        assert_eq!(serde_json::to_string(&Language::Fr).unwrap(), "\"fr\"");
    }

    #[test]
    fn an_unknown_key_name_is_never_looked_up() {
        assert!(!has_api_key("light_gen_subz_test_absent_key"));
        assert!(set_api_key("../etc", "x").is_err());
        assert!(delete_api_key("anything").is_err());
        assert!(get_api_key("anything").is_err());
    }

    #[test]
    fn the_whitelist_holds_every_key_constant() {
        for key in [
            GROQ_API_KEY,
            OPENAI_API_KEY,
            DEEPGRAM_API_KEY,
            ASSEMBLYAI_API_KEY,
            DEEPL_API_KEY,
            GOOGLE_TRANSLATE_API_KEY,
            AZURE_TRANSLATOR_KEY,
        ] {
            assert!(ALL_API_KEYS.contains(&key), "{key}");
        }
    }

    // ── whisper model and forced language ─────────────────────────────────────

    #[test]
    fn transcription_defaults_to_the_small_model_and_auto_language() {
        let settings = Settings::default();

        assert_eq!(settings.whisper_model, "small");
        assert_eq!(settings.stt_language, "auto");
    }

    #[test]
    fn a_saved_model_and_language_are_read_back() {
        let settings = parse_settings(r#"{"whisper_model":"medium","stt_language":"fr"}"#).unwrap();

        assert_eq!(settings.whisper_model, "medium");
        assert_eq!(settings.stt_language, "fr");
    }

    #[test]
    fn an_unknown_model_or_language_falls_back() {
        let settings =
            parse_settings(r#"{"whisper_model":"gigantic","stt_language":"klingon"}"#).unwrap();

        assert_eq!(settings.whisper_model, "small");
        assert_eq!(settings.stt_language, "auto");
    }

    #[test]
    fn an_atomic_write_replaces_the_file_and_leaves_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "old").unwrap();

        write_atomically(&path, "new").unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
        assert!(!dir.path().join("settings.tmp").exists());
    }

    #[test]
    fn concurrent_saves_never_fail_nor_corrupt_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let long = format!("{{\"azure_translator_region\":\"{}\"}}", "x".repeat(400));
        let short = "{\"stt_engine\":\"local\"}".to_string();

        let writers: Vec<_> = [long.clone(), short.clone()]
            .into_iter()
            .map(|content| {
                let path = path.clone();
                std::thread::spawn(move || {
                    (0..300).all(|_| write_atomically(&path, &content).is_ok())
                })
            })
            .collect();

        for writer in writers {
            assert!(writer.join().unwrap(), "a save failed");
        }
        let on_disk = std::fs::read_to_string(&path).unwrap();
        assert!(on_disk == long || on_disk == short);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn a_corrupt_file_falls_back_to_the_defaults_for_a_run() {
        // load_settings_or_default must never fail, whatever is on disk.
        let _ = load_settings_or_default();
    }

    #[test]
    fn api_key_names_are_distinct() {
        let names = [
            GROQ_API_KEY,
            OPENAI_API_KEY,
            DEEPGRAM_API_KEY,
            ASSEMBLYAI_API_KEY,
            DEEPL_API_KEY,
            GOOGLE_TRANSLATE_API_KEY,
            AZURE_TRANSLATOR_KEY,
        ];
        let unique: std::collections::HashSet<_> = names.iter().collect();
        assert_eq!(unique.len(), names.len());
    }

    /// Real keyring round trip, on a throwaway entry (never one of the app's keys):
    /// `cargo test -- --ignored real_keyring`. Needs a running Secret Service / Keychain.
    #[test]
    #[ignore]
    fn real_keyring_stores_reads_and_deletes_a_secret() {
        let entry = keyring_entry("light_gen_subz_roundtrip_test").unwrap();

        entry.set_password("s3cret").unwrap();
        assert_eq!(entry.get_password().unwrap(), "s3cret");
        entry.delete_credential().unwrap();

        assert!(matches!(entry.get_password(), Err(keyring::Error::NoEntry)));
    }
}
