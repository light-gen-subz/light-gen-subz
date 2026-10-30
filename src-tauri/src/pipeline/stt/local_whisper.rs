use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::{Context, Result};
use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperVadContext,
    WhisperVadContextParams, WhisperVadParams,
};

use super::{cues_from_words, Segment, SttEngine, Transcript};
use crate::cancel;
use crate::pipeline::segmentation::{is_unspaced, CueLimits, TimedWord, DEFAULT_LIMITS};

const SAMPLE_RATE: usize = 16_000;
/// Speech separated by less than this is transcribed in one go: whisper reads the context.
const BLOCK_GAP_SECS: f64 = 1.5;
/// Longest run of speech handed to whisper at once (it cuts its own 30 s windows inside).
const MAX_BLOCK_SECS: f64 = 120.0;
/// Margin kept around a block so the first and last words are not clipped.
const BLOCK_PAD_SECS: f64 = 0.25;
/// Shorter than this there is nothing to recognise.
const MIN_BLOCK_SECS: f64 = 0.1;

pub struct LocalWhisperEngine {
    model_path: PathBuf,
    /// Silero voice-activity model: silence and music are skipped instead of being given
    /// invented text.
    vad_model_path: Option<PathBuf>,
    /// Spoken language forced by the user, `None` to detect it.
    language: Option<String>,
    /// What a cue may hold: segments beyond it are split on real word times.
    limits: CueLimits,
}

impl LocalWhisperEngine {
    pub fn new(model_path: impl Into<PathBuf>) -> Self {
        Self {
            model_path: model_path.into(),
            vad_model_path: None,
            language: None,
            limits: DEFAULT_LIMITS,
        }
    }

    pub fn with_vad(mut self, vad_model_path: Option<PathBuf>) -> Self {
        self.vad_model_path = vad_model_path;
        self
    }

    #[cfg(test)]
    pub fn with_limits(mut self, limits: CueLimits) -> Self {
        self.limits = limits;
        self
    }

    pub fn with_language(mut self, language: Option<String>) -> Self {
        self.language = language;
        self
    }

    /// The loaded model is kept between runs: reading hundreds of megabytes of weights
    /// again for every file is wasted time. [`release_cache`] frees it.
    fn context(&self) -> Result<Arc<WhisperContext>> {
        let mut cache = cache()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some((path, ctx)) = cache.as_ref() {
            if *path == self.model_path {
                return Ok(Arc::clone(ctx));
            }
        }
        let model_path = self
            .model_path
            .to_str()
            .context("model path is not valid UTF-8")?;
        let ctx = Arc::new(
            WhisperContext::new_with_params(model_path, WhisperContextParameters::default())
                .context("failed to load whisper model")?,
        );
        *cache = Some((self.model_path.clone(), Arc::clone(&ctx)));
        Ok(ctx)
    }
}

type Cached = Option<(PathBuf, Arc<WhisperContext>)>;

fn cache() -> &'static Mutex<Cached> {
    static CACHE: OnceLock<Mutex<Cached>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

/// Frees the model kept in memory (a few hundred megabytes), e.g. when the user switches
/// to a cloud engine.
pub fn release_cache() {
    *cache().lock().unwrap_or_else(|p| p.into_inner()) = None;
}

/// Whether `bytes` stop in the middle of a UTF-8 character.
fn ends_mid_character(bytes: &[u8]) -> bool {
    std::str::from_utf8(bytes).is_err_and(|e| e.error_len().is_none())
}

fn first_char(bytes: &[u8]) -> Option<char> {
    let valid = std::str::from_utf8(bytes).map_or_else(|e| &bytes[..e.valid_up_to()], |_| bytes);
    std::str::from_utf8(valid).ok()?.chars().next()
}

/// Whether a token starts a new word after `previous`: it begins with a space, or it is a
/// character of a script written without spaces (each one may end a line).
fn starts_a_word(previous: &[u8], token: &[u8]) -> bool {
    if token.starts_with(b" ") {
        return true;
    }
    let after_unspaced = std::str::from_utf8(previous)
        .ok()
        .and_then(|p| p.chars().last())
        .is_some_and(is_unspaced);
    after_unspaced || first_char(token).is_some_and(is_unspaced)
}

