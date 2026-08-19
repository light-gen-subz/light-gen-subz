use std::path::Path;

use anyhow::{Context, Result};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use super::{Segment, SttEngine, Transcript};

pub struct LocalWhisperEngine {
    model_path: std::path::PathBuf,
}

impl LocalWhisperEngine {
    pub fn new(model_path: impl Into<std::path::PathBuf>) -> Self {
        Self {
            model_path: model_path.into(),
        }
    }
}

impl SttEngine for LocalWhisperEngine {
    fn transcribe(
        &self,
        wav_path: &Path,
        mut on_progress: Box<dyn FnMut(f32) + Send>,
    ) -> Result<Transcript> {
        let samples = read_wav_mono_f32(wav_path)?;

        let model_path = self
            .model_path
            .to_str()
            .context("model path is not valid UTF-8")?;
        let ctx = WhisperContext::new_with_params(model_path, WhisperContextParameters::default())
            .context("failed to load whisper model")?;
        let mut state = ctx
            .create_state()
            .context("failed to create whisper state")?;

        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(None); // auto-detect language
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_progress_callback_safe(move |percent: i32| {
            on_progress(percent as f32 / 100.0);
        });

        state
            .full(params, &samples)
            .context("whisper transcription failed")?;

        let num_segments = state.full_n_segments();
        let mut segments = Vec::with_capacity(num_segments as usize);
        for i in 0..num_segments {
            let seg = state.get_segment(i).context("reading segment")?;
            let text = seg.to_str().context("reading segment text")?;
            segments.push(Segment {
                start: seg.start_timestamp() as f64 / 100.0,
                end: seg.end_timestamp() as f64 / 100.0,
                text: text.trim().to_string(),
            });
        }

        let language = whisper_rs::get_lang_str(state.full_lang_id_from_state())
            .unwrap_or("unknown")
            .to_string();

        Ok(Transcript { language, segments })
    }
}

/// Reads a 16-bit mono WAV (produced by ffmpeg) and returns f32 samples normalized to [-1, 1].
fn read_wav_mono_f32(path: &Path) -> Result<Vec<f32>> {
    let mut reader = hound::WavReader::open(path).context("opening WAV file")?;
    let spec = reader.spec();
    anyhow::ensure!(
        spec.channels == 1 && spec.sample_rate == 16000,
        "WAV must be mono 16kHz (got: {} channels, {}Hz)",
        spec.channels,
        spec.sample_rate
    );

    let samples: Result<Vec<f32>, _> = match spec.sample_format {
        hound::SampleFormat::Int => reader
            .samples::<i16>()
            .map(|s| s.map(|v| v as f32 / i16::MAX as f32))
            .collect(),
        hound::SampleFormat::Float => reader.samples::<f32>().collect(),
    };
    samples.context("reading WAV samples")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Écrit un WAV avec la spec demandée et renvoie son chemin (le dossier temporaire
    /// est renvoyé aussi pour rester vivant le temps du test).
    fn write_wav(
        channels: u16,
        sample_rate: u32,
        format: hound::SampleFormat,
        bits: u16,
        samples: &[i32],
    ) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audio.wav");
        let spec = hound::WavSpec {
            channels,
            sample_rate,
            bits_per_sample: bits,
            sample_format: format,
        };
        let mut writer = hound::WavWriter::create(&path, spec).unwrap();
        for &s in samples {
            match format {
                hound::SampleFormat::Int => writer.write_sample(s as i16).unwrap(),
                hound::SampleFormat::Float => writer.write_sample(s as f32).unwrap(),
            }
        }
        writer.finalize().unwrap();
        (dir, path)
    }

    #[test]
    fn a_mono_16k_wav_is_normalised_to_minus_one_one() {
        let (_dir, path) = write_wav(
            1,
            16_000,
            hound::SampleFormat::Int,
            16,
            &[0, i16::MAX as i32, i16::MIN as i32 + 1],
        );

        let samples = read_wav_mono_f32(&path).unwrap();

        assert_eq!(samples.len(), 3);
        assert!((samples[0] - 0.0).abs() < 1e-6);
        assert!((samples[1] - 1.0).abs() < 1e-4);
        assert!((samples[2] + 1.0).abs() < 1e-4);
    }

    #[test]
    fn a_float_wav_is_read_as_is() {
        let (_dir, path) = write_wav(1, 16_000, hound::SampleFormat::Float, 32, &[0, 1, -1]);

        let samples = read_wav_mono_f32(&path).unwrap();

        assert_eq!(samples, vec![0.0, 1.0, -1.0]);
    }

    #[test]
    fn an_empty_recording_yields_no_sample() {
        let (_dir, path) = write_wav(1, 16_000, hound::SampleFormat::Int, 16, &[]);

        assert!(read_wav_mono_f32(&path).unwrap().is_empty());
    }

    #[test]
    fn a_stereo_wav_is_refused() {
        let (_dir, path) = write_wav(2, 16_000, hound::SampleFormat::Int, 16, &[0, 0]);

        let err = read_wav_mono_f32(&path).unwrap_err();

        assert!(err.to_string().contains("mono 16kHz"), "{err}");
        assert!(err.to_string().contains("2 channels"), "{err}");
    }

    #[test]
    fn a_wrong_sample_rate_is_refused() {
        let (_dir, path) = write_wav(1, 44_100, hound::SampleFormat::Int, 16, &[0]);

        let err = read_wav_mono_f32(&path).unwrap_err();

        assert!(err.to_string().contains("44100Hz"), "{err}");
    }

    #[test]
    fn a_missing_file_is_reported() {
        let err = read_wav_mono_f32(std::path::Path::new("/nowhere/audio.wav")).unwrap_err();

        assert!(err.to_string().contains("opening WAV file"), "{err}");
    }

    #[test]
    fn a_file_that_is_not_a_wav_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fake.wav");
        std::fs::write(&path, b"pas un wav").unwrap();

        assert!(read_wav_mono_f32(&path).is_err());
    }

    #[test]
    fn the_engine_keeps_the_model_path_it_was_given() {
        let engine = LocalWhisperEngine::new("/models/ggml-small.bin");

        assert_eq!(
            engine.model_path,
            std::path::PathBuf::from("/models/ggml-small.bin")
        );
    }
}
