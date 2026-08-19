pub mod assemblyai;
pub mod deepgram;
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
    fn transcribe(
        &self,
        wav_path: &std::path::Path,
        on_progress: Box<dyn FnMut(f32) + Send>,
    ) -> anyhow::Result<Transcript>;
}
