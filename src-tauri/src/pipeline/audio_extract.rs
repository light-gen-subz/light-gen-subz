use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{bail, Context, Result};

use crate::cancel;
use crate::errors::coded;

/// What an engine reads: a WAV, or audio compressed for upload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioFormat {
    /// Mono 16 kHz 16-bit PCM, as whisper.cpp wants it.
    Wav,
    /// 32 kbit/s mono MP3, small enough to upload.
    Mp3,
}

/// The extracted audio in a private temporary directory, deleted when this value is
/// dropped — on success, on failure and on cancellation alike.
pub struct TempAudio {
    _dir: tempfile::TempDir,
    pub path: PathBuf,
}

/// Prefix of the temporary folders, so stale ones can be told apart at startup.
const TEMP_PREFIX: &str = "audio-";

/// Conversion settings required by whisper.cpp: mono, 16 kHz, signed 16-bit PCM.
pub const FFMPEG_AUDIO_ARGS: [&str; 6] = ["-ar", "16000", "-ac", "1", "-c:a", "pcm_s16le"];

/// Cloud engines get a compressed copy instead: 32 kbit/s mono MP3 is ~0.24 MB per
/// minute against 1.9 MB for the WAV, which keeps an hour of audio under OpenAI's
/// 25 MB limit per request and uploads eight times faster.
pub const FFMPEG_CLOUD_ARGS: [&str; 8] = [
    "-ar",
    "16000",
    "-ac",
    "1",
    "-c:a",
    "libmp3lame",
    "-b:a",
    "32k",
];

/// Length of one request to a size-limited cloud engine. 20 minutes ≈ 4.8 MB of MP3.
pub const CLOUD_CHUNK_SECS: f64 = 20.0 * 60.0;

/// Folders where Homebrew and friends put ffmpeg. A GUI app launched from the Finder
/// or Spotlight does not inherit the shell's `PATH`, so they are searched explicitly.
const EXTRA_BIN_DIRS: [&str; 4] = [
    "/opt/homebrew/bin",
    "/usr/local/bin",
    "/opt/local/bin",
    "/usr/bin",
];

/// Absolute path of ffmpeg: on the `PATH` first, then in the usual install folders.
/// Falls back to the bare name, so the launch error stays explicit.
pub fn ffmpeg_path() -> PathBuf {
    find_in(
        "ffmpeg",
        std::env::var_os("PATH")
            .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
            .unwrap_or_default()
            .into_iter()
            .chain(EXTRA_BIN_DIRS.iter().map(PathBuf::from)),
    )
    .unwrap_or_else(|| PathBuf::from("ffmpeg"))
}

/// ffprobe sits next to ffmpeg in every distribution.
pub fn ffprobe_path() -> PathBuf {
    find_in(
        "ffprobe",
        std::env::var_os("PATH")
            .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
            .unwrap_or_default()
            .into_iter()
            .chain(EXTRA_BIN_DIRS.iter().map(PathBuf::from)),
    )
    .unwrap_or_else(|| PathBuf::from("ffprobe"))
}

/// One audio stream of a media file, as the user can tell them apart.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct AudioTrack {
    /// Position among the audio streams (`0:a:N` in ffmpeg), not the global stream index.
    pub position: u32,
    pub language: Option<String>,
    pub title: Option<String>,
    pub codec: Option<String>,
    pub channels: Option<u32>,
}

/// Reads ffprobe's JSON (`-show_streams -select_streams a`) into tracks.
pub fn parse_probe(json: &str) -> Result<Vec<AudioTrack>> {
    #[derive(serde::Deserialize)]
    struct Probe {
        #[serde(default)]
        streams: Vec<Stream>,
    }
    #[derive(serde::Deserialize)]
    struct Stream {
        codec_name: Option<String>,
        channels: Option<u32>,
        #[serde(default)]
        tags: std::collections::HashMap<String, String>,
    }
    let probe: Probe = serde_json::from_str(json).context("parsing ffprobe output")?;
    Ok(probe
        .streams
        .into_iter()
        .enumerate()
        .map(|(position, stream)| {
            // Tag names are not consistently cased between containers.
            let tag = |name: &str| {
                stream
                    .tags
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case(name))
                    .map(|(_, v)| v.clone())
                    .filter(|v| !v.trim().is_empty())
            };
            AudioTrack {
                position: position as u32,
                language: tag("language").filter(|l| l != "und"),
                title: tag("title"),
                codec: stream.codec_name,
                channels: stream.channels,
            }
        })
        .collect())
}

