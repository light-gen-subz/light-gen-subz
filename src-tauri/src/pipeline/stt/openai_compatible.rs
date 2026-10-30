use std::path::Path;
use zeroize::Zeroizing;

use anyhow::{Context, Result};
use serde::Deserialize;

use super::{cues_from_words, lang, SttEngine, Transcript};
use crate::cancel;
use crate::http;
use crate::pipeline::audio_extract::{self, AudioFormat};
use crate::pipeline::segmentation::{TimedWord, DEFAULT_LIMITS};

/// Transcription via any OpenAI-Whisper-compatible `/audio/transcriptions` endpoint.
/// Covers both OpenAI itself and Groq, which expose an identical request/response shape.
///
/// The audio is sent as compressed chunks of at most [`audio_extract::CLOUD_CHUNK_SECS`]:
/// both providers reject a request body above 25 MB, which an uncompressed WAV reaches
/// after about 13 minutes.
pub struct OpenAiCompatibleWhisperEngine {
    endpoint: String,
    model: &'static str,
    provider: &'static str,
    api_key: Zeroizing<String>,
    language: Option<String>,
    /// Longest slice of audio sent in one request.
    chunk_secs: f64,
}

impl OpenAiCompatibleWhisperEngine {
    pub fn groq(api_key: impl Into<String>) -> Self {
        Self {
            endpoint: "https://api.groq.com/openai/v1/audio/transcriptions".to_string(),
            model: crate::cloud_models::GROQ_TRANSCRIPTION,
            provider: "Groq",
            api_key: Zeroizing::new(api_key.into()),
            language: None,
            chunk_secs: audio_extract::CLOUD_CHUNK_SECS,
        }
    }

    pub fn openai(api_key: impl Into<String>) -> Self {
        Self {
            endpoint: "https://api.openai.com/v1/audio/transcriptions".to_string(),
            model: crate::cloud_models::OPENAI_TRANSCRIPTION,
            provider: "OpenAI",
            api_key: Zeroizing::new(api_key.into()),
            language: None,
            chunk_secs: audio_extract::CLOUD_CHUNK_SECS,
        }
    }

    /// Forces the spoken language (ISO-639-1) instead of letting the API detect it.
    pub fn with_language(mut self, language: Option<String>) -> Self {
        self.language = language;
        self
    }

    #[cfg(test)]
    fn with_chunk_secs(mut self, secs: f64) -> Self {
        self.chunk_secs = secs;
        self
    }

    #[cfg(test)]
    fn with_endpoint(mut self, endpoint: String) -> Self {
        self.endpoint = endpoint;
        self
    }

    fn transcribe_chunk(&self, audio: &Path) -> Result<ApiResponse> {
        http::send_checked(
            || {
                let bytes = std::fs::read(audio).context("reading audio chunk for upload")?;
                let part = reqwest::blocking::multipart::Part::bytes(bytes)
                    .file_name("audio.mp3")
                    .mime_str("audio/mpeg")
                    .context("building multipart file part")?;
                let mut form = reqwest::blocking::multipart::Form::new()
                    .part("file", part)
                    .text("model", self.model)
                    .text("response_format", "verbose_json")
                    // Words as well as segments: cues are cut where the speech pauses.
                    .text("timestamp_granularities[]", "word")
                    .text("timestamp_granularities[]", "segment");
                if let Some(language) = &self.language {
                    form = form.text("language", language.clone());
                }
                Ok(http::client()
                    .post(&self.endpoint)
                    .bearer_auth(self.api_key.as_str())
                    .multipart(form))
            },
            self.provider,
        )?
        .json()
        .with_context(|| format!("parsing {} response", self.provider))
    }
}

#[derive(Debug, Deserialize)]
struct ApiSegment {
    start: f64,
    end: f64,
    text: String,
}

#[derive(Debug, Deserialize)]
struct ApiWord {
    word: String,
    start: f64,
    end: f64,
}

