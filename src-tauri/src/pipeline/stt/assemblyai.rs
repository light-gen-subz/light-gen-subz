use std::path::Path;
use std::thread::sleep;
use std::time::Duration;
use zeroize::Zeroizing;

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use serde_json::json;

use super::{cues_from_words, lang, Segment, SttEngine, Transcript};
use crate::cancel;
use crate::http;
use crate::pipeline::audio_extract::{self, AudioFormat};
use crate::pipeline::segmentation::{TimedWord, DEFAULT_LIMITS};

const BASE_URL: &str = "https://api.assemblyai.com/v2";
const PROVIDER: &str = "AssemblyAI";
const POLL_INTERVAL: Duration = Duration::from_secs(3);
/// Shortest wait before giving up, whatever the length of the audio.
const MIN_WAIT_SECS: f64 = 600.0;

pub struct AssemblyAiEngine {
    api_key: Zeroizing<String>,
    base_url: String,
    poll_interval: Duration,
    language: Option<String>,
}

impl AssemblyAiEngine {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: Zeroizing::new(api_key.into()),
            base_url: BASE_URL.to_string(),
            poll_interval: POLL_INTERVAL,
            language: None,
        }
    }

    /// Forces the spoken language instead of letting AssemblyAI detect it.
    pub fn with_language(mut self, language: Option<String>) -> Self {
        self.language = language;
        self
    }

    #[cfg(test)]
    fn for_test(api_key: &str, base_url: String) -> Self {
        Self {
            api_key: Zeroizing::new(api_key.into()),
            base_url,
            poll_interval: Duration::from_millis(1),
            language: None,
        }
    }
}

#[derive(Deserialize)]
struct UploadResponse {
    upload_url: String,
}

#[derive(Deserialize)]
struct TranscriptCreated {
    id: String,
}

#[derive(Deserialize)]
struct TranscriptStatus {
    status: String,
    language_code: Option<String>,
    error: Option<String>,
}

#[derive(Deserialize)]
struct SentencesResponse {
    sentences: Vec<Sentence>,
}

#[derive(Deserialize)]
struct SentenceWord {
    text: String,
    start: i64,
    end: i64,
}

#[derive(Deserialize)]
struct Sentence {
    text: String,
    start: i64,
    end: i64,
    /// Time of every word: cues are cut where the speech pauses.
    #[serde(default)]
    words: Vec<SentenceWord>,
}

/// What to do after a status check. Extracted from the polling loop so the state machine
/// can be tested without waiting on the network.
#[derive(Debug, PartialEq, Eq)]
enum Poll {
    /// Done, with the detected language.
    Done(String),
    /// Failed on AssemblyAI's side, with its message.
    Failed(String),
    /// Still running: check again.
    Pending,
}

fn interpret_status(status: TranscriptStatus, fallback_language: &str) -> Poll {
    match status.status.as_str() {
        "completed" => Poll::Done(
            status
                .language_code
                .unwrap_or_else(|| fallback_language.to_string()),
        ),
        "error" => Poll::Failed(status.error.unwrap_or_else(|| "unknown error".to_string())),
        _ => Poll::Pending,
    }
}

/// Number of status checks to allow: at least ten minutes, and half the audio's length
/// for long files (AssemblyAI transcribes much faster than real time).
fn max_polls(audio_secs: f64, interval: Duration) -> u32 {
    let wait = (audio_secs * 0.5).max(MIN_WAIT_SECS);
    (wait / interval.as_secs_f64().max(0.001)).ceil() as u32
}

/// Progress reported at the n-th check: the polling phase takes the 0.2 – 0.8 slice of
/// the bar.
fn poll_progress(i: u32, max: u32) -> f32 {
    0.2 + 0.6 * (i as f32 / max.max(1) as f32).min(1.0)
}