/// Groups the tokens of a segment into words with their own times. A token that starts
/// with a space, or that is a character of an unspaced script, begins a new word; special
/// tokens (`[_BEG_]`, `<|...|>`) are skipped. whisper cuts a rare character into several
/// tokens that are not valid UTF-8 on their own, so the *bytes* are joined and decoded once
/// the word is complete. Times are in centiseconds, made monotone and kept inside the
/// segment.
pub fn words_from_tokens(
    tokens: &[(Vec<u8>, i64, i64)],
    seg_start: f64,
    seg_end: f64,
) -> Vec<TimedWord> {
    struct Word {
        bytes: Vec<u8>,
        start: f64,
        end: f64,
    }
    let mut words: Vec<Word> = Vec::new();
    for (bytes, t0, t1) in tokens {
        if bytes.starts_with(b"[_")
            || bytes.starts_with(b"<|")
            || bytes.iter().all(u8::is_ascii_whitespace)
        {
            continue;
        }
        let start = (*t0 as f64 / 100.0).clamp(seg_start, seg_end);
        let end = (*t1 as f64 / 100.0).clamp(start, seg_end);
        match words.last_mut() {
            Some(last) if ends_mid_character(&last.bytes) || !starts_a_word(&last.bytes, bytes) => {
                last.bytes.extend_from_slice(bytes);
                last.end = end.max(last.end);
            }
            previous => {
                let floor = previous.map_or(seg_start, |w| w.end);
                words.push(Word {
                    bytes: bytes.clone(),
                    start: start.max(floor),
                    end,
                });
            }
        }
    }
    words
        .into_iter()
        .filter_map(|w| {
            let text = String::from_utf8_lossy(&w.bytes).trim().to_string();
            (!text.is_empty()).then(|| TimedWord {
                text,
                start: w.start,
                end: w.end.max(w.start),
            })
        })
        .collect()
}

/// Groups the speech the VAD found (seconds) into blocks to transcribe: speech closer than
/// [`BLOCK_GAP_SECS`] goes together, up to [`MAX_BLOCK_SECS`], and every block gets a small
/// margin that never reaches into the neighbouring block.
pub fn speech_blocks(speech: &[(f64, f64)], total_secs: f64) -> Vec<(f64, f64)> {
    let mut blocks: Vec<(f64, f64)> = Vec::new();
    for &(start, end) in speech {
        match blocks.last_mut() {
            Some(last) if start - last.1 <= BLOCK_GAP_SECS && end - last.0 <= MAX_BLOCK_SECS => {
                last.1 = last.1.max(end);
            }
            _ => blocks.push((start, end)),
        }
    }
    let ends: Vec<f64> = blocks.iter().map(|b| b.1).collect();
    let starts: Vec<f64> = blocks.iter().map(|b| b.0).collect();
    blocks
        .iter()
        .enumerate()
        .map(|(i, &(start, end))| {
            let before = if i == 0 {
                start
            } else {
                (start - ends[i - 1]) / 2.0
            };
            let after = if i + 1 == blocks.len() {
                total_secs - end
            } else {
                (starts[i + 1] - end) / 2.0
            };
            (
                (start - before.min(BLOCK_PAD_SECS)).max(0.0),
                (end + after.min(BLOCK_PAD_SECS)).min(total_secs),
            )
        })
        .collect()
}

/// Audio the VAD looks at in one go: bounded, so an hour of recording is never held in
/// memory as floats.
const VAD_WINDOW_SECS: f64 = 600.0;
/// Overlap between two windows, so speech cut by a boundary is found on both sides.
const VAD_OVERLAP_SECS: f64 = 5.0;

/// Merges stretches that overlap or nearly touch (the same speech seen by two windows).
pub fn merge_speech(mut speech: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    speech.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut merged: Vec<(f64, f64)> = Vec::new();
    for (start, end) in speech {
        match merged.last_mut() {
            Some(last) if start <= last.1 + 0.2 => last.1 = last.1.max(end),
            _ => merged.push((start, end)),
        }
    }
    merged
}

/// The stretches of speech of the recording, in seconds, as Silero sees them.
fn detect_speech(vad_path: &Path, source: &mut WavSource) -> Result<Vec<(f64, f64)>> {
    detect_speech_in(vad_path, source, VAD_WINDOW_SECS)
}

