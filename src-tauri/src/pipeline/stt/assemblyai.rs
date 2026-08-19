use std::path::Path;
use std::thread::sleep;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use serde_json::json;

use super::{Segment, SttEngine, Transcript};

const BASE_URL: &str = "https://api.assemblyai.com/v2";
const POLL_INTERVAL: Duration = Duration::from_secs(3);
const MAX_POLLS: u32 = 200; // ~10 minutes

pub struct AssemblyAiEngine {
    api_key: String,
}

impl AssemblyAiEngine {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
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
struct Sentence {
    text: String,
    start: i64,
    end: i64,
}

/// Suite à donner après une interrogation de statut. Extrait de la boucle de polling
/// pour que la machine à états soit testable sans attendre le réseau.
#[derive(Debug, PartialEq, Eq)]
enum Poll {
    /// Terminé, avec la langue détectée.
    Done(String),
    /// Échec côté AssemblyAI, avec son message.
    Failed(String),
    /// Toujours en cours : on repasse.
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

/// Progression rapportée à la n-ième interrogation : la phase de polling occupe la
/// tranche 0,2 – 0,8 de la barre.
fn poll_progress(i: u32) -> f32 {
    0.2 + 0.6 * (i as f32 / MAX_POLLS as f32).min(1.0)
}

/// Traduit les phrases (millisecondes) en segments (secondes).
fn to_segments(sentences: SentencesResponse) -> Vec<Segment> {
    sentences
        .sentences
        .into_iter()
        .map(|s| Segment {
            start: s.start as f64 / 1000.0,
            end: s.end as f64 / 1000.0,
            text: s.text.trim().to_string(),
        })
        .collect()
}

impl SttEngine for AssemblyAiEngine {
    fn transcribe(
        &self,
        wav_path: &Path,
        mut on_progress: Box<dyn FnMut(f32) + Send>,
    ) -> Result<Transcript> {
        let client = reqwest::blocking::Client::new();

        on_progress(0.05);
        let file_bytes = std::fs::read(wav_path).context("reading WAV file for upload")?;
        let upload: UploadResponse = client
            .post(format!("{BASE_URL}/upload"))
            .header("authorization", &self.api_key)
            .body(file_bytes)
            .send()
            .context("uploading audio to AssemblyAI")?
            .error_for_status()
            .context("AssemblyAI upload returned an error")?
            .json()
            .context("parsing AssemblyAI upload response")?;

        on_progress(0.2);
        let created: TranscriptCreated = client
            .post(format!("{BASE_URL}/transcript"))
            .header("authorization", &self.api_key)
            .json(&json!({
                "audio_url": upload.upload_url,
                "language_detection": true,
            }))
            .send()
            .context("creating AssemblyAI transcript job")?
            .error_for_status()
            .context("AssemblyAI transcript creation returned an error")?
            .json()
            .context("parsing AssemblyAI transcript creation response")?;

        let status_url = format!("{BASE_URL}/transcript/{}", created.id);
        let mut language = "unknown".to_string();
        let mut completed = false;
        for i in 0..MAX_POLLS {
            sleep(POLL_INTERVAL);
            let status: TranscriptStatus = client
                .get(&status_url)
                .header("authorization", &self.api_key)
                .send()
                .context("polling AssemblyAI transcript status")?
                .error_for_status()
                .context("AssemblyAI status check returned an error")?
                .json()
                .context("parsing AssemblyAI status response")?;

            on_progress(poll_progress(i));

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
        let sentences: SentencesResponse = client
            .get(format!("{status_url}/sentences"))
            .header("authorization", &self.api_key)
            .send()
            .context("fetching AssemblyAI sentences")?
            .error_for_status()
            .context("AssemblyAI sentences endpoint returned an error")?
            .json()
            .context("parsing AssemblyAI sentences response")?;

        on_progress(1.0);

        Ok(Transcript {
            language,
            segments: to_segments(sentences),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!((poll_progress(0) - 0.2).abs() < 1e-6);
        assert!((poll_progress(MAX_POLLS) - 0.8).abs() < 1e-6);
        // Monotone et jamais au-delà de la tranche allouée.
        assert!(poll_progress(1) > poll_progress(0));
        assert!(poll_progress(MAX_POLLS * 2) <= 0.8 + 1e-6);
    }

    #[test]
    fn the_poll_budget_covers_about_ten_minutes() {
        let total = POLL_INTERVAL * MAX_POLLS;

        assert_eq!(total.as_secs(), 600);
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
}