/// Turns sentences (milliseconds) into segments (seconds).
fn to_segments(sentences: SentencesResponse) -> Vec<Segment> {
    sentences
        .sentences
        .into_iter()
        .flat_map(|s| {
            let words = s
                .words
                .into_iter()
                .map(|w| TimedWord {
                    text: w.text.trim().to_string(),
                    start: w.start as f64 / 1000.0,
                    end: w.end as f64 / 1000.0,
                })
                .filter(|w| !w.text.is_empty())
                .collect();
            cues_from_words(
                s.start as f64 / 1000.0,
                s.end as f64 / 1000.0,
                s.text.trim(),
                words,
                DEFAULT_LIMITS,
            )
        })
        .collect()
}

/// Body of the job creation: forced language or automatic detection.
fn job_body(audio_url: &str, language: Option<&str>) -> serde_json::Value {
    match language {
        Some(code) => json!({ "audio_url": audio_url, "language_code": code }),
        None => json!({ "audio_url": audio_url, "language_detection": true }),
    }
}

impl SttEngine for AssemblyAiEngine {
    fn audio_format(&self) -> AudioFormat {
        AudioFormat::Mp3
    }

    fn transcribe(
        &self,
        audio: &Path,
        mut on_progress: Box<dyn FnMut(f32) + Send>,
    ) -> Result<Transcript> {
        let client = http::client();
        let base = &self.base_url;

        on_progress(0.05);
        let total = audio_extract::media_duration_secs(audio)?;

        let upload: UploadResponse = http::send_checked(
            || {
                let file = std::fs::File::open(audio).context("opening audio for upload")?;
                Ok(client
                    .post(format!("{base}/upload"))
                    .header("authorization", self.api_key.as_str())
                    .body(file))
            },
            PROVIDER,
        )?
        .json()
        .context("parsing AssemblyAI upload response")?;

        cancel::check()?;
        on_progress(0.2);
        let created: TranscriptCreated = http::send_checked(
            || {
                Ok(client
                    .post(format!("{base}/transcript"))
                    .header("authorization", self.api_key.as_str())
                    .json(&job_body(&upload.upload_url, self.language.as_deref())))
            },
            PROVIDER,
        )?
        .json()
        .context("parsing AssemblyAI transcript creation response")?;

        let status_url = format!("{base}/transcript/{}", created.id);
        let result = self.wait_and_fetch(&status_url, total, &mut on_progress);

        // Best effort: the transcript stays on AssemblyAI's servers until deleted, and
        // the provider only removes a finished one — after a cancellation or a timeout
        // the job may still be running and this is refused. The uploaded audio has no
        // delete endpoint at all (it expires on their side).
        let _ = client
            .delete(&status_url)
            .header("authorization", self.api_key.as_str())
            .send();

        result
    }
}

