use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::json;
use zeroize::Zeroizing;

use super::{translate_in_batches, BatchLimits, TranslationEngine};
use crate::http;

const ENDPOINT: &str = "https://api.openai.com/v1/chat/completions";
const MODEL: &str = crate::cloud_models::OPENAI_TRANSLATION;

/// A long JSON array in one answer gets merged or cut short by the model: small batches.
const LIMITS: BatchLimits = BatchLimits {
    max_items: 40,
    max_chars: 6_000,
};

pub struct OpenAiTranslateEngine {
    api_key: Zeroizing<String>,
    endpoint: String,
}

impl OpenAiTranslateEngine {
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

    fn call(
        &self,
        texts: &[String],
        target: &str,
        source: Option<&str>,
        context: &[String],
    ) -> Result<Vec<String>> {
        let body = request_body(texts, target, source, context)?;
        let parsed: ChatResponse = http::send_checked(
            || {
                Ok(http::client()
                    .post(&self.endpoint)
                    .bearer_auth(self.api_key.as_str())
                    .json(&body))
            },
            "OpenAI",
        )?
        .json()
        .context("parsing OpenAI API response")?;
        extract_translations(parsed, texts.len())
    }
}

/// The model answered with a different number of lines than it was given.
#[derive(Debug)]
struct LineCountMismatch(String);

impl std::fmt::Display for LineCountMismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for LineCountMismatch {}

/// Asks the model for `texts`; when it merges or drops lines, asks again for each half
/// separately, down to single lines. Only a miscount is retried — a network or API
/// error is returned as is.
fn translate_resilient(
    texts: &[String],
    call: &mut dyn FnMut(&[String]) -> Result<Vec<String>>,
) -> Result<Vec<String>> {
    match call(texts) {
        Err(e) if texts.len() > 1 && e.downcast_ref::<LineCountMismatch>().is_some() => {
            let (left, right) = texts.split_at(texts.len() / 2);
            let mut out = translate_resilient(left, call)?;
            out.extend(translate_resilient(right, call)?);
            Ok(out)
        }
        other => other,
    }
}

#[derive(Deserialize)]
struct ChatMessage {
    content: String,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Deserialize)]
struct TranslationsPayload {
    translations: Vec<String>,
}

/// System prompt sent to the model. Separate from `translate` so the contract imposed on
/// the model (same number of lines, same order, JSON) can be checked.
fn system_prompt(target_lang: &str, source_lang: Option<&str>, context: &[String]) -> String {
    let source_note = source_lang
        .map(|s| format!("The source language is '{s}'."))
        .unwrap_or_else(|| "Detect the source language automatically.".to_string());
    let context_note = if context.is_empty() {
        String::new()
    } else {
        format!(
            " The subtitle lines just before these ones are {}: they are context only, do \
             not translate them and do not return them.",
            serde_json::to_string(context).unwrap_or_default()
        )
    };

    format!(
        "You translate subtitle lines into language code '{target_lang}'. {source_note} \
         You will receive a JSON array of strings, each one subtitle line. Return a JSON \
         object {{\"translations\": [...]}} with EXACTLY the same number of strings, in the \
         same order, translated. Keep each translation concise and matching the tone of the \
         original. Do not merge or split lines.{context_note}"
    )
}

/// Body of the chat-completions request.
fn request_body(
    texts: &[String],
    target_lang: &str,
    source_lang: Option<&str>,
    context: &[String],
) -> Result<serde_json::Value> {
    Ok(json!({
        "model": MODEL,
        "response_format": {"type": "json_object"},
        "messages": [
            {"role": "system", "content": system_prompt(target_lang, source_lang, context)},
            {"role": "user", "content": serde_json::to_string(texts).context("serializing input lines")?},
        ],
    }))
}

/// Extracts the translations from the response and checks that the model honoured the
/// "one output per input" contract.
fn extract_translations(parsed: ChatResponse, expected_len: usize) -> Result<Vec<String>> {
    let content = parsed
        .choices
        .into_iter()
        .next()
        .context("OpenAI API returned no choices")?
        .message
        .content;
    let payload: TranslationsPayload =
        serde_json::from_str(&content).context("parsing OpenAI translation JSON payload")?;

    if payload.translations.len() != expected_len {
        return Err(anyhow::Error::new(LineCountMismatch(format!(
            "OpenAI returned {} translations for {} input lines",
            payload.translations.len(),
            expected_len
        ))));
    }

    Ok(payload.translations)
}