fn detect_speech_in(
    vad_path: &Path,
    source: &mut WavSource,
    window_secs: f64,
) -> Result<Vec<(f64, f64)>> {
    let path = vad_path
        .to_str()
        .context("VAD model path is not valid UTF-8")?;
    let mut params = WhisperVadContextParams::new();
    params.set_use_gpu(false);
    params.set_n_threads(thread_count());
    let mut vad = WhisperVadContext::new(path, params).context("loading the VAD model")?;

    let window = (window_secs * SAMPLE_RATE as f64) as usize;
    let step = window - (VAD_OVERLAP_SECS * SAMPLE_RATE as f64) as usize;
    let mut speech = Vec::new();
    let mut from = 0;
    loop {
        cancel::check()?;
        let to = (from + window).min(source.len());
        let samples = source.read(from, to)?;
        let offset = from as f64 / SAMPLE_RATE as f64;
        let segments = vad
            .segments_from_samples(WhisperVadParams::new(), &samples)
            .context("detecting speech")?;
        // Centiseconds.
        speech.extend(segments.map(|s| {
            (
                offset + f64::from(s.start) / 100.0,
                offset + f64::from(s.end) / 100.0,
            )
        }));
        if to >= source.len() {
            break;
        }
        from += step;
    }
    Ok(merge_speech(speech))
}

/// Every core: whisper.cpp defaults to `min(4, cores)`, which leaves most of a modern
/// CPU idle.
pub fn thread_count() -> i32 {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(1, 64) as i32
}

type Progress = Arc<Mutex<Box<dyn FnMut(f32) + Send>>>;

impl SttEngine for LocalWhisperEngine {
    fn transcribe(
        &self,
        wav_path: &Path,
        on_progress: Box<dyn FnMut(f32) + Send>,
    ) -> Result<Transcript> {
        let mut source = WavSource::open(wav_path)?;
        let total_secs = source.len() as f64 / SAMPLE_RATE as f64;

        // The VAD is run here, on the whole audio, because whisper.cpp ignores its own
        // `vad` parameter on the entry point whisper-rs uses. Each stretch of speech is
        // then transcribed alone and its times shifted back to the file's timeline.
        let blocks = match self.vad_model_path.as_deref() {
            Some(vad) => match detect_speech(vad, &mut source) {
                Ok(speech) => speech_blocks(&speech, total_secs),
                Err(_) => vec![(0.0, total_secs)],
            },
            None => vec![(0.0, total_secs)],
        };
        let work: f64 = blocks
            .iter()
            .map(|(a, b)| b - a)
            .sum::<f64>()
            .max(f64::EPSILON);

        let ctx = self.context()?;
        let mut state = ctx
            .create_state()
            .context("failed to create whisper state")?;
        let progress: Progress = Arc::new(Mutex::new(on_progress));

        // Detected on the first block, then kept: the language must not flip between
        // blocks on a bit of music.
        let mut language = self.language.clone();
        let mut segments = Vec::new();
        let mut done = 0.0;
        for &(from, to) in &blocks {
            cancel::check()?;
            if to - from < MIN_BLOCK_SECS {
                continue;
            }
            let first = (from * SAMPLE_RATE as f64) as usize;
            let last = ((to * SAMPLE_RATE as f64) as usize).min(source.len());
            // Only this block is held in memory, not the whole recording.
            let samples = source.read(first, last)?;

            let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
            params.set_language(language.as_deref()); // None = auto-detect
            params.set_n_threads(thread_count());
            params.set_print_special(false);
            params.set_print_progress(false);
            params.set_print_realtime(false);
            params.set_print_timestamps(false);
            // Drops "[music]"-style annotations that are not speech.
            params.set_suppress_nst(true);
            // Per-token timestamps: used below to give every word its own time.
            params.set_token_timestamps(true);
            let shared = Arc::clone(&progress);
            let (base, span) = (done, to - from);
            params.set_progress_callback_safe(move |percent: i32| {
                let fraction = (base + f64::from(percent) / 100.0 * span) / work;
                let mut report = shared.lock().unwrap_or_else(|p| p.into_inner());
                report(fraction.min(1.0) as f32);
            });
            params.set_abort_callback_safe(cancel::is_cancelled);

            let outcome = state.full(params, &samples);
            cancel::check()?;
            outcome.context("whisper transcription failed")?;
            done += span;

            let offset_cs = (from * 100.0).round() as i64;
            segments.extend(self.read_segments(&state, offset_cs)?);
            if language.is_none() {
                language =
                    whisper_rs::get_lang_str(state.full_lang_id_from_state()).map(str::to_string);
            }
        }
        let mut report = progress.lock().unwrap_or_else(|p| p.into_inner());
        report(1.0);

        Ok(Transcript {
            language: language.unwrap_or_else(|| "unknown".to_string()),
            segments,
        })
    }
}