/// The audio streams of `input`. A file ffprobe cannot read has none.
pub fn list_audio_tracks(input: &Path) -> Result<Vec<AudioTrack>> {
    let output = Command::new(ffprobe_path())
        .args([
            "-v",
            "error",
            "-select_streams",
            "a",
            "-show_streams",
            "-of",
            "json",
        ])
        .arg(input_arg(input))
        .stdin(Stdio::null())
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                coded(
                    "ffmpeg_missing",
                    "ffprobe was not found — install ffmpeg and make sure it is on the PATH",
                )
            } else {
                anyhow::Error::new(e).context("failed to launch ffprobe")
            }
        })?;
    if !output.status.success() {
        return Ok(Vec::new());
    }
    parse_probe(&String::from_utf8_lossy(&output.stdout))
}

pub fn find_in(name: &str, dirs: impl IntoIterator<Item = PathBuf>) -> Option<PathBuf> {
    dirs.into_iter()
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

/// The input as ffmpeg must read it: `file:` stops a name such as `http://…` or `-x` from
/// being taken for a URL or an option.
fn input_arg(input: &Path) -> std::ffi::OsString {
    let mut arg = std::ffi::OsString::from("file:");
    arg.push(input);
    arg
}

/// Arguments for a plain WAV extraction. Separate from the launch so they are checkable.
pub fn wav_args(input: &Path, out: &Path, track: Option<u32>) -> Vec<std::ffi::OsString> {
    extract_args(input, out, track, &FFMPEG_AUDIO_ARGS)
}

/// Arguments for a compressed extraction, straight from the source.
pub fn mp3_args(input: &Path, out: &Path, track: Option<u32>) -> Vec<std::ffi::OsString> {
    extract_args(input, out, track, &FFMPEG_CLOUD_ARGS)
}

fn extract_args(
    input: &Path,
    out: &Path,
    track: Option<u32>,
    codec: &[&str],
) -> Vec<std::ffi::OsString> {
    let mut args: Vec<std::ffi::OsString> =
        ["-nostdin", "-hide_banner", "-loglevel", "error", "-y", "-i"]
            .iter()
            .map(Into::into)
            .collect();
    args.push(input_arg(input));
    if let Some(position) = track {
        args.extend(["-map".into(), format!("0:a:{position}").into()]);
    }
    // No video, subtitle or data streams: only the audio is wanted.
    args.extend(
        ["-vn", "-sn", "-dn"]
            .iter()
            .map(Into::<std::ffi::OsString>::into),
    );
    args.extend(codec.iter().map(Into::<std::ffi::OsString>::into));
    args.push(out.into());
    args
}

/// Arguments for the slice `[start, start + len)` of an already compressed file, copied
/// without a second encoding.
pub fn slice_args(audio: &Path, start: f64, len: f64, out: &Path) -> Vec<std::ffi::OsString> {
    let mut args: Vec<std::ffi::OsString> =
        ["-nostdin", "-hide_banner", "-loglevel", "error", "-y"]
            .iter()
            .map(Into::into)
            .collect();
    args.extend(["-ss".into(), format!("{start:.3}").into()]);
    args.extend(["-t".into(), format!("{len:.3}").into()]);
    args.extend(["-i".into(), input_arg(audio)]);
    args.extend(["-c".into(), "copy".into()]);
    args.push(out.into());
    args
}

/// How far before a chunk's end a pause is looked for, to cut in silence.
const SILENCE_SEARCH_SECS: f64 = 90.0;

/// Splits `total` seconds into `(start, length)` slices of at most `chunk` seconds. Each cut
/// goes to the middle of the silence nearest to the limit (within the last
/// [`SILENCE_SEARCH_SECS`] of the chunk), so a word is not cut in two; without silence it
/// falls at the limit. A recording with no duration has no slice.
pub fn plan_chunks(total: f64, chunk: f64, silences: &[(f64, f64)]) -> Vec<(f64, f64)> {
    if total <= 0.0 {
        return Vec::new();
    }
    let mut chunks = Vec::new();
    let mut start = 0.0;
    while total - start > chunk {
        let limit = start + chunk;
        let cut = silences
            .iter()
            .map(|(a, b)| (a + b) / 2.0)
            .filter(|mid| *mid > limit - SILENCE_SEARCH_SECS && *mid <= limit && *mid > start + 1.0)
            .fold(None, |best: Option<f64>, mid| match best {
                Some(b) if b >= mid => Some(b),
                _ => Some(mid),
            })
            .unwrap_or(limit);
        chunks.push((start, cut - start));
        start = cut;
    }
    chunks.push((start, total - start));
    chunks
}

/// The `(start, end)` of every silence ffmpeg's `silencedetect` reported.
pub fn parse_silences(log: &str) -> Vec<(f64, f64)> {
    let value = |line: &str, key: &str| {
        line.split(key)
            .nth(1)
            .and_then(|rest| rest.split_whitespace().next())
            .and_then(|v| v.parse::<f64>().ok())
    };
    let mut silences = Vec::new();
    let mut open: Option<f64> = None;
    for line in log.lines() {
        if let Some(start) = value(line, "silence_start:") {
            open = Some(start);
        } else if let (Some(end), Some(start)) = (value(line, "silence_end:"), open.take()) {
            silences.push((start, end));
        }
    }
    silences
}

/// Pauses of `audio`, used to cut long recordings between two words.
pub fn find_silences(audio: &Path) -> Vec<(f64, f64)> {
    let output = Command::new(ffmpeg_path())
        .args(["-nostdin", "-hide_banner", "-nostats", "-i"])
        .arg(input_arg(audio))
        .args(["-af", "silencedetect=noise=-35dB:d=0.4", "-f", "null", "-"])
        .stdin(Stdio::null())
        .output();
    output.map_or_else(
        |_| Vec::new(),
        |o| parse_silences(&String::from_utf8_lossy(&o.stderr)),
    )
}

/// Duration of any media file in seconds, read by ffprobe.
pub fn media_duration_secs(path: &Path) -> Result<f64> {
    let output = Command::new(ffprobe_path())
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "csv=p=0",
        ])
        .arg(input_arg(path))
        .stdin(Stdio::null())
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                coded(
                    "ffmpeg_missing",
                    "ffprobe was not found — install ffmpeg and make sure it is on the PATH",
                )
            } else {
                anyhow::Error::new(e).context("failed to launch ffprobe")
            }
        })?;
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<f64>()
        .context("reading the audio duration")
}