impl TranslationEngine for OpenAiTranslateEngine {
    fn translate(
        &self,
        texts: &[String],
        source_lang: Option<&str>,
        target_lang: &str,
        mut on_progress: Box<dyn FnMut(f32) + Send>,
    ) -> Result<Vec<String>> {
        translate_in_batches(texts, LIMITS, &mut *on_progress, |batch, context| {
            translate_resilient(batch, &mut |part| {
                self.call(part, target_lang, source_lang, context)
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_assistant_message() {
        let parsed: ChatResponse = serde_json::from_str(
            r#"{"choices":[{"message":{"content":"{\"translations\":[\"Bonjour\"]}"}}]}"#,
        )
        .unwrap();

        assert_eq!(parsed.choices.len(), 1);
        let payload: TranslationsPayload =
            serde_json::from_str(&parsed.choices[0].message.content).unwrap();
        assert_eq!(payload.translations, vec!["Bonjour"]);
    }

    #[test]
    fn parses_a_multi_line_translation_payload() {
        let payload: TranslationsPayload =
            serde_json::from_str(r#"{"translations":["une","deux","trois"]}"#).unwrap();
        assert_eq!(payload.translations.len(), 3);
    }

    #[test]
    fn rejects_a_payload_that_is_not_the_agreed_shape() {
        let err = serde_json::from_str::<TranslationsPayload>(r#"{"lines":["a"]}"#);
        assert!(err.is_err());
    }

    // ── system prompt ────────────────────────────────────────────────────────

    #[test]
    fn the_prompt_names_the_target_language() {
        let prompt = system_prompt("fr", None, &[]);

        assert!(prompt.contains("'fr'"), "{prompt}");
    }

    #[test]
    fn the_prompt_asks_for_auto_detection_when_the_source_is_unknown() {
        let prompt = system_prompt("fr", None, &[]);

        assert!(prompt.contains("Detect the source language automatically"));
        assert!(!prompt.contains("The source language is"));
    }

    #[test]
    fn the_prompt_states_the_source_language_when_known() {
        let prompt = system_prompt("fr", Some("en"), &[]);

        assert!(prompt.contains("The source language is 'en'"));
    }

    #[test]
    fn the_prompt_pins_the_one_line_in_one_line_out_contract() {
        let prompt = system_prompt("fr", None, &[]);

        assert!(prompt.contains("EXACTLY the same number of strings"));
        assert!(prompt.contains("same order"));
        assert!(prompt.contains("Do not merge or split lines"));
    }

    // ── request body ─────────────────────────────────────────────────────────

    fn lines() -> Vec<String> {
        vec!["Hello".to_string(), "World".to_string()]
    }

    #[test]
    fn the_body_forces_a_json_object_response() {
        let body = request_body(&lines(), "fr", None, &[]).unwrap();

        assert_eq!(body["response_format"]["type"], "json_object");
        assert_eq!(body["model"], MODEL);
    }

    #[test]
    fn the_lines_are_sent_as_a_json_array_in_the_user_message() {
        let body = request_body(&lines(), "fr", None, &[]).unwrap();

        assert_eq!(body["messages"][1]["role"], "user");
        assert_eq!(body["messages"][1]["content"], r#"["Hello","World"]"#);
    }

    #[test]
    fn the_system_message_comes_first() {
        let body = request_body(&lines(), "fr", Some("en"), &[]).unwrap();

        assert_eq!(body["messages"][0]["role"], "system");
        assert!(body["messages"][0]["content"]
            .as_str()
            .unwrap()
            .contains("'en'"));
    }

    // ── extracting the response ──────────────────────────────────────────────

    fn response(json: &str) -> ChatResponse {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn the_translations_are_read_from_the_first_choice() {
        let parsed = response(
            r#"{"choices":[{"message":{"content":"{\"translations\":[\"Bonjour\",\"Monde\"]}"}}]}"#,
        );

        assert_eq!(
            extract_translations(parsed, 2).unwrap(),
            vec!["Bonjour", "Monde"]
        );
    }

    #[test]
    fn a_response_without_a_choice_is_rejected() {
        let err = extract_translations(response(r#"{"choices":[]}"#), 1).unwrap_err();

        assert!(err.to_string().contains("no choices"), "{err}");
    }

    #[test]
    fn a_non_json_assistant_message_is_rejected() {
        let parsed = response(r#"{"choices":[{"message":{"content":"Bonjour"}}]}"#);

        let err = extract_translations(parsed, 1).unwrap_err();

        assert!(
            err.to_string().contains("parsing OpenAI translation"),
            "{err}"
        );
    }

    #[test]
    fn a_miscount_is_retried_on_each_half() {
        let texts: Vec<String> = ["a", "b", "c", "d"].iter().map(|s| s.to_string()).collect();
        let mut calls: Vec<usize> = Vec::new();
        let mut call = |part: &[String]| -> Result<Vec<String>> {
            calls.push(part.len());
            if part.len() > 2 {
                Err(anyhow::Error::new(LineCountMismatch("merged".into())))
            } else {
                Ok(part.iter().map(|t| t.to_uppercase()).collect())
            }
        };

        let out = translate_resilient(&texts, &mut call).unwrap();

        assert_eq!(out, vec!["A", "B", "C", "D"]);
        assert_eq!(calls, vec![4, 2, 2]);
    }

    #[test]
    fn a_single_line_that_keeps_failing_is_reported() {
        let texts = vec!["a".to_string()];
        let mut call = |_: &[String]| -> Result<Vec<String>> {
            Err(anyhow::Error::new(LineCountMismatch("empty".into())))
        };

        assert!(translate_resilient(&texts, &mut call).is_err());
    }

    #[test]
    fn an_api_error_is_not_retried() {
        let texts = vec!["a".to_string(), "b".to_string()];
        let mut calls = 0;
        let mut call = |_: &[String]| -> Result<Vec<String>> {
            calls += 1;
            anyhow::bail!("HTTP 429")
        };

        assert!(translate_resilient(&texts, &mut call).is_err());
        assert_eq!(calls, 1);
    }

    #[test]
    fn a_long_input_is_translated_through_the_api_in_batches() {
        use crate::http::testserver::serve;
        let texts: Vec<String> = (0..60).map(|i| format!("l{i}")).collect();
        let answer = |n: usize| {
            let items: Vec<String> = (0..n).map(|_| "\"t\"".to_string()).collect();
            let content = format!("{{\"translations\":[{}]}}", items.join(","));
            let envelope = serde_json::json!({"choices":[{"message":{"content": content}}]});
            (200, envelope.to_string())
        };
        let mock = serve(vec![answer(40), answer(20)]);
        let engine = OpenAiTranslateEngine::new("sk-x").with_endpoint(mock.url.clone());

        let out = engine
            .translate(&texts, Some("en"), "fr", Box::new(|_| {}))
            .unwrap();

        assert_eq!(out.len(), 60);
        let sent = mock.requests.lock().unwrap();
        assert_eq!(sent.len(), 2);
        assert!(sent[0].headers.contains("authorization: Bearer sk-x"));
    }

    #[test]
    fn a_mismatched_line_count_is_rejected() {
        // The model merged two subtitles: the timings would no longer line up.
        let parsed = response(
            r#"{"choices":[{"message":{"content":"{\"translations\":[\"Bonjour Monde\"]}"}}]}"#,
        );

        let err = extract_translations(parsed, 2).unwrap_err();

        assert!(
            err.to_string().contains("1 translations for 2 input lines"),
            "{err}"
        );
    }

    #[test]
    fn the_previous_lines_are_given_as_context_only() {
        let prompt = system_prompt("fr", None, &["Earlier line.".to_string()]);

        assert!(prompt.contains("[\"Earlier line.\"]"), "{prompt}");
        assert!(prompt.contains("do not translate them"));
        assert!(!system_prompt("fr", None, &[]).contains("context only"));
    }
}
