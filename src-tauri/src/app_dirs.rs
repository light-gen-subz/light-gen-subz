use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Folder name used for the app's config and data directories.
const APP_DIR: &str = "light-gen-subz";

/// Returns `<base>/light-gen-subz`, adopting a folder whose name differs only by
/// letter case so downloaded models and settings survive a rename of the app.
///
/// On case-insensitive filesystems both spellings denote the same directory, so
/// nothing is moved and the existing folder is used as-is.
pub fn app_dir_in(base: &Path) -> PathBuf {
    let current = base.join(APP_DIR);
    if !current.exists() {
        if let Some(previous) = case_variant_of(base, APP_DIR) {
            let _ = fs::rename(&previous, &current);
        }
    }
    current
}

/// Finds an entry of `base` whose name matches `name` apart from letter case.
fn case_variant_of(base: &Path, name: &str) -> Option<PathBuf> {
    fs::read_dir(base).ok()?.flatten().find_map(|entry| {
        let path = entry.path();
        let file_name = path.file_name()?.to_str()?;
        (file_name != name && file_name.eq_ignore_ascii_case(name)).then_some(path)
    })
}

/// The user's config directory for this app, created if missing.
pub fn config_dir() -> Result<PathBuf> {
    let base = dirs::config_dir().context("could not determine the user's config directory")?;
    let dir = app_dir_in(&base);
    fs::create_dir_all(&dir).context("creating config directory")?;
    Ok(dir)
}

/// The user's data directory for this app, created if missing.
pub fn data_dir() -> Result<PathBuf> {
    let base = dirs::data_dir().context("could not determine the user's data directory")?;
    let dir = app_dir_in(&base);
    fs::create_dir_all(&dir).context("creating data directory")?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A spelling of `APP_DIR` that differs from it only by letter case.
    fn other_casing() -> String {
        APP_DIR.to_uppercase()
    }

    #[test]
    fn adopts_a_directory_that_differs_only_by_case() {
        let base = tempfile::tempdir().unwrap();
        let previous = base.path().join(other_casing());
        fs::create_dir_all(previous.join("models")).unwrap();
        fs::write(previous.join("models/ggml-small.bin"), b"weights").unwrap();

        let dir = app_dir_in(base.path());

        assert_eq!(dir, base.path().join(APP_DIR));
        assert_eq!(
            fs::read(dir.join("models/ggml-small.bin")).unwrap(),
            b"weights"
        );
    }

    #[test]
    fn keeps_the_current_directory_when_both_exist() {
        let base = tempfile::tempdir().unwrap();
        let previous = base.path().join(other_casing());
        let current = base.path().join(APP_DIR);
        fs::create_dir_all(&previous).unwrap();
        fs::create_dir_all(&current).unwrap();
        fs::write(previous.join("previous.txt"), b"old").unwrap();
        fs::write(current.join("current.txt"), b"new").unwrap();

        let dir = app_dir_in(base.path());

        assert_eq!(dir, current);
        assert!(dir.join("current.txt").exists());
        assert!(previous.join("previous.txt").exists());
    }

    #[test]
    fn returns_the_current_path_when_nothing_exists_yet() {
        let base = tempfile::tempdir().unwrap();

        let dir = app_dir_in(base.path());

        assert_eq!(dir, base.path().join(APP_DIR));
        assert!(!dir.exists());
    }

    #[test]
    fn ignores_unrelated_neighbours() {
        let base = tempfile::tempdir().unwrap();
        fs::create_dir_all(base.path().join("some-other-app")).unwrap();

        let dir = app_dir_in(base.path());

        assert_eq!(dir, base.path().join(APP_DIR));
        assert!(!dir.exists());
    }
}