#[derive(Debug, Deserialize)]
struct ApiResponse {
    language: String,
    segments: Vec<ApiSegment>,
    /// Present when word timestamps were requested and the provider supports them.
    #[serde(default)]
    words: Vec<ApiWord>,
}

/// Turns the `verbose_json` response into a transcript, shifting the timings by `offset`
/// seconds (start of the chunk in the whole file). Every word goes to the segment it
/// starts in, so the cues can be cut on real word times. Separate from `transcribe` so it
/// can be tested without a network call.
fn to_transcript(parsed: ApiResponse, offset: f64) -> Transcript {
    let mut per_segment: Vec<Vec<TimedWord>> = vec![Vec::new(); parsed.segments.len()];
    for word in &parsed.words {
        let owner = parsed
            .segments
            .iter()
            .rposition(|s| s.start <= word.start + 0.05)
            .unwrap_or(0);
        if let Some(words) = per_segment.get_mut(owner) {
            words.push(TimedWord {
                text: word.word.trim().to_string(),
                start: word.start + offset,
                end: word.end + offset,
            });
        }
    }
    let segments = parsed
        .segments
        .iter()
        .zip(per_segment)
        .flat_map(|(s, words)| {
            let words: Vec<TimedWord> = words.into_iter().filter(|w| !w.text.is_empty()).collect();
            cues_from_words(
                s.start + offset,
                s.end + offset,
                s.text.trim(),
                words,
                DEFAULT_LIMITS,
            )
        })
        .collect();
    Transcript {
        language: lang::to_code(&parsed.language),
        segments,
    }
}

impl SttEngine for OpenAiCompatibleWhisperEngine {
    fn audio_format(&self) -> AudioFormat {
        AudioFormat::Mp3
    }

    fn transcribe(
        &self,
        audio: &Path,
        mut on_progress: Box<dyn FnMut(f32) + Send>,
    ) -> Result<Transcript> {
        let total = audio_extract::media_duration_secs(audio)?;
        // Cuts fall in a pause, not in the middle of a word.
        let silences = if total > self.chunk_secs {
            audio_extract::find_silences(audio)
        } else {
            Vec::new()
        };
        let chunks = audio_extract::plan_chunks(total, self.chunk_secs, &silences);
        let scratch = tempfile::tempdir().context("creating temporary directory")?;

        let mut language = "unknown".to_string();
        let mut segments = Vec::new();
        for (i, (start, len)) in chunks.iter().enumerate() {
            cancel::check()?;
            on_progress(i as f32 / chunks.len() as f32);

            let part = if chunks.len() == 1 {
                // The whole file is under the size limit: sent as it is.
                to_transcript(self.transcribe_chunk(audio)?, 0.0)
            } else {
                let out = scratch.path().join("chunk.mp3");
                audio_extract::cut_slice(audio, *start, *len, &out)?;
                to_transcript(self.transcribe_chunk(&out)?, *start)
            };

            if language == "unknown" {
                language = part.language;
            }
            segments.extend(part.segments);
        }
        on_progress(1.0);

        Ok(Transcript { language, segments })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::testserver::serve;

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
        to_transcript(serde_json::from_str(json).unwrap(), 0.0)
    }

