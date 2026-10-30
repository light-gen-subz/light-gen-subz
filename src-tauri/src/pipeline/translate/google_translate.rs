use anyhow::Context;
use anyhow::Result;
use serde::Deserialize;
use serde_json::json;
use zeroize::Zeroizing;

use super::{translate_in_batches, BatchLimits, TranslationEngine};
use crate::http;

const ENDPOINT: &str = "https://translation.googleapis.com/language/translate/v2";

/// Google v2 takes at most 128 strings per request.
const LIMITS: BatchLimits = BatchLimits {
    max_items: 100,
    max_chars: 20_000,
};

pub struct GoogleTranslateEngine {
    api_key: Zeroizing<String>,
    endpoint: String,
}

impl GoogleTranslateEngine {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: Zeroizing::new(api_key.into()),
            endpoint: ENDPOINT.to_string(),
        }
    }

    #[cfg(test)]
    fn with_endpoint(mut self, url: String) -> Self {
        self.endpoint = url;
        self
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

/// JSON body sent to Google Translate. Separate from `translate` so it can be checked
/// without a network call.
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
        translate_in_batches(texts, LIMITS, &mut *on_progress, |batch, _| {
            // The key travels in a header, not in the URL, so it cannot end up in an
            // error message or a proxy log.
            let parsed: GoogleResponse = http::send_checked(
                || {
                    Ok(http::client()
                        .post(&self.endpoint)
                        .header("X-Goog-Api-Key", self.api_key.as_str())
                        .json(&request_body(batch, target_lang, source_lang)))
                },
                "Google Translate",
            )?
            .json()
            .context("parsing Google Translate API response")?;
            Ok(extract_texts(parsed))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::testserver::serve;

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

    #[test]
    fn the_key_is_sent_in_a_header_and_never_in_the_url() {
        let mock = serve(vec![(
            200,
            r#"{"data":{"translations":[{"translatedText":"a"},{"translatedText":"b"}]}}"#.into(),
        )]);
        let engine = GoogleTranslateEngine::new("SECRET").with_endpoint(mock.url.clone());

        engine
            .translate(&lines(), None, "fr", Box::new(|_| {}))
            .unwrap();

        let sent = mock.requests.lock().unwrap();
        assert!(sent[0]
            .headers
            .to_ascii_lowercase()
            .contains("x-goog-api-key: secret"));
        assert!(!sent[0].request_line.contains("SECRET"));
    }

    #[test]
    fn more_than_a_hundred_lines_are_sent_in_several_requests() {
        let texts: Vec<String> = (0..230).map(|i| format!("l{i}")).collect();
        let answer = |n: usize| {
            let items: Vec<String> = (0..n)
                .map(|_| r#"{"translatedText":"t"}"#.to_string())
                .collect();
            (
                200,
                format!(r#"{{"data":{{"translations":[{}]}}}}"#, items.join(",")),
            )
        };
        let mock = serve(vec![answer(100), answer(100), answer(30)]);
        let engine = GoogleTranslateEngine::new("k").with_endpoint(mock.url.clone());

        let out = engine
            .translate(&texts, None, "fr", Box::new(|_| {}))
            .unwrap();

        assert_eq!(out.len(), 230);
        assert_eq!(mock.requests.lock().unwrap().len(), 3);
    }
}