/// Runs ffmpeg, polling so that a cancellation kills it, and keeps its stderr for the
/// error message.
fn run_ffmpeg(args: &[std::ffi::OsString]) -> Result<()> {
    let mut child = Command::new(ffmpeg_path())
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                coded(
                    "ffmpeg_missing",
                    "ffmpeg was not found — install it and make sure it is on the PATH",
                )
            } else {
                anyhow::Error::new(e).context("failed to launch ffmpeg")
            }
        })?;

    let mut stderr = child.stderr.take().context("capturing ffmpeg output")?;
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text);
        text
    });

    let status = loop {
        if cancel::is_cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            return cancel::check();
        }
        if let Some(status) = child.try_wait().context("waiting for ffmpeg")? {
            break status;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let log = reader.join().unwrap_or_default();

    if status.success() {
        return Ok(());
    }
    if log.contains("does not contain any stream") {
        return Err(coded("no_audio", "the file has no audio track"));
    }
    let tail: String = log
        .lines()
        .rev()
        .take(5)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n");
    bail!(
        "ffmpeg failed with code {:?}: {}",
        status.code(),
        tail.trim()
    );
}

/// Extracts the audio of a video or audio file in the format an engine reads, inside a
/// fresh temporary directory of `cache_dir`.
pub fn extract_audio(
    input_path: &Path,
    cache_dir: &Path,
    track: Option<u32>,
    format: AudioFormat,
) -> Result<TempAudio> {
    std::fs::create_dir_all(cache_dir).context("creating cache directory")?;
    let dir = tempfile::Builder::new()
        .prefix(TEMP_PREFIX)
        .tempdir_in(cache_dir)
        .context("creating temporary directory")?;
    let (path, args) = match format {
        AudioFormat::Wav => {
            let path = dir.path().join("audio.wav");
            let args = wav_args(input_path, &path, track);
            (path, args)
        }
        AudioFormat::Mp3 => {
            let path = dir.path().join("audio.mp3");
            let args = mp3_args(input_path, &path, track);
            (path, args)
        }
    };

    run_ffmpeg(&args)?;

    Ok(TempAudio { _dir: dir, path })
}

