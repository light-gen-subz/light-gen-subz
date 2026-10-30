use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::json;
use zeroize::Zeroizing;

use super::{translate_in_batches, BatchLimits, TranslationEngine};
use crate::http;

const ENDPOINT: &str = "https://api.cognitive.microsofttranslator.com/translate";

/// Azure caps a request at 1 000 elements and 50 000 characters; this keeps well below.
const LIMITS: BatchLimits = BatchLimits {
    max_items: 100,
    max_chars: 20_000,
};

pub struct AzureTranslateEngine {
    api_key: Zeroizing<String>,
    region: String,
    endpoint: String,
}

impl AzureTranslateEngine {
    pub fn new(api_key: impl Into<String>, region: impl Into<String>) -> Self {
        Self {
            api_key: Zeroizing::new(api_key.into()),
            region: region.into(),
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
struct Translation {
    text: String,
}

#[derive(Deserialize)]
struct TranslationResult {
    translations: Vec<Translation>,
}

/// Azure names Simplified Chinese `zh-Hans`; the other codes are the ones we use.
fn azure_code(code: &str) -> &str {
    if code.eq_ignore_ascii_case("zh") {
        "zh-Hans"
    } else {
        code
    }
}

/// URL parameters: pinned API version, target language, and source language if known.
fn query_params<'a>(target_lang: &'a str, source_lang: Option<&'a str>) -> Vec<(&'a str, &'a str)> {
    let mut query = vec![("api-version", "3.0"), ("to", azure_code(target_lang))];
    if let Some(src) = source_lang {
        query.push(("from", azure_code(src)));
    }
    query
}

/// Azure expects an array of `{"Text": ...}` objects.
fn request_body(texts: &[String]) -> Vec<serde_json::Value> {
    texts.iter().map(|t| json!({ "Text": t })).collect()
}

/// Azure returns a list of results, each of which may carry several translations (one per
/// requested target language). Only one is requested: the first is taken.
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
        let query = query_params(target_lang, source_lang);

        translate_in_batches(texts, LIMITS, &mut *on_progress, |batch, _| {
            let parsed: Vec<TranslationResult> = http::send_checked(
                || {
                    Ok(http::client()
                        .post(&self.endpoint)
                        .query(&query)
                        .header("Ocp-Apim-Subscription-Key", self.api_key.as_str())
                        .header("Ocp-Apim-Subscription-Region", &self.region)
                        .json(&request_body(batch)))
                },
                "Azure Translator",
            )?
            .json()
            .context("parsing Azure Translator API response")?;
            Ok(extract_texts(parsed))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::testserver::serve;

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
    fn chinese_uses_the_simplified_script_code() {
        assert!(query_params("zh", Some("zh")).contains(&("to", "zh-Hans")));
        assert!(query_params("fr", Some("zh")).contains(&("from", "zh-Hans")));
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

    #[test]
    fn the_key_and_region_headers_are_sent_and_long_inputs_are_batched() {
        let texts: Vec<String> = (0..150).map(|i| format!("l{i}")).collect();
        let answer = |n: usize| {
            let items: Vec<String> = (0..n)
                .map(|_| r#"{"translations":[{"text":"t"}]}"#.to_string())
                .collect();
            (200, format!("[{}]", items.join(",")))
        };
        let mock = serve(vec![answer(100), answer(50)]);
        let engine = AzureTranslateEngine::new("KEY", "westeurope").with_endpoint(mock.url.clone());

        let out = engine
            .translate(&texts, None, "fr", Box::new(|_| {}))
            .unwrap();

        assert_eq!(out.len(), 150);
        let sent = mock.requests.lock().unwrap();
        assert_eq!(sent.len(), 2);
        let headers = sent[0].headers.to_ascii_lowercase();
        assert!(headers.contains("ocp-apim-subscription-key: key"));
        assert!(headers.contains("ocp-apim-subscription-region: westeurope"));
    }
}
