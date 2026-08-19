use anyhow::{Context, Result};
use serde::Deserialize;

use super::TranslationEngine;

#[derive(Deserialize)]
struct DeepLTranslation {
    text: String,
}

#[derive(Deserialize)]
struct DeepLResponse {
    translations: Vec<DeepLTranslation>,
}

pub struct CloudDeepLEngine {
    api_key: String,
}

impl CloudDeepLEngine {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
        }
    }

    /// DeepL uses separate hosts for free-tier and paid keys (free keys end in ":fx").
    fn endpoint(&self) -> &'static str {
        if self.api_key.ends_with(":fx") {
            "https://api-free.deepl.com/v2/translate"
        } else {
            "https://api.deepl.com/v2/translate"
        }
    }
}

/// DeepL requires a region variant for some target languages (plain source codes work fine).
fn deepl_target_lang(code: &str) -> String {
    match code.to_ascii_lowercase().as_str() {
        "en" => "EN-US".to_string(),
        "pt" => "PT-PT".to_string(),
        other => other.to_ascii_uppercase(),
    }
}

/// Paramètres de formulaire envoyés à DeepL : une entrée `text` par ligne, plus la
/// langue cible et, si connue, la langue source. Séparé de `translate` pour être
/// vérifiable sans appel réseau.
fn form_params<'a>(
    texts: &'a [String],
    target_lang: &'a str,
    source_lang: Option<&'a str>,
) -> Vec<(&'a str, &'a str)> {
    let mut params: Vec<(&str, &str)> = texts.iter().map(|t| ("text", t.as_str())).collect();
    params.push(("target_lang", target_lang));
    if let Some(src) = source_lang {
        params.push(("source_lang", src));
    }
    params
}

fn auth_header(api_key: &str) -> String {
    format!("DeepL-Auth-Key {api_key}")
}

fn extract_texts(parsed: DeepLResponse) -> Vec<String> {
    parsed.translations.into_iter().map(|t| t.text).collect()
}

impl TranslationEngine for CloudDeepLEngine {
    fn translate(
        &self,
        texts: &[String],
        source_lang: Option<&str>,
        target_lang: &str,
        mut on_progress: Box<dyn FnMut(f32) + Send>,
    ) -> Result<Vec<String>> {
        on_progress(0.1);

        let target_lang = deepl_target_lang(target_lang);
        let source_lang = source_lang.map(|s| s.to_ascii_uppercase());

        let params = form_params(texts, &target_lang, source_lang.as_deref());

        let client = reqwest::blocking::Client::new();
        let response = client
            .post(self.endpoint())
            .header("Authorization", auth_header(&self.api_key))
            .form(&params)
            .send()
            .context("sending request to DeepL API")?
            .error_for_status()
            .context("DeepL API returned an error")?;

        on_progress(0.8);

        let parsed: DeepLResponse = response.json().context("parsing DeepL API response")?;

        on_progress(1.0);

        Ok(extract_texts(parsed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_tier_keys_use_the_free_host() {
        let engine = CloudDeepLEngine::new("xxxxxxxx-xxxx-xxxx:fx");
        assert_eq!(engine.endpoint(), "https://api-free.deepl.com/v2/translate");
    }

    #[test]
    fn paid_keys_use_the_paid_host() {
        let engine = CloudDeepLEngine::new("xxxxxxxx-xxxx-xxxx");
        assert_eq!(engine.endpoint(), "https://api.deepl.com/v2/translate");
    }

    #[test]
    fn english_and_portuguese_need_a_region_variant() {
        assert_eq!(deepl_target_lang("en"), "EN-US");
        assert_eq!(deepl_target_lang("pt"), "PT-PT");
    }

    #[test]
    fn other_targets_are_simply_upper_cased() {
        assert_eq!(deepl_target_lang("fr"), "FR");
        assert_eq!(deepl_target_lang("de"), "DE");
    }

    #[test]
    fn the_incoming_code_case_does_not_matter() {
        assert_eq!(deepl_target_lang("EN"), "EN-US");
        assert_eq!(deepl_target_lang("Fr"), "FR");
    }

    #[test]
    fn parses_a_translation_response() {
        let parsed: DeepLResponse =
            serde_json::from_str(r#"{"translations":[{"text":"Bonjour"},{"text":"Monde"}]}"#)
                .unwrap();
        let texts: Vec<_> = parsed.translations.into_iter().map(|t| t.text).collect();
        assert_eq!(texts, vec!["Bonjour", "Monde"]);
    }

    #[test]
    fn parses_an_empty_translation_response() {
        let parsed: DeepLResponse = serde_json::from_str(r#"{"translations":[]}"#).unwrap();
        assert!(parsed.translations.is_empty());
    }

    fn lines() -> Vec<String> {
        vec!["Hello".to_string(), "World".to_string()]
    }

    #[test]
    fn each_line_becomes_its_own_text_parameter() {
        let input = lines();
        let params = form_params(&input, "FR", None);

        let texts: Vec<_> = params
            .iter()
            .filter(|(k, _)| *k == "text")
            .map(|(_, v)| *v)
            .collect();
        assert_eq!(texts, vec!["Hello", "World"]);
    }

    #[test]
    fn the_target_language_is_always_sent() {
        let input = lines();
        let params = form_params(&input, "FR", None);

        assert!(params.contains(&("target_lang", "FR")));
    }

    #[test]
    fn the_source_language_is_omitted_when_unknown() {
        let input = lines();
        let params = form_params(&input, "FR", None);

        assert!(!params.iter().any(|(k, _)| *k == "source_lang"));
    }

    #[test]
    fn the_source_language_is_sent_when_known() {
        let input = lines();
        let params = form_params(&input, "FR", Some("EN"));

        assert!(params.contains(&("source_lang", "EN")));
    }

    #[test]
    fn an_empty_input_still_carries_the_target_language() {
        let params = form_params(&[], "DE", None);

        assert_eq!(params, vec![("target_lang", "DE")]);
    }

    #[test]
    fn the_key_goes_in_a_deepl_auth_header() {
        assert_eq!(auth_header("abc:fx"), "DeepL-Auth-Key abc:fx");
    }

    #[test]
    fn the_response_yields_the_translations_in_order() {
        let parsed: DeepLResponse =
            serde_json::from_str(r#"{"translations":[{"text":"Bonjour"},{"text":"Monde"}]}"#)
                .unwrap();

        assert_eq!(extract_texts(parsed), vec!["Bonjour", "Monde"]);
    }

    #[test]
    fn an_empty_response_yields_no_text() {
        let parsed: DeepLResponse = serde_json::from_str(r#"{"translations":[]}"#).unwrap();

        assert!(extract_texts(parsed).is_empty());
    }
}
