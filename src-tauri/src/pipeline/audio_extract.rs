use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};

/// Emplacement du WAV extrait : `<cache>/<nom source>.wav`. Séparé de `extract_to_wav`
/// pour être vérifiable sans lancer ffmpeg.
pub fn wav_output_path(input_path: &Path, cache_dir: &Path) -> PathBuf {
    let stem = input_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("audio");
    cache_dir.join(format!("{stem}.wav"))
}

/// Réglages de conversion imposés par whisper.cpp : mono, 16 kHz, PCM 16 bits signés.
pub const FFMPEG_AUDIO_ARGS: [&str; 6] = ["-ar", "16000", "-ac", "1", "-c:a", "pcm_s16le"];

/// Converts a video or audio file to mono 16kHz WAV via ffmpeg (expected on PATH),
/// writes it to the app's cache directory, and returns the resulting WAV path.
pub fn extract_to_wav(input_path: &Path, cache_dir: &Path) -> Result<PathBuf> {
    std::fs::create_dir_all(cache_dir).context("creating cache directory")?;

    let out_path = wav_output_path(input_path, cache_dir);

    let status = Command::new("ffmpeg")
        .arg("-y") // overwrite the output file if it already exists
        .arg("-i")
        .arg(input_path)
        .args(FFMPEG_AUDIO_ARGS)
        .arg(&out_path)
        .status()
        .context("failed to launch ffmpeg — is it installed and on the PATH?")?;

    if !status.success() {
        bail!("ffmpeg failed with code {:?}", status.code());
    }

    Ok(out_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wav_takes_the_source_name_inside_the_cache() {
        assert_eq!(
            wav_output_path(Path::new("/home/me/clip.mp4"), Path::new("/cache")),
            PathBuf::from("/cache/clip.wav")
        );
    }

    #[test]
    fn a_dotted_source_name_only_loses_its_last_extension() {
        assert_eq!(
            wav_output_path(Path::new("/home/me/s01.e02.mkv"), Path::new("/cache")),
            PathBuf::from("/cache/s01.e02.wav")
        );
    }

    #[test]
    fn a_source_without_a_usable_stem_falls_back_to_audio() {
        assert_eq!(
            wav_output_path(Path::new("/"), Path::new("/cache")),
            PathBuf::from("/cache/audio.wav")
        );
    }

    #[test]
    fn two_sources_do_not_collide_in_the_cache() {
        let a = wav_output_path(Path::new("/a/clip.mp4"), Path::new("/cache"));
        let b = wav_output_path(Path::new("/a/other.mp4"), Path::new("/cache"));

        assert_ne!(a, b);
    }

    #[test]
    fn the_conversion_matches_what_whisper_expects() {
        // whisper.cpp n'accepte que du PCM 16 bits mono à 16 kHz.
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
    fn a_missing_input_makes_ffmpeg_fail() {
        let cache = tempfile::tempdir().unwrap();

        // Si ffmpeg n'est pas installé, l'erreur porte sur le lancement ; dans les deux
        // cas la fonction doit rendre une erreur et non paniquer.
        let result = extract_to_wav(Path::new("/nowhere/clip.mp4"), cache.path());

        assert!(result.is_err());
    }
}