impl LocalWhisperEngine {
    /// The segments of the last block, on the file's timeline (`offset_cs` is the block's
    /// start in centiseconds).
    fn read_segments(
        &self,
        state: &whisper_rs::WhisperState,
        offset_cs: i64,
    ) -> Result<Vec<Segment>> {
        let mut segments = Vec::new();
        for i in 0..state.full_n_segments() {
            let seg = state.get_segment(i).context("reading segment")?;
            let text = seg.to_str_lossy().context("reading segment text")?;
            let (start, end) = (
                (seg.start_timestamp() + offset_cs) as f64 / 100.0,
                (seg.end_timestamp() + offset_cs) as f64 / 100.0,
            );
            let tokens: Vec<(Vec<u8>, i64, i64)> = (0..seg.n_tokens())
                .filter_map(|t| seg.get_token(t))
                .filter_map(|token| {
                    let data = token.token_data();
                    token
                        .to_bytes()
                        .ok()
                        .map(|b| (b.to_vec(), data.t0 + offset_cs, data.t1 + offset_cs))
                })
                .collect();
            let words = words_from_tokens(&tokens, start, end);
            segments.extend(cues_from_words(start, end, text.trim(), words, self.limits));
        }
        Ok(segments)
    }
}

/// A mono 16 kHz WAV (as ffmpeg writes it) read by ranges: samples are normalized to
/// [-1, 1] as they are asked for, so a long recording never sits in memory as floats.
struct WavSource {
    reader: hound::WavReader<std::io::BufReader<std::fs::File>>,
    len: usize,
}

impl WavSource {
    fn open(path: &Path) -> Result<Self> {
        let reader = hound::WavReader::open(path).context("opening WAV file")?;
        let spec = reader.spec();
        anyhow::ensure!(
            spec.channels == 1 && spec.sample_rate == 16000,
            "WAV must be mono 16kHz (got: {} channels, {}Hz)",
            spec.channels,
            spec.sample_rate
        );
        let len = reader.duration() as usize;
        Ok(Self { reader, len })
    }

    /// Number of samples.
    fn len(&self) -> usize {
        self.len
    }

    /// The samples `from..to`.
    fn read(&mut self, from: usize, to: usize) -> Result<Vec<f32>> {
        let count = to.saturating_sub(from);
        self.reader
            .seek(from as u32)
            .context("seeking in the WAV file")?;
        let samples: Result<Vec<f32>, _> = match self.reader.spec().sample_format {
            hound::SampleFormat::Int => self
                .reader
                .samples::<i16>()
                .take(count)
                .map(|s| s.map(|v| f32::from(v) / f32::from(i16::MAX)))
                .collect(),
            hound::SampleFormat::Float => self.reader.samples::<f32>().take(count).collect(),
        };
        samples.context("reading WAV samples")
    }
}