impl AssemblyAiEngine {
    fn wait_and_fetch(
        &self,
        status_url: &str,
        audio_secs: f64,
        on_progress: &mut Box<dyn FnMut(f32) + Send>,
    ) -> Result<Transcript> {
        let client = http::client();
        let max = max_polls(audio_secs, self.poll_interval);

        let mut language = self
            .language
            .clone()
            .unwrap_or_else(|| "unknown".to_string());
        let mut completed = false;
        for i in 0..max {
            cancel::check()?;
            sleep(self.poll_interval);
            let status: TranscriptStatus = http::send_checked(
                || {
                    Ok(client
                        .get(status_url)
                        .header("authorization", self.api_key.as_str()))
                },
                PROVIDER,
            )?
            .json()
            .context("parsing AssemblyAI status response")?;

            on_progress(poll_progress(i, max));

            match interpret_status(status, &language) {
                Poll::Done(detected) => {
                    language = detected;
                    completed = true;
                    break;
                }
                Poll::Failed(reason) => bail!("AssemblyAI transcription failed: {reason}"),
                Poll::Pending => continue,
            }
        }
        anyhow::ensure!(completed, "AssemblyAI transcription timed out");

        on_progress(0.85);
        let sentences: SentencesResponse = http::send_checked(
            || {
                Ok(client
                    .get(format!("{status_url}/sentences"))
                    .header("authorization", self.api_key.as_str()))
            },
            PROVIDER,
        )?
        .json()
        .context("parsing AssemblyAI sentences response")?;

        on_progress(1.0);

        Ok(Transcript {
            language: lang::to_code(&language),
            segments: to_segments(sentences),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::testserver::serve;

    #[test]
    fn parses_the_upload_url() {
        let parsed: UploadResponse =
            serde_json::from_str(r#"{"upload_url":"https://cdn/upload/abc"}"#).unwrap();
        assert_eq!(parsed.upload_url, "https://cdn/upload/abc");
    }

    #[test]
    fn parses_the_created_transcript_id() {
        let parsed: TranscriptCreated = serde_json::from_str(r#"{"id":"tr-1"}"#).unwrap();
        assert_eq!(parsed.id, "tr-1");
    }

    #[test]
    fn parses_a_completed_status_with_its_language() {
        let parsed: TranscriptStatus =
            serde_json::from_str(r#"{"status":"completed","language_code":"fr"}"#).unwrap();
        assert_eq!(parsed.status, "completed");
        assert_eq!(parsed.language_code.as_deref(), Some("fr"));
        assert!(parsed.error.is_none());
    }

    #[test]
    fn parses_an_errored_status() {
        let parsed: TranscriptStatus =
            serde_json::from_str(r#"{"status":"error","error":"audio too short"}"#).unwrap();
        assert_eq!(parsed.status, "error");
        assert_eq!(parsed.error.as_deref(), Some("audio too short"));
        assert!(parsed.language_code.is_none());
    }

    #[test]
    fn parses_the_sentence_list() {
        let parsed: SentencesResponse =
            serde_json::from_str(r#"{"sentences":[{"start":0,"end":1500,"text":"Bonjour"}]}"#)
                .unwrap();
        assert_eq!(parsed.sentences.len(), 1);
        assert_eq!(parsed.sentences[0].text, "Bonjour");
    }

    #[test]
    fn parses_an_empty_sentence_list() {
        let parsed: SentencesResponse = serde_json::from_str(r#"{"sentences":[]}"#).unwrap();
        assert!(parsed.sentences.is_empty());
    }

    fn status(json: &str) -> TranscriptStatus {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn a_completed_job_reports_its_detected_language() {
        let poll = interpret_status(
            status(r#"{"status":"completed","language_code":"fr"}"#),
            "unknown",
        );

        assert_eq!(poll, Poll::Done("fr".into()));
    }

    #[test]
    fn a_completed_job_without_a_language_keeps_the_fallback() {
        let poll = interpret_status(status(r#"{"status":"completed"}"#), "unknown");

        assert_eq!(poll, Poll::Done("unknown".into()));
    }

    #[test]
    fn a_failed_job_carries_its_reason() {
        let poll = interpret_status(
            status(r#"{"status":"error","error":"audio too short"}"#),
            "unknown",
        );

        assert_eq!(poll, Poll::Failed("audio too short".into()));
    }

    #[test]
    fn a_failed_job_without_a_reason_still_reports_a_failure() {
        let poll = interpret_status(status(r#"{"status":"error"}"#), "unknown");

        assert_eq!(poll, Poll::Failed("unknown error".into()));
    }

    #[test]
    fn the_intermediate_statuses_keep_the_loop_going() {
        for s in ["queued", "processing", "something-new"] {
            let json = format!(r#"{{"status":"{s}"}}"#);
            assert_eq!(
                interpret_status(status(&json), "unknown"),
                Poll::Pending,
                "{s}"
            );
        }
    }

    #[test]
    fn the_polling_progress_spans_the_expected_slice() {
        assert!((poll_progress(0, 200) - 0.2).abs() < 1e-6);
        assert!((poll_progress(200, 200) - 0.8).abs() < 1e-6);
        // Monotonic and never beyond the allotted slice.
        assert!(poll_progress(1, 200) > poll_progress(0, 200));
        assert!(poll_progress(400, 200) <= 0.8 + 1e-6);
    }

    #[test]
    fn a_short_file_waits_at_least_ten_minutes() {
        assert_eq!(max_polls(30.0, POLL_INTERVAL), 200);
    }

    #[test]
    fn a_long_file_waits_longer() {
        // 4 h of audio: half of it is 2 h = 7200 s.
        assert_eq!(max_polls(4.0 * 3600.0, POLL_INTERVAL), 2400);
    }

    #[test]
    fn sentences_in_milliseconds_become_segments_in_seconds() {
        let sentences: SentencesResponse = serde_json::from_str(
            r#"{"sentences":[{"start":0,"end":1500,"text":"  Bonjour "},
                             {"start":1500,"end":3250,"text":"le monde"}]}"#,
        )
        .unwrap();

        let segments = to_segments(sentences);

        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].start, 0.0);
        assert_eq!(segments[0].end, 1.5);
        assert_eq!(segments[0].text, "Bonjour");
        assert_eq!(segments[1].end, 3.25);
    }

    #[test]
    fn an_empty_sentence_list_yields_no_segment() {
        let sentences: SentencesResponse = serde_json::from_str(r#"{"sentences":[]}"#).unwrap();

        assert!(to_segments(sentences).is_empty());
    }

    #[test]
    fn the_job_detects_the_language_unless_one_is_forced() {
        assert_eq!(job_body("u", None)["language_detection"], true);
        let forced = job_body("u", Some("fr"));
        assert_eq!(forced["language_code"], "fr");
        assert!(forced.get("language_detection").is_none());
    }

    #[test]
    fn a_finished_job_is_collected_through_the_status_and_sentences_endpoints() {
        let mock = serve(vec![
            (200, r#"{"status":"processing"}"#.into()),
            (200, r#"{"status":"completed","language_code":"fr"}"#.into()),
            (
                200,
                r#"{"sentences":[{"start":0,"end":1500,"text":"Bonjour"}]}"#.into(),
            ),
        ]);
        let engine = AssemblyAiEngine::for_test("k", mock.url.clone());
        let mut progress: Box<dyn FnMut(f32) + Send> = Box::new(|_| {});

        let t = engine
            .wait_and_fetch(&format!("{}/transcript/1", mock.url), 30.0, &mut progress)
            .unwrap();

        assert_eq!(t.language, "fr");
        assert_eq!(t.segments[0].text, "Bonjour");
        let sent = mock.requests.lock().unwrap();
        assert_eq!(sent.len(), 3);
        assert!(sent[2].request_line.contains("/sentences"));
    }

    #[test]
    fn a_job_that_fails_on_their_side_is_reported_with_its_reason() {
        let mock = serve(vec![(
            200,
            r#"{"status":"error","error":"audio too short"}"#.into(),
        )]);
        let engine = AssemblyAiEngine::for_test("k", mock.url.clone());
        let mut progress: Box<dyn FnMut(f32) + Send> = Box::new(|_| {});

        let err = engine
            .wait_and_fetch(&format!("{}/transcript/1", mock.url), 30.0, &mut progress)
            .unwrap_err();

        assert!(err.to_string().contains("audio too short"), "{err}");
    }

    #[test]
    fn a_sentence_with_words_is_cut_at_a_real_pause() {
        let sentences: SentencesResponse = serde_json::from_str(
            r#"{"sentences":[{"start":0,"end":12000,"text":"Hello there. Again.",
                "words":[{"text":"Hello","start":0,"end":400},{"text":"there.","start":500,"end":900},
                         {"text":"Again.","start":10000,"end":10600}]}]}"#,
        )
        .unwrap();

        let segments = to_segments(sentences);

        let texts: Vec<_> = segments.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(texts, ["Hello there.", "Again."]);
        assert_eq!(segments[1].start, 10.0);
    }
}
