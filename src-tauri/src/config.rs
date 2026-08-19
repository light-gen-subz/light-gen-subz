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
    #[default]
    None,
    Local,
    DeepL,
    OpenAi,
    Google,
    Azure,
}

/// Langue de l'interface. Anglais par défaut : c'est la langue dans laquelle l'app
/// était écrite avant que le choix existe, donc le défaut ne change rien pour
/// quelqu'un qui met à jour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    #[default]
    En,
    Fr,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Settings {
    pub stt_engine: SttEngineChoice,
    pub translation_engine: TranslationEngineChoice,
    /// Azure Translator requires a resource region alongside its API key (not a secret).
    #[serde(default)]
    pub azure_translator_region: String,
    #[serde(default)]
    pub language: Language,
}

fn settings_path() -> Result<PathBuf> {
    Ok(crate::app_dirs::config_dir()?.join("settings.json"))
}

/// Lecture champ par champ, volontairement indulgente : un fichier écrit par une version
/// plus récente (champ inconnu, valeur d'enum inconnue) ne doit pas empêcher l'app de
/// démarrer — le champ fautif retombe simplement sur son défaut.
///
/// Séparé de `load_settings` pour être testable sans toucher au dossier de config réel.
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
    })
}

pub fn load_settings() -> Result<Settings> {
    let path = settings_path()?;
    if !path.exists() {
        return Ok(Settings::default());
    }
    let raw = std::fs::read_to_string(&path).context("reading settings file")?;
    parse_settings(&raw)
}

pub fn save_settings(settings: &Settings) -> Result<()> {
    let path = settings_path()?;
    let raw = serde_json::to_string_pretty(settings).context("serializing settings")?;
    std::fs::write(path, raw).context("writing settings file")
}

fn keyring_entry(key_name: &str) -> Result<Entry> {
    Entry::new(SERVICE_NAME, key_name).context("creating keyring entry")
}

pub fn set_api_key(key_name: &str, value: &str) -> Result<()> {
    keyring_entry(key_name)?
        .set_password(value)
        .context("storing API key in keyring")
}

pub fn get_api_key(key_name: &str) -> Result<Option<String>> {
    match keyring_entry(key_name)?.get_password() {
        Ok(password) => Ok(Some(password)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e).context("reading API key from keyring"),
    }
}

pub fn has_api_key(key_name: &str) -> bool {
    get_api_key(key_name).ok().flatten().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_local_transcription_and_no_translation() {
        let settings = Settings::default();
        assert_eq!(settings.stt_engine, SttEngineChoice::Local);
        assert_eq!(settings.translation_engine, TranslationEngineChoice::None);
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
            TranslationEngineChoice::None,
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
        };

        let json = serde_json::to_string(&settings).unwrap();
        let back: Settings = serde_json::from_str(&json).unwrap();

        assert_eq!(back.stt_engine, settings.stt_engine);
        assert_eq!(back.translation_engine, settings.translation_engine);
        assert_eq!(back.azure_translator_region, "westeurope");
        assert_eq!(back.language, Language::Fr);
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
        // Un fichier écrit par une version plus récente ne doit pas bloquer le démarrage.
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
        assert_eq!(settings.translation_engine, TranslationEngineChoice::None);
    }

    #[test]
    fn missing_fields_fall_back_to_the_defaults() {
        let settings = parse_settings("{}").unwrap();

        assert_eq!(settings.stt_engine, SttEngineChoice::Local);
        assert_eq!(settings.translation_engine, TranslationEngineChoice::None);
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
        };

        let raw = serde_json::to_string_pretty(&settings).unwrap();
        let back = parse_settings(&raw).unwrap();

        assert_eq!(back.stt_engine, settings.stt_engine);
        assert_eq!(back.translation_engine, settings.translation_engine);
        assert_eq!(back.azure_translator_region, "francecentral");
        assert_eq!(back.language, Language::Fr);
    }

    // ── langue de l'interface ────────────────────────────────────────────────

    #[test]
    fn the_interface_starts_in_english() {
        assert_eq!(Settings::default().language, Language::En);
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
    fn an_absent_key_is_reported_as_missing() {
        // Nom improbable : le trousseau ne peut pas le contenir. Selon la machine, le
        // trousseau peut être indisponible — dans les deux cas la réponse est « non ».
        assert!(!has_api_key("light_gen_subz_test_absent_key"));
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
}
