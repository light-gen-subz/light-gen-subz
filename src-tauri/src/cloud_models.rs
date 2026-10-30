//! The provider models the app asks for, in one place: when a provider retires a model,
//! this is the only file to change (and the test below lists what depends on it).

pub const OPENAI_TRANSCRIPTION: &str = "whisper-1";
pub const GROQ_TRANSCRIPTION: &str = "whisper-large-v3-turbo";
pub const DEEPGRAM_TRANSCRIPTION: &str = "nova-2";
pub const OPENAI_TRANSLATION: &str = "gpt-4o-mini";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_model_is_named() {
        for model in [
            OPENAI_TRANSCRIPTION,
            GROQ_TRANSCRIPTION,
            DEEPGRAM_TRANSCRIPTION,
            OPENAI_TRANSLATION,
        ] {
            assert!(!model.trim().is_empty());
            assert!(!model.contains(' '));
        }
    }
}
