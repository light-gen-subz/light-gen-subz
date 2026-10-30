use crate::pipeline::audio_extract::AudioFormat;
use crate::pipeline::segmentation::{self, CueLimits, TimedWord};

pub mod assemblyai;
pub mod deepgram;
pub mod lang;
pub mod local_whisper;
pub mod openai_compatible;

pub use assemblyai::AssemblyAiEngine;
pub use deepgram::DeepgramEngine;
pub use local_whisper::LocalWhisperEngine;
pub use openai_compatible::OpenAiCompatibleWhisperEngine;

#[derive(Debug, Clone, serde::Serialize)]
pub struct Segment {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Transcript {
    pub language: String,
    pub segments: Vec<Segment>,
}

pub trait SttEngine {
    /// What the engine reads: a WAV, or audio already compressed for upload.
    fn audio_format(&self) -> AudioFormat {
        AudioFormat::Wav
    }

    fn transcribe(
        &self,
        audio: &std::path::Path,
        on_progress: Box<dyn FnMut(f32) + Send>,
    ) -> anyhow::Result<Transcript>;
}

/// The cues of one segment: cut on the real time of its words when the engine gave them,
/// otherwise the segment as it is (`build_cues` then splits it in proportion).
pub fn cues_from_words(
    start: f64,
    end: f64,
    text: &str,
    words: Vec<TimedWord>,
    limits: CueLimits,
) -> Vec<Segment> {
    if words.len() > 1 {
        segmentation::split_timed(&words, limits)
    } else {
        vec![Segment {
            start,
            end,
            text: text.to_string(),
        }]
    }
}