/// The whole of a mono 16 kHz WAV.
#[cfg(test)]
fn read_wav_mono_f32(path: &Path) -> Result<Vec<f32>> {
    let mut source = WavSource::open(path)?;
    let len = source.len();
    source.read(0, len)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes a WAV with the requested spec and returns its path (the temporary folder is
    /// returned too so it stays alive for the duration of the test).
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
    fn every_core_is_used() {
        let expected = std::thread::available_parallelism().map_or(4, |n| n.get()) as i32;

        assert_eq!(thread_count(), expected.clamp(1, 64));
        assert!(thread_count() >= 1);
    }

    #[test]
    fn the_builder_keeps_the_vad_model_and_the_language() {
        let engine = LocalWhisperEngine::new("/m.bin")
            .with_vad(Some("/vad.bin".into()))
            .with_language(Some("fr".into()));

        assert_eq!(engine.vad_model_path, Some(PathBuf::from("/vad.bin")));
        assert_eq!(engine.language.as_deref(), Some("fr"));
    }

    #[test]
    fn the_engine_keeps_the_model_path_it_was_given() {
        let engine = LocalWhisperEngine::new("/models/ggml-small.bin");

        assert_eq!(
            engine.model_path,
            std::path::PathBuf::from("/models/ggml-small.bin")
        );
    }

    fn tok(text: &str, t0: i64, t1: i64) -> (Vec<u8>, i64, i64) {
        (text.as_bytes().to_vec(), t0, t1)
    }

    fn raw(bytes: &[u8], t0: i64, t1: i64) -> (Vec<u8>, i64, i64) {
        (bytes.to_vec(), t0, t1)
    }

    #[test]
    fn tokens_are_grouped_into_words_on_the_leading_space() {
        let tokens = [
            tok("[_BEG_]", 0, 0),
            tok(" Hel", 10, 20),
            tok("lo", 20, 40),
            tok(" world", 50, 90),
            tok("<|endoftext|>", 90, 90),
        ];

        let words = words_from_tokens(&tokens, 0.0, 1.0);

        assert_eq!(words.len(), 2);
        assert_eq!(words[0].text, "Hello");
        assert_eq!((words[0].start, words[0].end), (0.1, 0.4));
        assert_eq!(words[1].text, "world");
        assert_eq!((words[1].start, words[1].end), (0.5, 0.9));
    }

    #[test]
    fn word_times_are_kept_inside_the_segment_and_never_go_backwards() {
        let tokens = [tok(" a", -1, 30), tok(" b", 10, 500)];

        let words = words_from_tokens(&tokens, 1.0, 2.0);

        assert!(words[0].start >= 1.0);
        assert!(words[1].start >= words[0].end);
        assert!(words[1].end <= 2.0);
        assert!(words.iter().all(|w| w.end >= w.start));
    }

    #[test]
    fn a_segment_with_only_special_tokens_has_no_word() {
        assert!(words_from_tokens(&[tok("[_TT_100]", 0, 0)], 0.0, 1.0).is_empty());
    }

    #[test]
    fn the_builder_keeps_custom_limits() {
        let engine = LocalWhisperEngine::new("/m.bin").with_limits(CueLimits {
            max_secs: 3.0,
            max_chars: 40,
        });

        assert_eq!(engine.limits.max_secs, 3.0);
    }

    #[test]
    fn a_character_cut_into_partial_tokens_is_not_lost() {
        // "논" is three bytes that whisper may emit as two tokens, neither valid UTF-8.
        let bytes = "논".as_bytes();
        let first = [&b" "[..], &bytes[..1]].concat();
        let tokens = [
            tok(" 안", 0, 20),
            raw(&first, 20, 30),
            raw(&bytes[1..], 30, 50),
            tok(" 문", 60, 90),
        ];

        let words = words_from_tokens(&tokens, 0.0, 1.0);

        assert_eq!(words.len(), 3);
        assert_eq!(words[0].text, "안");
        assert_eq!(words[1].text, "논");
        assert_eq!(words[2].text, "문");
    }

    #[test]
    fn a_hangul_word_made_of_several_tokens_stays_whole() {
        let bytes = "젊".as_bytes();
        let tokens = [
            tok(" 안", 0, 20),
            raw(&bytes[..2], 20, 30),
            raw(&bytes[2..], 30, 50),
        ];

        let words = words_from_tokens(&tokens, 0.0, 1.0);

        // Hangul is written with spaces: the pieces stay together, whole characters.
        assert_eq!(words.len(), 1);
        assert_eq!(words[0].text, "안젊");
        assert_eq!(words[0].end, 0.5);
    }

    #[test]
    fn each_chinese_token_is_its_own_word() {
        let tokens = [tok("我们", 0, 30), tok("今天", 30, 60), tok("开会", 60, 90)];

        let words = words_from_tokens(&tokens, 0.0, 1.0);

        let texts: Vec<_> = words.iter().map(|w| w.text.as_str()).collect();
        assert_eq!(texts, ["我们", "今天", "开会"]);
    }

    #[test]
    fn a_latin_suffix_stays_in_its_word() {
        let tokens = [tok(" Ameri", 0, 20), tok("cans", 20, 50)];

        let words = words_from_tokens(&tokens, 0.0, 1.0);

        assert_eq!(words.len(), 1);
        assert_eq!(words[0].text, "Americans");
    }

    #[test]
    fn nearby_speech_goes_in_one_block_with_a_margin() {
        let blocks = speech_blocks(&[(1.0, 3.0), (3.5, 6.0)], 20.0);

        assert_eq!(blocks, vec![(0.75, 6.25)]);
    }

    #[test]
    fn speech_after_a_long_silence_is_a_separate_block() {
        let blocks = speech_blocks(&[(0.0, 11.0), (31.0, 42.0)], 73.0);

        assert_eq!(blocks.len(), 2);
        assert!(blocks[0].1 <= 11.25 + 1e-9);
        assert!(blocks[1].0 >= 30.75 - 1e-9);
    }

    #[test]
    fn a_block_never_grows_past_the_limit() {
        let speech: Vec<(f64, f64)> = (0..100)
            .map(|i| (i as f64 * 10.0, i as f64 * 10.0 + 9.5))
            .collect();

        let blocks = speech_blocks(&speech, 1000.0);

        assert!(blocks.len() > 1);
        assert!(blocks
            .iter()
            .all(|(a, b)| b - a <= MAX_BLOCK_SECS + 2.0 * BLOCK_PAD_SECS));
    }

    #[test]
    fn margins_stay_inside_the_recording() {
        let blocks = speech_blocks(&[(0.0, 5.0)], 5.0);

        assert_eq!(blocks, vec![(0.0, 5.0)]);
    }

    #[test]
    fn no_speech_means_no_block() {
        assert!(speech_blocks(&[], 30.0).is_empty());
    }

    #[test]
    fn the_cached_model_can_be_released() {
        release_cache();

        assert!(cache().lock().unwrap().is_none());
    }

    #[test]
    fn a_range_of_a_wav_is_read_without_the_rest() {
        let samples: Vec<i32> = (0..100).collect();
        let (_dir, path) = write_wav(1, 16_000, hound::SampleFormat::Int, 16, &samples);
        let mut source = WavSource::open(&path).unwrap();

        let part = source.read(10, 13).unwrap();

        assert_eq!(source.len(), 100);
        assert_eq!(part.len(), 3);
        assert!((part[0] - 10.0 / f32::from(i16::MAX)).abs() < 1e-7);
        assert!((part[2] - 12.0 / f32::from(i16::MAX)).abs() < 1e-7);
        // Reading backwards and past the end is fine too.
        assert_eq!(source.read(0, 2).unwrap().len(), 2);
        assert_eq!(source.read(98, 200).unwrap().len(), 2);
    }

    #[test]
    fn speech_seen_by_two_windows_is_merged() {
        let merged = merge_speech(vec![(595.0, 604.0), (3.0, 8.0), (600.0, 612.0), (8.1, 9.0)]);

        assert_eq!(merged, vec![(3.0, 9.0), (595.0, 612.0)]);
    }

    /// Real VAD on a recording of speech and silences: cutting it in short windows must
    /// find the same speech as one pass.
    /// `LGS_TEST_GAPS=/path/gaps.wav LGS_TEST_VAD=/path/ggml-silero-v5.1.2.bin
    /// cargo test real_vad_windows -- --ignored`
    #[test]
    #[ignore]
    fn real_vad_windows_agree_with_one_pass() {
        let gaps = std::env::var("LGS_TEST_GAPS").expect("LGS_TEST_GAPS");
        let vad = std::env::var("LGS_TEST_VAD").expect("LGS_TEST_VAD");
        let mut source = WavSource::open(Path::new(&gaps)).unwrap();

        let whole = detect_speech_in(Path::new(&vad), &mut source, 10_000.0).unwrap();
        let windows = detect_speech_in(Path::new(&vad), &mut source, 25.0).unwrap();

        println!("{whole:?}\n{windows:?}");
        let blocks = |speech: &[(f64, f64)]| speech_blocks(speech, 73.0);
        let (a, b) = (blocks(&whole), blocks(&windows));
        assert_eq!(a.len(), 3, "{a:?}");
        assert_eq!(a.len(), b.len(), "{a:?} vs {b:?}");
        for (x, y) in a.iter().zip(&b) {
            assert!(
                (x.0 - y.0).abs() < 0.6 && (x.1 - y.1).abs() < 0.6,
                "{x:?} vs {y:?}"
            );
        }
    }
}
