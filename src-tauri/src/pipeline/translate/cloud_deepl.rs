use anyhow::{Context, Result};
use serde::Deserialize;
use zeroize::Zeroizing;

use super::{translate_in_batches, BatchLimits, TranslationEngine};
use crate::http;

/// DeepL accepts at most 50 `text` entries and 128 KiB of body per request.
const LIMITS: BatchLimits = BatchLimits {
    max_items: 50,
    max_chars: 30_000,
};

#[derive(Deserialize)]
struct DeepLTranslation {
    text: String,
}

#[derive(Deserialize)]
struct DeepLResponse {
    translations: Vec<DeepLTranslation>,
}

pub struct CloudDeepLEngine {
    api_key: Zeroizing<String>,
    endpoint_override: Option<String>,
}

impl CloudDeepLEngine {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: Zeroizing::new(api_key.into()),
            endpoint_override: None,
        }
    }

    #[cfg(test)]
    fn with_endpoint(mut self, url: String) -> Self {
        self.endpoint_override = Some(url);
        self
    }

    /// DeepL uses separate hosts for free-tier and paid keys (free keys end in ":fx").
    fn endpoint(&self) -> String {
        if let Some(url) = &self.endpoint_override {
            return url.clone();
        }
        if self.api_key.ends_with(":fx") {
            "https://api-free.deepl.com/v2/translate".to_string()
        } else {
            "https://api.deepl.com/v2/translate".to_string()
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

/// Form parameters sent to DeepL: one `text` entry per line, plus the target language and,
/// if known, the source language. Separate from `translate` so it can be checked without
/// a network call.
fn form_params<'a>(
    texts: &'a [String],
    target_lang: &'a str,
    source_lang: Option<&'a str>,
    context: &'a str,
) -> Vec<(&'a str, &'a str)> {
    let mut params: Vec<(&str, &str)> = texts.iter().map(|t| ("text", t.as_str())).collect();
    params.push(("target_lang", target_lang));
    if let Some(src) = source_lang {
        params.push(("source_lang", src));
    }
    // Read by DeepL to disambiguate, never translated, never billed.
    if !context.is_empty() {
        params.push(("context", context));
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
        let target_lang = deepl_target_lang(target_lang);
        let source_lang = source_lang.map(|s| s.to_ascii_uppercase());
        let endpoint = self.endpoint();

        translate_in_batches(texts, LIMITS, &mut *on_progress, |batch, context| {
            let context = context.join(" ");
            let params = form_params(batch, &target_lang, source_lang.as_deref(), &context);
            let parsed: DeepLResponse = http::send_checked(
                || {
                    Ok(http::client()
                        .post(&endpoint)
                        .header("Authorization", auth_header(self.api_key.as_str()))
                        .form(&params))
                },
                "DeepL",
            )?
            .json()
            .context("parsing DeepL API response")?;
            Ok(extract_texts(parsed))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::testserver::serve;

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
        let params = form_params(&input, "FR", None, "");

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
        let params = form_params(&input, "FR", None, "");

        assert!(params.contains(&("target_lang", "FR")));
    }

    #[test]
    fn the_source_language_is_omitted_when_unknown() {
        let input = lines();
        let params = form_params(&input, "FR", None, "");

        assert!(!params.iter().any(|(k, _)| *k == "source_lang"));
    }

    #[test]
    fn the_source_language_is_sent_when_known() {
        let input = lines();
        let params = form_params(&input, "FR", Some("EN"), "");

        assert!(params.contains(&("source_lang", "EN")));
    }

    #[test]
    fn an_empty_input_still_carries_the_target_language() {
        let params = form_params(&[], "DE", None, "");

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

    #[test]
    fn more_than_fifty_lines_are_sent_in_several_requests() {
        // DeepL rejects a request with over 50 `text` entries.
        let texts: Vec<String> = (0..120).map(|i| format!("l{i}")).collect();
        let answer = |n: usize| {
            let items: Vec<String> = (0..n).map(|_| r#"{"text":"t"}"#.to_string()).collect();
            (200, format!(r#"{{"translations":[{}]}}"#, items.join(",")))
        };
        let mock = serve(vec![answer(50), answer(50), answer(20)]);
        let engine = CloudDeepLEngine::new("k").with_endpoint(mock.url.clone());

        let out = engine
            .translate(&texts, Some("en"), "fr", Box::new(|_| {}))
            .unwrap();

        assert_eq!(out.len(), 120);
        let sent = mock.requests.lock().unwrap();
        assert_eq!(sent.len(), 3);
        for request in sent.iter() {
            let body = String::from_utf8_lossy(&request.body).to_string();
            assert!(body.split('&').filter(|p| p.starts_with("text=")).count() <= 50);
            assert!(body.contains("target_lang=FR"));
        }
    }

    #[test]
    fn a_quota_error_is_reported_with_its_own_code() {
        let mock = serve(vec![(456, r#"{"message":"Quota exceeded"}"#.into())]);
        let engine = CloudDeepLEngine::new("k").with_endpoint(mock.url.clone());

        let err = engine
            .translate(&lines(), None, "fr", Box::new(|_| {}))
            .unwrap_err();
        let out = crate::errors::CommandError::from(err);

        assert_eq!(out.code, Some("quota_exceeded"));
        assert!(out.message.contains("Quota exceeded"), "{}", out.message);
    }

    #[test]
    fn the_previous_lines_are_sent_as_context_only() {
        let input = lines();
        let params = form_params(&input, "FR", None, "Before.");

        assert!(params.contains(&("context", "Before.")));
        assert_eq!(params.iter().filter(|(k, _)| *k == "text").count(), 2);
    }

    #[test]
    fn the_second_request_carries_the_end_of_the_first_batch() {
        let texts: Vec<String> = (0..60).map(|i| format!("l{i}")).collect();
        let answer = |n: usize| {
            let items: Vec<String> = (0..n).map(|_| r#"{"text":"t"}"#.to_string()).collect();
            (200, format!(r#"{{"translations":[{}]}}"#, items.join(",")))
        };
        let mock = serve(vec![answer(50), answer(10)]);
        let engine = CloudDeepLEngine::new("k").with_endpoint(mock.url.clone());

        engine
            .translate(&texts, Some("en"), "fr", Box::new(|_| {}))
            .unwrap();

        let sent = mock.requests.lock().unwrap();
        assert!(!String::from_utf8_lossy(&sent[0].body).contains("context="));
        assert!(String::from_utf8_lossy(&sent[1].body).contains("context=l48+l49"));
    }

    #[test]
    fn a_busy_deepl_is_asked_again() {
        let mock = serve(vec![
            (429, r#"{"message":"Too many requests"}"#.into()),
            (
                200,
                r#"{"translations":[{"text":"Bonjour"},{"text":"Monde"}]}"#.into(),
            ),
        ]);
        let engine = CloudDeepLEngine::new("k").with_endpoint(mock.url.clone());

        let out = engine
            .translate(&lines(), None, "fr", Box::new(|_| {}))
            .unwrap();

        assert_eq!(out, vec!["Bonjour", "Monde"]);
    }
}
