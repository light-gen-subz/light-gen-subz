use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

use super::{Segment, SttEngine, Transcript};

/// Transcription via any OpenAI-Whisper-compatible `/audio/transcriptions` endpoint.
/// Covers both OpenAI itself and Groq, which expose an identical request/response shape.
pub struct OpenAiCompatibleWhisperEngine {
    endpoint: &'static str,
    model: &'static str,
    api_key: String,
}

impl OpenAiCompatibleWhisperEngine {
    pub fn groq(api_key: impl Into<String>) -> Self {
        Self {
            endpoint: "https://api.groq.com/openai/v1/audio/transcriptions",
            model: "whisper-large-v3-turbo",
            api_key: api_key.into(),
        }
    }

    pub fn openai(api_key: impl Into<String>) -> Self {
        Self {
            endpoint: "https://api.openai.com/v1/audio/transcriptions",
            model: "whisper-1",
            api_key: api_key.into(),
        }
    }
}

#[derive(Deserialize)]
struct ApiSegment {
    start: f64,
    end: f64,
    text: String,
}

#[derive(Deserialize)]
struct ApiResponse {
    language: String,
    segments: Vec<ApiSegment>,
}

/// Traduit la réponse `verbose_json` en transcription. Séparé de `transcribe` pour être
/// testable sans appel réseau.
fn to_transcript(parsed: ApiResponse) -> Transcript {
    Transcript {
        language: parsed.language,
        segments: parsed
            .segments
            .into_iter()
            .map(|s| Segment {
                start: s.start,
                end: s.end,
                text: s.text.trim().to_string(),
            })
            .collect(),
    }
}

impl SttEngine for OpenAiCompatibleWhisperEngine {
    fn transcribe(
        &self,
        wav_path: &Path,
        mut on_progress: Box<dyn FnMut(f32) + Send>,
    ) -> Result<Transcript> {
        on_progress(0.05);

        let file_bytes = std::fs::read(wav_path).context("reading WAV file for upload")?;
        let part = reqwest::blocking::multipart::Part::bytes(file_bytes)
            .file_name("audio.wav")
            .mime_str("audio/wav")
            .context("building multipart file part")?;
        let form = reqwest::blocking::multipart::Form::new()
            .part("file", part)
            .text("model", self.model)
            .text("response_format", "verbose_json");

        on_progress(0.2);

        let client = reqwest::blocking::Client::new();
        let response = client
            .post(self.endpoint)
            .bearer_auth(&self.api_key)
            .multipart(form)
            .send()
            .with_context(|| format!("sending request to {}", self.endpoint))?
            .error_for_status()
            .with_context(|| format!("{} returned an error", self.endpoint))?;

        on_progress(0.9);

        let parsed: ApiResponse = response.json().context("parsing API response")?;

        on_progress(1.0);

        Ok(to_transcript(parsed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groq_and_openai_use_their_own_endpoint_and_model() {
        let groq = OpenAiCompatibleWhisperEngine::groq("k");
        assert_eq!(
            groq.endpoint,
            "https://api.groq.com/openai/v1/audio/transcriptions"
        );
        assert_eq!(groq.model, "whisper-large-v3-turbo");

        let openai = OpenAiCompatibleWhisperEngine::openai("k");
        assert_eq!(
            openai.endpoint,
            "https://api.openai.com/v1/audio/transcriptions"
        );
        assert_eq!(openai.model, "whisper-1");
    }

    #[test]
    fn parses_a_segment_with_its_timings() {
        let parsed: ApiSegment =
            serde_json::from_str(r#"{"start":1.25,"end":2.5,"text":" Bonjour"}"#).unwrap();
        assert_eq!(parsed.start, 1.25);
        assert_eq!(parsed.end, 2.5);
        assert_eq!(parsed.text, " Bonjour");
    }

    fn transcript_of(json: &str) -> Transcript {
        to_transcript(serde_json::from_str(json).unwrap())
    }

    #[test]
    fn a_response_becomes_a_transcript() {
        let t = transcript_of(
            r#"{"language":"french","segments":[
                {"start":0.0,"end":1.5,"text":" Bonjour"},
                {"start":1.5,"end":3.0,"text":"le monde "}
            ]}"#,
        );

        assert_eq!(t.language, "french");
        assert_eq!(t.segments.len(), 2);
        // Whisper préfixe ses segments d'une espace : elle est retirée.
        assert_eq!(t.segments[0].text, "Bonjour");
        assert_eq!(t.segments[1].text, "le monde");
    }

    #[test]
    fn a_silent_recording_yields_no_segment() {
        let t = transcript_of(r#"{"language":"en","segments":[]}"#);

        assert_eq!(t.language, "en");
        assert!(t.segments.is_empty());
    }

    #[test]
    fn the_timings_are_carried_over_untouched() {
        let t = transcript_of(
            r#"{"language":"en","segments":[{"start":12.34,"end":56.78,"text":"x"}]}"#,
        );

        assert_eq!(t.segments[0].start, 12.34);
        assert_eq!(t.segments[0].end, 56.78);
    }

    #[test]
    fn a_response_missing_the_language_is_rejected() {
        assert!(serde_json::from_str::<ApiResponse>(r#"{"segments":[]}"#).is_err());
    }

    #[test]
    fn both_factories_keep_the_api_key() {
        assert_eq!(OpenAiCompatibleWhisperEngine::groq("k1").api_key, "k1");
        assert_eq!(OpenAiCompatibleWhisperEngine::openai("k2").api_key, "k2");
    }
}