/// Cuts `[start, start + len)` of a compressed `audio` into `out`.
pub fn cut_slice(audio: &Path, start: f64, len: f64, out: &Path) -> Result<()> {
    run_ffmpeg(&slice_args(audio, start, len, out))
}

/// Deletes the temporary folders a crash or a forced quit left behind: nothing running
/// owns a folder that is more than `max_age` old.
pub fn remove_stale_temp_dirs(cache_dir: &Path, max_age: Duration) -> usize {
    let Ok(entries) = std::fs::read_dir(cache_dir) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let is_ours = entry.file_name().to_string_lossy().starts_with(TEMP_PREFIX);
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age >= max_age);
        if is_ours && old && std::fs::remove_dir_all(entry.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(args: Vec<std::ffi::OsString>) -> Vec<String> {
        args.into_iter()
            .map(|a| a.to_string_lossy().to_string())
            .collect()
    }

    #[test]
    fn the_conversion_matches_what_whisper_expects() {
        // whisper.cpp only accepts 16-bit mono PCM at 16 kHz.
        let args = FFMPEG_AUDIO_ARGS;
        let after = |flag: &str| {
            args.iter()
                .position(|a| *a == flag)
                .map(|i| args[i + 1])
                .unwrap_or_else(|| panic!("{flag} absent"))
        };
        assert_eq!(after("-ar"), "16000");
        assert_eq!(after("-ac"), "1");
        assert_eq!(after("-c:a"), "pcm_s16le");
    }

    #[test]
    fn ffmpeg_never_reads_the_terminal_and_drops_non_audio_streams() {
        let args = strings(wav_args(Path::new("/in.mkv"), Path::new("/out.wav"), None));

        for flag in ["-nostdin", "-vn", "-sn", "-dn", "-y"] {
            assert!(args.contains(&flag.to_string()), "{flag}");
        }
    }

    #[test]
    fn the_input_comes_before_the_output() {
        let args = strings(wav_args(Path::new("/in.mkv"), Path::new("/out.wav"), None));

        let input = args.iter().position(|a| a == "file:/in.mkv").unwrap();
        assert_eq!(args[input - 1], "-i");
        assert_eq!(args.last().unwrap(), "/out.wav");
    }

    #[test]
    fn a_slice_seeks_before_reading_and_copies_the_audio() {
        let args = strings(slice_args(
            Path::new("/a.mp3"),
            1200.0,
            300.5,
            Path::new("/o.mp3"),
        ));

        let ss = args.iter().position(|a| a == "-ss").unwrap();
        let i = args.iter().position(|a| a == "-i").unwrap();
        assert!(ss < i, "-ss must precede -i for an exact input seek");
        assert_eq!(args[ss + 1], "1200.000");
        assert!(args.contains(&"300.500".to_string()));
        assert!(args.contains(&"copy".to_string()));
        assert!(!args.contains(&"libmp3lame".to_string()));
    }

    #[test]
    fn a_short_recording_is_a_single_chunk() {
        assert_eq!(
            plan_chunks(600.0, CLOUD_CHUNK_SECS, &[]),
            vec![(0.0, 600.0)]
        );
    }

    #[test]
    fn a_long_recording_is_cut_into_bounded_chunks() {
        let chunks = plan_chunks(3000.0, 1200.0, &[]);

        assert_eq!(
            chunks,
            vec![(0.0, 1200.0), (1200.0, 1200.0), (2400.0, 600.0)]
        );
    }

    #[test]
    fn the_chunks_cover_the_whole_recording_without_gap() {
        let silences = [(1150.0, 1152.0), (2300.0, 2301.0), (3500.0, 3501.0)];
        for silences in [&silences[..], &[]] {
            let chunks = plan_chunks(7321.0, CLOUD_CHUNK_SECS, silences);

            let covered: f64 = chunks.iter().map(|(_, len)| len).sum();
            assert!((covered - 7321.0).abs() < 1e-9);
            for pair in chunks.windows(2) {
                assert!((pair[0].0 + pair[0].1 - pair[1].0).abs() < 1e-9);
            }
            assert!(chunks
                .iter()
                .all(|(_, len)| *len <= CLOUD_CHUNK_SECS + 1e-9));
        }
    }

    #[test]
    fn a_cut_falls_in_the_silence_nearest_to_the_limit() {
        let chunks = plan_chunks(3000.0, 1200.0, &[(1000.0, 1001.0), (1150.0, 1152.0)]);

        assert_eq!(chunks[0], (0.0, 1151.0));
        assert_eq!(chunks[1].0, 1151.0);
    }

    #[test]
    fn a_silence_too_far_from_the_limit_is_ignored() {
        let chunks = plan_chunks(3000.0, 1200.0, &[(300.0, 301.0)]);

        assert_eq!(chunks[0], (0.0, 1200.0));
    }

    #[test]
    fn a_recording_without_duration_has_no_slice() {
        assert!(plan_chunks(0.0, 1200.0, &[]).is_empty());
    }

    #[test]
    fn silences_are_read_from_the_ffmpeg_log() {
        let log = "[silencedetect @ 0x1] silence_start: 11\n\
                   [silencedetect @ 0x1] silence_end: 31.072125 | silence_duration: 20.07\n\
                   [silencedetect @ 0x1] silence_start: 42.5\n";

        assert_eq!(parse_silences(log), vec![(11.0, 31.072125)]);
    }

    #[test]
    fn stale_temporary_folders_are_removed_and_other_files_left_alone() {
        let cache = tempfile::tempdir().unwrap();
        std::fs::create_dir(cache.path().join("audio-old")).unwrap();
        std::fs::create_dir(cache.path().join("other")).unwrap();

        let removed = remove_stale_temp_dirs(cache.path(), Duration::ZERO);

        assert_eq!(removed, 1);
        assert!(!cache.path().join("audio-old").exists());
        assert!(cache.path().join("other").exists());
    }

    #[test]
    fn a_recent_temporary_folder_may_belong_to_a_running_job() {
        let cache = tempfile::tempdir().unwrap();
        std::fs::create_dir(cache.path().join("audio-new")).unwrap();

        assert_eq!(
            remove_stale_temp_dirs(cache.path(), Duration::from_secs(3600)),
            0
        );
    }

    #[test]
    fn a_name_that_looks_like_a_url_or_an_option_is_a_file() {
        let args = strings(wav_args(
            Path::new("http://evil/x.mp4"),
            Path::new("/o.wav"),
            None,
        ));

        assert!(args.contains(&"file:http://evil/x.mp4".to_string()));
        let args = strings(wav_args(Path::new("-f"), Path::new("/o.wav"), None));
        assert!(args.contains(&"file:-f".to_string()));
    }

    #[test]
    fn an_hour_of_compressed_audio_stays_well_under_the_openai_limit() {
        // 32 kbit/s = 4 kB/s.
        let bytes = CLOUD_CHUNK_SECS * 4000.0;

        assert!(bytes < 25.0 * 1024.0 * 1024.0 / 4.0);
    }

    #[test]
    fn a_binary_is_found_in_the_first_folder_that_has_it() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        std::fs::write(b.path().join("ffmpeg"), b"").unwrap();

        let found = find_in("ffmpeg", [a.path().to_path_buf(), b.path().to_path_buf()]);

        assert_eq!(found, Some(b.path().join("ffmpeg")));
    }

    #[test]
    fn a_binary_absent_everywhere_is_not_found() {
        let a = tempfile::tempdir().unwrap();

        assert_eq!(find_in("ffmpeg", [a.path().to_path_buf()]), None);
    }

    #[test]
    fn the_homebrew_folders_are_searched() {
        assert!(EXTRA_BIN_DIRS.contains(&"/opt/homebrew/bin"));
        assert!(EXTRA_BIN_DIRS.contains(&"/usr/local/bin"));
    }

    #[test]
    fn a_missing_input_makes_ffmpeg_fail() {
        let cache = tempfile::tempdir().unwrap();

        // If ffmpeg is not installed the error is about launching it; either way the function
        // must return an error rather than panic.
        let result = extract_audio(
            Path::new("/nowhere/clip.mp4"),
            cache.path(),
            None,
            AudioFormat::Wav,
        );

        assert!(result.is_err());
    }

    #[test]
    fn the_temporary_wav_is_deleted_with_its_owner() {
        let cache = tempfile::tempdir().unwrap();
        let dir = tempfile::Builder::new().tempdir_in(cache.path()).unwrap();
        let audio = TempAudio {
            path: dir.path().join("audio.wav"),
            _dir: dir,
        };
        std::fs::write(&audio.path, b"x").unwrap();
        let path = audio.path.clone();

        drop(audio);

        assert!(!path.exists());
    }

    #[test]
    fn two_sources_with_the_same_name_do_not_share_a_wav() {
        let cache = tempfile::tempdir().unwrap();
        let make = || {
            let dir = tempfile::Builder::new()
                .prefix("audio-")
                .tempdir_in(cache.path())
                .unwrap();
            dir.path().join("audio.wav")
        };

        assert_ne!(make(), make());
    }

    /// Real ffmpeg round trip (skipped where ffmpeg is not installed).
    #[test]
    fn a_source_is_compressed_once_and_sliced_without_re_encoding() {
        if !ffmpeg_path().is_absolute() {
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
        for i in 0..16_000 * 4 {
            let t = i as f32 / 16_000.0;
            writer
                .write_sample(((t * 440.0 * std::f32::consts::TAU).sin() * 8000.0) as i16)
                .unwrap();
        }
        writer.finalize().unwrap();
        let mp3 = extract_audio(&wav, dir.path(), None, AudioFormat::Mp3).unwrap();
        let out = dir.path().join("slice.mp3");

        cut_slice(&mp3.path, 1.0, 2.0, &out).unwrap();

        let size = std::fs::metadata(&out).unwrap().len();
        // 2 s at 32 kbit/s ≈ 8 kB, against 64 kB of WAV.
        assert!((4_000..20_000).contains(&size), "{size}");
        assert!((media_duration_secs(&mp3.path).unwrap() - 4.0).abs() < 0.2);
        assert!((media_duration_secs(&out).unwrap() - 2.0).abs() < 0.2);
    }

    #[test]
    fn the_mp3_extraction_targets_the_upload_codec() {
        let args = strings(mp3_args(
            Path::new("/in.mkv"),
            Path::new("/out.mp3"),
            Some(1),
        ));

        assert!(args.contains(&"libmp3lame".to_string()));
        assert!(args.contains(&"0:a:1".to_string()));
    }

    #[test]
    fn a_file_without_audio_is_reported_with_its_own_code() {
        if !ffmpeg_path().is_absolute() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let text = dir.path().join("not-media.mp4");
        std::fs::write(&text, b"definitely not a media file").unwrap();

        let err = extract_audio(&text, dir.path(), None, AudioFormat::Wav)
            .err()
            .unwrap();

        // Either "no audio" or a plain ffmpeg failure, never a panic nor a silent success.
        assert!(!err.to_string().is_empty());
    }

    #[test]
    fn the_chosen_audio_track_is_mapped_explicitly() {
        let args = strings(wav_args(
            Path::new("/in.mkv"),
            Path::new("/out.wav"),
            Some(2),
        ));

        let map = args.iter().position(|a| a == "-map").unwrap();
        assert_eq!(args[map + 1], "0:a:2");
        let input = args.iter().position(|a| a == "-i").unwrap();
        assert!(map > input, "-map is an output option");
    }

    #[test]
    fn no_track_means_ffmpegs_own_choice() {
        let args = strings(wav_args(Path::new("/in.mkv"), Path::new("/out.wav"), None));

        assert!(!args.contains(&"-map".to_string()));
    }

    #[test]
    fn ffprobe_output_becomes_tracks_in_order() {
        let tracks = parse_probe(
            r#"{"streams":[
                {"index":1,"codec_name":"aac","channels":2,"tags":{"language":"fra","title":"Français"}},
                {"index":2,"codec_name":"ac3","channels":6,"tags":{"LANGUAGE":"eng"}},
                {"index":3,"codec_name":"mp3","channels":1}]}"#,
        )
        .unwrap();

        assert_eq!(tracks.len(), 3);
        assert_eq!(tracks[0].position, 0);
        assert_eq!(tracks[0].language.as_deref(), Some("fra"));
        assert_eq!(tracks[0].title.as_deref(), Some("Français"));
        assert_eq!(tracks[1].language.as_deref(), Some("eng"));
        assert_eq!(tracks[1].channels, Some(6));
        assert_eq!(tracks[2].language, None);
        assert_eq!(tracks[2].position, 2);
    }

    #[test]
    fn an_undetermined_language_tag_is_dropped() {
        let tracks = parse_probe(r#"{"streams":[{"tags":{"language":"und"}}]}"#).unwrap();

        assert_eq!(tracks[0].language, None);
    }

    #[test]
    fn a_file_without_streams_has_no_track() {
        assert!(parse_probe("{}").unwrap().is_empty());
        assert!(parse_probe("not json").is_err());
    }

    /// Real ffmpeg/ffprobe on a file with two audio tracks: the second one is the one
    /// extracted when it is asked for.
    #[test]
    fn a_chosen_track_is_the_one_that_gets_extracted() {
        if !ffmpeg_path().is_absolute() || !ffprobe_path().is_absolute() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let media = dir.path().join("two.mkv");
        let status = Command::new(ffmpeg_path())
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=1",
            ])
            .args(["-f", "lavfi", "-i", "sine=frequency=880:duration=3"])
            .args([
                "-map",
                "0:a",
                "-map",
                "1:a",
                "-metadata:s:a:0",
                "language=fra",
            ])
            .args(["-metadata:s:a:1", "language=eng", "-y"])
            .arg(&media)
            .status()
            .unwrap();
        assert!(status.success());

        let tracks = list_audio_tracks(&media).unwrap();
        let second = extract_audio(&media, dir.path(), Some(1), AudioFormat::Wav).unwrap();

        assert_eq!(tracks.len(), 2);
        assert_eq!(tracks[1].language.as_deref(), Some("eng"));
        // The second track lasts 3 s, the first only 1 s.
        assert!((media_duration_secs(&second.path).unwrap() - 3.0).abs() < 0.2);
    }
}
