use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::json;

use super::TranslationEngine;

const ENDPOINT: &str = "https://api.cognitive.microsofttranslator.com/translate";

pub struct AzureTranslateEngine {
    api_key: String,
    region: String,
}

impl AzureTranslateEngine {
    pub fn new(api_key: impl Into<String>, region: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            region: region.into(),
        }
    }
}

#[derive(Deserialize)]
struct Translation {
    text: String,
}

#[derive(Deserialize)]
struct TranslationResult {
    translations: Vec<Translation>,
}

/// Paramètres d'URL : version d'API figée, langue cible, et langue source si connue.
fn query_params<'a>(target_lang: &'a str, source_lang: Option<&'a str>) -> Vec<(&'a str, &'a str)> {
    let mut query = vec![("api-version", "3.0"), ("to", target_lang)];
    if let Some(src) = source_lang {
        query.push(("from", src));
    }
    query
}

/// Azure attend un tableau d'objets `{"Text": ...}`.
fn request_body(texts: &[String]) -> Vec<serde_json::Value> {
    texts.iter().map(|t| json!({ "Text": t })).collect()
}

/// Azure renvoie une liste de résultats, chacun pouvant porter plusieurs traductions
/// (une par langue cible demandée). On n'en demande qu'une : on prend la première.
fn extract_texts(parsed: Vec<TranslationResult>) -> Vec<String> {
    parsed
        .into_iter()
        .filter_map(|r| r.translations.into_iter().next().map(|t| t.text))
        .collect()
}

impl TranslationEngine for AzureTranslateEngine {
    fn translate(
        &self,
        texts: &[String],
        source_lang: Option<&str>,
        target_lang: &str,
        mut on_progress: Box<dyn FnMut(f32) + Send>,
    ) -> Result<Vec<String>> {
        on_progress(0.1);

        let query = query_params(target_lang, source_lang);
        let body = request_body(texts);

        let client = reqwest::blocking::Client::new();
        let response = client
            .post(ENDPOINT)
            .query(&query)
            .header("Ocp-Apim-Subscription-Key", &self.api_key)
            .header("Ocp-Apim-Subscription-Region", &self.region)
            .json(&body)
            .send()
            .context("sending request to Azure Translator API")?
            .error_for_status()
            .context("Azure Translator API returned an error")?;

        on_progress(0.8);

        let parsed: Vec<TranslationResult> = response
            .json()
            .context("parsing Azure Translator API response")?;

        on_progress(1.0);

        Ok(extract_texts(parsed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_one_result_per_input_text() {
        let parsed: Vec<TranslationResult> = serde_json::from_str(
            r#"[{"translations":[{"text":"Bonjour"}]},{"translations":[{"text":"Monde"}]}]"#,
        )
        .unwrap();

        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].translations[0].text, "Bonjour");
        assert_eq!(parsed[1].translations[0].text, "Monde");
    }

    #[test]
    fn keeps_the_region_it_was_built_with() {
        let engine = AzureTranslateEngine::new("key", "westeurope");
        assert_eq!(engine.region, "westeurope");
    }

    #[test]
    fn the_api_version_is_pinned() {
        assert!(query_params("fr", None).contains(&("api-version", "3.0")));
    }

    #[test]
    fn the_target_language_is_always_sent() {
        assert!(query_params("fr", None).contains(&("to", "fr")));
    }

    #[test]
    fn the_source_language_is_omitted_when_unknown() {
        assert!(!query_params("fr", None).iter().any(|(k, _)| *k == "from"));
    }

    #[test]
    fn the_source_language_is_sent_when_known() {
        assert!(query_params("fr", Some("en")).contains(&("from", "en")));
    }

    #[test]
    fn each_line_becomes_a_text_object() {
        let body = request_body(&["Hello".to_string(), "World".to_string()]);

        assert_eq!(
            body,
            vec![json!({"Text": "Hello"}), json!({"Text": "World"})]
        );
    }

    #[test]
    fn an_empty_input_sends_an_empty_array() {
        assert!(request_body(&[]).is_empty());
    }

    #[test]
    fn the_first_translation_of_each_result_is_kept() {
        let parsed: Vec<TranslationResult> = serde_json::from_str(
            r#"[{"translations":[{"text":"Bonjour"},{"text":"Hola"}]},
                {"translations":[{"text":"Monde"}]}]"#,
        )
        .unwrap();

        assert_eq!(extract_texts(parsed), vec!["Bonjour", "Monde"]);
    }

    #[test]
    fn a_result_without_any_translation_is_dropped() {
        let parsed: Vec<TranslationResult> =
            serde_json::from_str(r#"[{"translations":[]},{"translations":[{"text":"Monde"}]}]"#)
                .unwrap();

        assert_eq!(extract_texts(parsed), vec!["Monde"]);
    }

    #[test]
    fn an_empty_response_yields_no_text() {
        assert!(extract_texts(vec![]).is_empty());
    }
}
