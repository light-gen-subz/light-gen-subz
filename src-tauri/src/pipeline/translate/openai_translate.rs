use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::json;

use super::TranslationEngine;

const ENDPOINT: &str = "https://api.openai.com/v1/chat/completions";
const MODEL: &str = "gpt-4o-mini";

pub struct OpenAiTranslateEngine {
    api_key: String,
}

impl OpenAiTranslateEngine {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
        }
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

/// Consigne système envoyée au modèle. Séparée de `translate` pour que le contrat
/// imposé au modèle (même nombre de lignes, même ordre, JSON) soit vérifiable.
fn system_prompt(target_lang: &str, source_lang: Option<&str>) -> String {
    let source_note = source_lang
        .map(|s| format!("The source language is '{s}'."))
        .unwrap_or_else(|| "Detect the source language automatically.".to_string());

    format!(
        "You translate subtitle lines into language code '{target_lang}'. {source_note} \
         You will receive a JSON array of strings, each one subtitle line. Return a JSON \
         object {{\"translations\": [...]}} with EXACTLY the same number of strings, in the \
         same order, translated. Keep each translation concise and matching the tone of the \
         original. Do not merge or split lines."
    )
}

/// Corps de la requête chat-completions.
fn request_body(
    texts: &[String],
    target_lang: &str,
    source_lang: Option<&str>,
) -> Result<serde_json::Value> {
    Ok(json!({
        "model": MODEL,
        "response_format": {"type": "json_object"},
        "messages": [
            {"role": "system", "content": system_prompt(target_lang, source_lang)},
            {"role": "user", "content": serde_json::to_string(texts).context("serializing input lines")?},
        ],
    }))
}

/// Extrait les traductions de la réponse et vérifie que le modèle a respecté le contrat
/// « une sortie par entrée ».
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

    anyhow::ensure!(
        payload.translations.len() == expected_len,
        "OpenAI returned {} translations for {} input lines",
        payload.translations.len(),
        expected_len
    );

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
        on_progress(0.1);

        let body = request_body(texts, target_lang, source_lang)?;

        let client = reqwest::blocking::Client::new();
        let response = client
            .post(ENDPOINT)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .context("sending request to OpenAI API")?
            .error_for_status()
            .context("OpenAI API returned an error")?;

        on_progress(0.8);

        let parsed: ChatResponse = response.json().context("parsing OpenAI API response")?;
        let translations = extract_translations(parsed, texts.len())?;

        on_progress(1.0);
        Ok(translations)
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

    // ── consigne système ─────────────────────────────────────────────────────

    #[test]
    fn the_prompt_names_the_target_language() {
        let prompt = system_prompt("fr", None);

        assert!(prompt.contains("'fr'"), "{prompt}");
    }

    #[test]
    fn the_prompt_asks_for_auto_detection_when_the_source_is_unknown() {
        let prompt = system_prompt("fr", None);

        assert!(prompt.contains("Detect the source language automatically"));
        assert!(!prompt.contains("The source language is"));
    }

    #[test]
    fn the_prompt_states_the_source_language_when_known() {
        let prompt = system_prompt("fr", Some("en"));

        assert!(prompt.contains("The source language is 'en'"));
    }

    #[test]
    fn the_prompt_pins_the_one_line_in_one_line_out_contract() {
        let prompt = system_prompt("fr", None);

        assert!(prompt.contains("EXACTLY the same number of strings"));
        assert!(prompt.contains("same order"));
        assert!(prompt.contains("Do not merge or split lines"));
    }

    // ── corps de requête ─────────────────────────────────────────────────────

    fn lines() -> Vec<String> {
        vec!["Hello".to_string(), "World".to_string()]
    }

    #[test]
    fn the_body_forces_a_json_object_response() {
        let body = request_body(&lines(), "fr", None).unwrap();

        assert_eq!(body["response_format"]["type"], "json_object");
        assert_eq!(body["model"], MODEL);
    }

    #[test]
    fn the_lines_are_sent_as_a_json_array_in_the_user_message() {
        let body = request_body(&lines(), "fr", None).unwrap();

        assert_eq!(body["messages"][1]["role"], "user");
        assert_eq!(body["messages"][1]["content"], r#"["Hello","World"]"#);
    }

    #[test]
    fn the_system_message_comes_first() {
        let body = request_body(&lines(), "fr", Some("en")).unwrap();

        assert_eq!(body["messages"][0]["role"], "system");
        assert!(body["messages"][0]["content"]
            .as_str()
            .unwrap()
            .contains("'en'"));
    }

    // ── extraction de la réponse ─────────────────────────────────────────────

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
    fn a_mismatched_line_count_is_rejected() {
        // Le modèle a fusionné deux sous-titres : les timings ne colleraient plus.
        let parsed = response(
            r#"{"choices":[{"message":{"content":"{\"translations\":[\"Bonjour Monde\"]}"}}]}"#,
        );

        let err = extract_translations(parsed, 2).unwrap_err();

        assert!(
            err.to_string().contains("1 translations for 2 input lines"),
            "{err}"
        );
    }
}
