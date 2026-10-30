use std::path::Path;
use zeroize::Zeroizing;

use anyhow::{Context, Result};
use serde::Deserialize;

use super::{cues_from_words, lang, SttEngine, Transcript};
use crate::http;
use crate::pipeline::audio_extract::AudioFormat;
use crate::pipeline::segmentation::{TimedWord, DEFAULT_LIMITS};

const BASE_URL: &str = "https://api.deepgram.com/v1/listen";

pub struct DeepgramEngine {
    api_key: Zeroizing<String>,
    base_url: String,
    language: Option<String>,
}

impl DeepgramEngine {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: Zeroizing::new(api_key.into()),
            base_url: BASE_URL.to_string(),
            language: None,
        }
    }

    /// Forces the spoken language instead of letting Deepgram detect it.
    pub fn with_language(mut self, language: Option<String>) -> Self {
        self.language = language;
        self
    }

    #[cfg(test)]
    fn with_base_url(mut self, url: String) -> Self {
        self.base_url = url;
        self
    }
}

#[derive(Deserialize)]
struct Word {
    word: String,
    punctuated_word: Option<String>,
    start: f64,
    end: f64,
}

#[derive(Deserialize)]
struct Utterance {
    start: f64,
    end: f64,
    transcript: String,
    /// Time of every word: cues are cut where the speech pauses.
    #[serde(default)]
    words: Vec<Word>,
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

/// Request URL: either the forced language or automatic detection.
fn endpoint(base: &str, language: Option<&str>) -> String {
    let language_param = match language {
        Some(code) => format!("language={code}"),
        None => "detect_language=true".to_string(),
    };
    format!(
        "{base}?model={}&smart_format=true&punctuate=true&utterances=true&{language_param}",
        crate::cloud_models::DEEPGRAM_TRANSCRIPTION
    )
}

/// Turns Deepgram's response into a transcript. Separate from `transcribe` so the
/// conversion can be tested without a network call. `forced` is the forced language,
/// which Deepgram does not send back.
fn to_transcript(parsed: DeepgramResponse, forced: Option<&str>) -> Transcript {
    let language = parsed
        .results
        .channels
        .first()
        .and_then(|c| c.detected_language.clone())
        .or_else(|| forced.map(str::to_string))
        .map(|l| lang::to_code(&l))
        .unwrap_or_else(|| "unknown".to_string());

    Transcript {
        language,
        segments: parsed
            .results
            .utterances
            .into_iter()
            .flat_map(|u| {
                let words = u
                    .words
                    .into_iter()
                    .map(|w| TimedWord {
                        text: w.punctuated_word.unwrap_or(w.word).trim().to_string(),
                        start: w.start,
                        end: w.end,
                    })
                    .filter(|w| !w.text.is_empty())
                    .collect();
                cues_from_words(u.start, u.end, u.transcript.trim(), words, DEFAULT_LIMITS)
            })
            .collect(),
    }
}

/// Authentication header Deepgram expects.
fn auth_header(api_key: &str) -> String {
    format!("Token {api_key}")
}

impl SttEngine for DeepgramEngine {
    fn audio_format(&self) -> AudioFormat {
        AudioFormat::Mp3
    }

    fn transcribe(
        &self,
        audio: &Path,
        mut on_progress: Box<dyn FnMut(f32) + Send>,
    ) -> Result<Transcript> {
        on_progress(0.1);

        // Streamed from disk: the whole file is never held in memory.
        let response = http::send_checked(
            || {
                let file = std::fs::File::open(audio).context("opening audio for upload")?;
                Ok(http::client()
                    .post(endpoint(&self.base_url, self.language.as_deref()))
                    .header("Authorization", auth_header(self.api_key.as_str()))
                    .header("Content-Type", "audio/mpeg")
                    .body(file))
            },
            "Deepgram",
        )?;

        on_progress(0.9);

        let parsed: DeepgramResponse = response.json().context("parsing Deepgram API response")?;

        on_progress(1.0);

        Ok(to_transcript(parsed, self.language.as_deref()))
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
        to_transcript(serde_json::from_str(json).unwrap(), None)
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
    fn a_regional_language_is_reduced_to_its_code() {
        let t = transcript_of(
            r#"{"results":{"channels":[{"detected_language":"en-US"}],"utterances":[]}}"#,
        );

        assert_eq!(t.language, "en");
    }

    #[test]
    fn an_undetected_language_falls_back_to_unknown() {
        let t = transcript_of(r#"{"results":{"channels":[{}],"utterances":[]}}"#);

        assert_eq!(t.language, "unknown");
        assert!(t.segments.is_empty());
    }

    #[test]
    fn a_forced_language_is_reported_when_deepgram_does_not_detect() {
        let t = to_transcript(
            serde_json::from_str(r#"{"results":{"channels":[{}],"utterances":[]}}"#).unwrap(),
            Some("de"),
        );

        assert_eq!(t.language, "de");
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
        // Downstream segmentation depends on the utterances, and the language is reported to the UI.
        let url = endpoint(BASE_URL, None);
        assert!(url.contains("utterances=true"));
        assert!(url.contains("detect_language=true"));
        assert!(url.contains("punctuate=true"));
    }

    #[test]
    fn a_forced_language_replaces_the_detection() {
        let url = endpoint(BASE_URL, Some("fr"));

        assert!(url.contains("language=fr"));
        assert!(!url.contains("detect_language"));
    }

    #[test]
    fn the_builder_keeps_its_settings() {
        let engine = DeepgramEngine::new("k")
            .with_language(Some("fr".into()))
            .with_base_url("http://x".into());

        assert_eq!(engine.language.as_deref(), Some("fr"));
        assert_eq!(engine.base_url, "http://x");
    }

    #[test]
    fn words_cut_an_utterance_at_a_real_pause() {
        let t = transcript_of(
            r#"{"results":{"channels":[{"detected_language":"en"}],"utterances":[
                {"start":0.0,"end":12.0,"transcript":"Hello there. Again.",
                 "words":[{"word":"hello","punctuated_word":"Hello","start":0.0,"end":0.4},
                          {"word":"there","punctuated_word":"there.","start":0.5,"end":0.9},
                          {"word":"again","punctuated_word":"Again.","start":10.0,"end":10.6}]}]}}"#,
        );

        let texts: Vec<_> = t.segments.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(texts, ["Hello there.", "Again."]);
        assert_eq!(t.segments[1].start, 10.0);
    }

    #[test]
    fn an_utterance_without_words_is_kept_as_it_is() {
        let t = transcript_of(SAMPLE);

        assert_eq!(t.segments[0].text, "Bonjour");
        assert_eq!(t.segments[0].end, 1.5);
    }

    #[test]
    fn the_upload_survives_a_busy_provider() {
        use crate::http::testserver::serve;
        let mock = serve(vec![(503, "busy".into()), (200, SAMPLE.into())]);
        let dir = tempfile::tempdir().unwrap();
        let audio = dir.path().join("a.mp3");
        std::fs::write(&audio, b"fake").unwrap();
        let engine = DeepgramEngine::new("k").with_base_url(mock.url.clone());

        let t = engine.transcribe(&audio, Box::new(|_| {})).unwrap();

        assert_eq!(t.segments.len(), 2);
        let sent = mock.requests.lock().unwrap();
        assert_eq!(sent.len(), 2);
        assert!(sent[1].body.starts_with(b"fake"));
    }
}
