use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

use super::{Segment, SttEngine, Transcript};

const ENDPOINT: &str = "https://api.deepgram.com/v1/listen?model=nova-2&smart_format=true&punctuate=true&utterances=true&detect_language=true";

pub struct DeepgramEngine {
    api_key: String,
}

impl DeepgramEngine {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
        }
    }
}

#[derive(Deserialize)]
struct Utterance {
    start: f64,
    end: f64,
    transcript: String,
}

#[derive(Deserialize)]
struct Channel {
    detected_language: Option<String>,
}

#[derive(Deserialize)]
struct Results {
    channels: Vec<Channel>,
    utterances: Vec<Utterance>,
}

#[derive(Deserialize)]
struct DeepgramResponse {
    results: Results,
}

/// Traduit la réponse Deepgram en transcription. Séparé de `transcribe` pour que la
/// conversion soit testable sans appel réseau.
fn to_transcript(parsed: DeepgramResponse) -> Transcript {
    let language = parsed
        .results
        .channels
        .first()
        .and_then(|c| c.detected_language.clone())
        .unwrap_or_else(|| "unknown".to_string());

    Transcript {
        language,
        segments: parsed
            .results
            .utterances
            .into_iter()
            .map(|u| Segment {
                start: u.start,
                end: u.end,
                text: u.transcript.trim().to_string(),
            })
            .collect(),
    }
}

/// En-tête d'authentification attendu par Deepgram.
fn auth_header(api_key: &str) -> String {
    format!("Token {api_key}")
}

impl SttEngine for DeepgramEngine {
    fn transcribe(
        &self,
        wav_path: &Path,
        mut on_progress: Box<dyn FnMut(f32) + Send>,
    ) -> Result<Transcript> {
        on_progress(0.05);

        let file_bytes = std::fs::read(wav_path).context("reading WAV file for upload")?;

        on_progress(0.2);

        let client = reqwest::blocking::Client::new();
        let response = client
            .post(ENDPOINT)
            .header("Authorization", auth_header(&self.api_key))
            .header("Content-Type", "audio/wav")
            .body(file_bytes)
            .send()
            .context("sending request to Deepgram API")?
            .error_for_status()
            .context("Deepgram API returned an error")?;

        on_progress(0.9);

        let parsed: DeepgramResponse = response.json().context("parsing Deepgram API response")?;

        on_progress(1.0);

        Ok(to_transcript(parsed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "results": {
        "channels": [{ "detected_language": "fr" }],
        "utterances": [
          { "start": 0.0, "end": 1.5, "transcript": "Bonjour" },
          { "start": 1.5, "end": 3.0, "transcript": "le monde" }
        ]
      }
    }"#;

    #[test]
    fn parses_utterances_with_their_timings() {
        let parsed: DeepgramResponse = serde_json::from_str(SAMPLE).unwrap();
        let utterances = parsed.results.utterances;

        assert_eq!(utterances.len(), 2);
        assert_eq!(utterances[0].transcript, "Bonjour");
        assert_eq!(utterances[0].start, 0.0);
        assert_eq!(utterances[1].end, 3.0);
    }

    #[test]
    fn reads_the_detected_language() {
        let parsed: DeepgramResponse = serde_json::from_str(SAMPLE).unwrap();
        assert_eq!(
            parsed.results.channels[0].detected_language.as_deref(),
            Some("fr")
        );
    }

    #[test]
    fn tolerates_a_response_with_no_detected_language() {
        let parsed: DeepgramResponse =
            serde_json::from_str(r#"{"results":{"channels":[{}],"utterances":[]}}"#).unwrap();
        assert!(parsed.results.channels[0].detected_language.is_none());
        assert!(parsed.results.utterances.is_empty());
    }

    fn transcript_of(json: &str) -> Transcript {
        to_transcript(serde_json::from_str(json).unwrap())
    }

    #[test]
    fn a_response_becomes_a_transcript_with_its_segments() {
        let t = transcript_of(SAMPLE);

        assert_eq!(t.language, "fr");
        assert_eq!(t.segments.len(), 2);
        assert_eq!(t.segments[0].text, "Bonjour");
        assert_eq!(t.segments[1].start, 1.5);
        assert_eq!(t.segments[1].end, 3.0);
    }

    #[test]
    fn an_undetected_language_falls_back_to_unknown() {
        let t = transcript_of(r#"{"results":{"channels":[{}],"utterances":[]}}"#);

        assert_eq!(t.language, "unknown");
        assert!(t.segments.is_empty());
    }

    #[test]
    fn a_response_without_any_channel_also_falls_back() {
        let t = transcript_of(r#"{"results":{"channels":[],"utterances":[]}}"#);

        assert_eq!(t.language, "unknown");
    }

    #[test]
    fn the_surrounding_whitespace_of_an_utterance_is_trimmed() {
        let t = transcript_of(
            r#"{"results":{"channels":[{"detected_language":"en"}],
                "utterances":[{"start":0.0,"end":1.0,"transcript":"  hello  "}]}}"#,
        );

        assert_eq!(t.segments[0].text, "hello");
    }

    #[test]
    fn only_the_first_channel_decides_the_language() {
        let t = transcript_of(
            r#"{"results":{"channels":[{"detected_language":"es"},{"detected_language":"de"}],
                "utterances":[]}}"#,
        );

        assert_eq!(t.language, "es");
    }

    #[test]
    fn the_api_key_goes_in_a_token_header() {
        assert_eq!(auth_header("sk-abc"), "Token sk-abc");
    }

    #[test]
    fn the_endpoint_asks_for_utterances_and_language_detection() {
        // La segmentation en aval dépend des utterances, et la langue est remontée à l'UI.
        assert!(ENDPOINT.contains("utterances=true"));
        assert!(ENDPOINT.contains("detect_language=true"));
        assert!(ENDPOINT.contains("punctuate=true"));
    }
}
