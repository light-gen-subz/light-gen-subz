use anyhow::Context;
use anyhow::Result;
use serde::Deserialize;
use serde_json::json;

use super::TranslationEngine;

const ENDPOINT: &str = "https://translation.googleapis.com/language/translate/v2";

pub struct GoogleTranslateEngine {
    api_key: String,
}

impl GoogleTranslateEngine {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
        }
    }
}

#[derive(Deserialize)]
struct TranslationEntry {
    #[serde(rename = "translatedText")]
    translated_text: String,
}

#[derive(Deserialize)]
struct TranslateData {
    translations: Vec<TranslationEntry>,
}

#[derive(Deserialize)]
struct GoogleResponse {
    data: TranslateData,
}

/// Corps JSON envoyé à Google Translate. Séparé de `translate` pour être vérifiable
/// sans appel réseau.
fn request_body(
    texts: &[String],
    target_lang: &str,
    source_lang: Option<&str>,
) -> serde_json::Value {
    let mut body = json!({
        "q": texts,
        "target": target_lang,
        "format": "text",
    });
    if let Some(src) = source_lang {
        body["source"] = json!(src);
    }
    body
}

fn extract_texts(parsed: GoogleResponse) -> Vec<String> {
    parsed
        .data
        .translations
        .into_iter()
        .map(|t| t.translated_text)
        .collect()
}

impl TranslationEngine for GoogleTranslateEngine {
    fn translate(
        &self,
        texts: &[String],
        source_lang: Option<&str>,
        target_lang: &str,
        mut on_progress: Box<dyn FnMut(f32) + Send>,
    ) -> Result<Vec<String>> {
        on_progress(0.1);

        let body = request_body(texts, target_lang, source_lang);

        let client = reqwest::blocking::Client::new();
        let response = client
            .post(ENDPOINT)
            .query(&[("key", &self.api_key)])
            .json(&body)
            .send()
            .context("sending request to Google Translate API")?
            .error_for_status()
            .context("Google Translate API returned an error")?;

        on_progress(0.8);

        let parsed: GoogleResponse = response
            .json()
            .context("parsing Google Translate API response")?;

        on_progress(1.0);

        Ok(extract_texts(parsed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_camel_cased_translated_text() {
        let parsed: GoogleResponse = serde_json::from_str(
            r#"{"data":{"translations":[{"translatedText":"Bonjour"},{"translatedText":"Monde"}]}}"#,
        )
        .unwrap();

        let texts: Vec<_> = parsed
            .data
            .translations
            .into_iter()
            .map(|t| t.translated_text)
            .collect();
        assert_eq!(texts, vec!["Bonjour", "Monde"]);
    }

    #[test]
    fn parses_an_empty_result_set() {
        let parsed: GoogleResponse =
            serde_json::from_str(r#"{"data":{"translations":[]}}"#).unwrap();
        assert!(parsed.data.translations.is_empty());
    }

    fn lines() -> Vec<String> {
        vec!["Hello".to_string(), "World".to_string()]
    }

    #[test]
    fn the_body_sends_every_line_under_q() {
        let body = request_body(&lines(), "fr", None);

        assert_eq!(body["q"], json!(["Hello", "World"]));
        assert_eq!(body["target"], "fr");
    }

    #[test]
    fn plain_text_is_requested_so_no_html_comes_back() {
        let body = request_body(&lines(), "fr", None);

        assert_eq!(body["format"], "text");
    }

    #[test]
    fn the_source_language_is_omitted_when_unknown() {
        let body = request_body(&lines(), "fr", None);

        assert!(body.get("source").is_none());
    }

    #[test]
    fn the_source_language_is_sent_when_known() {
        let body = request_body(&lines(), "fr", Some("en"));

        assert_eq!(body["source"], "en");
    }

    #[test]
    fn an_empty_input_sends_an_empty_array() {
        let body = request_body(&[], "fr", None);

        assert_eq!(body["q"], json!([]));
    }

    #[test]
    fn the_response_yields_the_translations_in_order() {
        let parsed: GoogleResponse = serde_json::from_str(
            r#"{"data":{"translations":[{"translatedText":"Bonjour"},{"translatedText":"Monde"}]}}"#,
        )
        .unwrap();

        assert_eq!(extract_texts(parsed), vec!["Bonjour", "Monde"]);
    }

    #[test]
    fn the_endpoint_is_the_v2_rest_api() {
        assert_eq!(
            ENDPOINT,
            "https://translation.googleapis.com/language/translate/v2"
        );
    }
}