    #[test]
    fn a_response_becomes_a_transcript() {
        let t = transcript_of(
            r#"{"language":"french","segments":[
                {"start":0.0,"end":1.5,"text":" Bonjour"},
                {"start":1.5,"end":3.0,"text":"le monde "}
            ]}"#,
        );

        // The API says "french", the rest of the app speaks ISO codes.
        assert_eq!(t.language, "fr");
        assert_eq!(t.segments.len(), 2);
        // Whisper prefixes its segments with a space: it is removed.
        assert_eq!(t.segments[0].text, "Bonjour");
        assert_eq!(t.segments[1].text, "le monde");
    }

    #[test]
    fn swedish_is_not_mistaken_for_swahili() {
        assert_eq!(
            transcript_of(r#"{"language":"swedish","segments":[]}"#).language,
            "sv"
        );
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
    fn a_later_chunk_is_shifted_to_its_place_in_the_file() {
        let parsed: ApiResponse = serde_json::from_str(
            r#"{"language":"en","segments":[{"start":1.0,"end":2.0,"text":"x"}]}"#,
        )
        .unwrap();

        let t = to_transcript(parsed, 1200.0);

        assert_eq!(t.segments[0].start, 1201.0);
        assert_eq!(t.segments[0].end, 1202.0);
    }

    #[test]
    fn a_response_missing_the_language_is_rejected() {
        assert!(serde_json::from_str::<ApiResponse>(r#"{"segments":[]}"#).is_err());
    }

    #[test]
    fn both_factories_keep_the_api_key() {
        assert_eq!(
            OpenAiCompatibleWhisperEngine::groq("k1").api_key.as_str(),
            "k1"
        );
        assert_eq!(
            OpenAiCompatibleWhisperEngine::openai("k2").api_key.as_str(),
            "k2"
        );
    }

    fn chunk_file() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.mp3");
        std::fs::write(&path, b"fake audio").unwrap();
        (dir, path)
    }

    #[test]
    fn a_chunk_is_posted_with_the_key_the_model_and_the_forced_language() {
        let mock = serve(vec![(
            200,
            r#"{"language":"english","segments":[]}"#.into(),
        )]);
        let engine = OpenAiCompatibleWhisperEngine::openai("sk-test")
            .with_endpoint(mock.url.clone())
            .with_language(Some("fr".into()));
        let (_dir, chunk) = chunk_file();

        engine.transcribe_chunk(&chunk).unwrap();

        let sent = mock.requests.lock().unwrap();
        let body = String::from_utf8_lossy(&sent[0].body).to_string();
        assert!(sent[0].headers.contains("authorization: Bearer sk-test"));
        assert!(body.contains("whisper-1"));
        assert!(body.contains("verbose_json"));
        assert!(body.contains("name=\"language\""));
        assert!(body.contains("fake audio"));
    }

    #[test]
    fn no_language_field_is_sent_when_it_is_left_to_detection() {
        let mock = serve(vec![(
            200,
            r#"{"language":"english","segments":[]}"#.into(),
        )]);
        let engine = OpenAiCompatibleWhisperEngine::groq("k").with_endpoint(mock.url.clone());
        let (_dir, chunk) = chunk_file();

        engine.transcribe_chunk(&chunk).unwrap();

        let sent = mock.requests.lock().unwrap();
        assert!(!String::from_utf8_lossy(&sent[0].body).contains("name=\"language\""));
    }

    #[test]
    fn an_api_failure_explains_itself() {
        let mock = serve(vec![(
            413,
            r#"{"error":{"message":"Maximum content size limit (26214400) exceeded"}}"#.into(),
        )]);
        let engine = OpenAiCompatibleWhisperEngine::openai("k").with_endpoint(mock.url.clone());
        let (_dir, chunk) = chunk_file();

        let err = engine.transcribe_chunk(&chunk).unwrap_err();
        let out = crate::errors::CommandError::from(err);

        assert_eq!(out.code, Some("file_too_large"));
        assert!(
            out.message.contains("Maximum content size"),
            "{}",
            out.message
        );
        assert!(out.message.contains("OpenAI"), "{}", out.message);
    }

    #[test]
    fn words_are_attached_to_the_segment_they_start_in_and_cut_the_cues() {
        let parsed: ApiResponse = serde_json::from_str(
            r#"{"language":"english",
                "segments":[{"start":0.0,"end":30.0,"text":" a b c d"}],
                "words":[{"word":"a","start":0.0,"end":0.5},{"word":"b","start":0.6,"end":1.0},
                         {"word":"c","start":9.0,"end":9.5},{"word":"d","start":9.6,"end":10.0}]}"#,
        )
        .unwrap();

        let t = to_transcript(parsed, 100.0);

        // The 8 s silence ends the first cue, which keeps its real end instead of 30 s.
        assert_eq!(t.segments.len(), 2);
        assert_eq!(t.segments[0].text, "a b");
        assert_eq!(t.segments[0].end, 101.0);
        assert_eq!(t.segments[1].start, 109.0);
    }

    #[test]
    fn a_word_before_the_first_segment_is_not_lost() {
        let parsed: ApiResponse = serde_json::from_str(
            r#"{"language":"en","segments":[{"start":1.0,"end":2.0,"text":"x y"}],
                "words":[{"word":"x","start":0.5,"end":0.9},{"word":"y","start":1.0,"end":1.5}]}"#,
        )
        .unwrap();

        let t = to_transcript(parsed, 0.0);

        assert_eq!(t.segments[0].text, "x y");
    }

    #[test]
    fn word_timestamps_are_requested() {
        let mock = serve(vec![(
            200,
            r#"{"language":"english","segments":[]}"#.into(),
        )]);
        let engine = OpenAiCompatibleWhisperEngine::openai("k").with_endpoint(mock.url.clone());
        let (_dir, chunk) = chunk_file();

        engine.transcribe_chunk(&chunk).unwrap();

        let sent = mock.requests.lock().unwrap();
        let body = String::from_utf8_lossy(&sent[0].body).to_string();
        assert_eq!(body.matches("timestamp_granularities[]").count(), 2);
    }

    #[test]
    fn a_rate_limited_chunk_is_sent_again() {
        let mock = serve(vec![
            (429, r#"{"error":"slow down"}"#.into()),
            (200, r#"{"language":"english","segments":[]}"#.into()),
        ]);
        let engine = OpenAiCompatibleWhisperEngine::groq("k").with_endpoint(mock.url.clone());
        let (_dir, chunk) = chunk_file();

        assert!(engine.transcribe_chunk(&chunk).is_ok());
        assert_eq!(mock.requests.lock().unwrap().len(), 2);
    }

    /// Real ffmpeg: a recording longer than a chunk is cut, each slice goes to the API
    /// alone, and the answers are put back on the file's timeline.
    #[test]
    fn a_long_recording_is_sent_in_chunks_and_the_times_are_put_back() {
        if !audio_extract::ffmpeg_path().is_absolute() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let wav = dir.path().join("tone.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&wav, spec).unwrap();
        for i in 0..16_000 * 5 {
            let t = i as f32 / 16_000.0;
            writer
                .write_sample(((t * 440.0 * std::f32::consts::TAU).sin() * 8000.0) as i16)
                .unwrap();
        }
        writer.finalize().unwrap();
        let mp3 = audio_extract::extract_audio(&wav, dir.path(), None, AudioFormat::Mp3).unwrap();
        let answer = (
            200,
            r#"{"language":"french","segments":[{"start":0.2,"end":0.9,"text":"Bonjour"}]}"#
                .to_string(),
        );
        let mock = serve(vec![answer.clone(), answer.clone(), answer]);
        let engine = OpenAiCompatibleWhisperEngine::openai("k")
            .with_endpoint(mock.url.clone())
            .with_chunk_secs(2.0);

        let t = engine.transcribe(&mp3.path, Box::new(|_| {})).unwrap();

        assert_eq!(mock.requests.lock().unwrap().len(), 3);
        assert_eq!(t.language, "fr");
        let starts: Vec<f64> = t.segments.iter().map(|s| s.start).collect();
        assert_eq!(starts.len(), 3);
        assert!((starts[0] - 0.2).abs() < 1e-9);
        assert!((starts[1] - 2.2).abs() < 0.1, "{starts:?}");
        assert!((starts[2] - 4.2).abs() < 0.1, "{starts:?}");
    }
}
