use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// Paths cross the IPC boundary as UTF-8 strings. A file name that is not valid UTF-8
/// (possible on Linux) would come back altered and point nowhere, so every path handed
/// to the webview is remembered here and mapped back to the exact original on return.
#[derive(Default)]
struct Registry {
    originals: Vec<(String, PathBuf)>,
    outputs: HashSet<PathBuf>,
    inputs: HashSet<PathBuf>,
}

fn registry() -> std::sync::MutexGuard<'static, Registry> {
    static CELL: OnceLock<Mutex<Registry>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(Registry::default()))
        .lock()
        .unwrap_or_else(|p| p.into_inner())
}

const MAX_REMEMBERED: usize = 64;

/// The string form of `path` for the webview. Only paths that would not survive the
/// round trip are stored.
pub fn to_webview(path: &Path) -> String {
    let lossy = path.to_string_lossy().to_string();
    if path.to_str().is_none() {
        let mut reg = registry();
        reg.originals.retain(|(s, _)| *s != lossy);
        reg.originals.push((lossy.clone(), path.to_path_buf()));
        if reg.originals.len() > MAX_REMEMBERED {
            reg.originals.remove(0);
        }
    }
    lossy
}

/// The real path behind a string received from the webview.
pub fn from_webview(text: &str) -> PathBuf {
    registry()
        .originals
        .iter()
        .find(|(s, _)| s == text)
        .map(|(_, p)| p.clone())
        .unwrap_or_else(|| PathBuf::from(text))
}

/// Marks `path` as a file this app wrote, so later edits may overwrite it — and only it.
pub fn mark_output(path: &Path) {
    registry().outputs.insert(path.to_path_buf());
}

pub fn is_output(path: &Path) -> bool {
    registry().outputs.contains(path)
}

/// Marks `path` as a media file the user chose (dialog or drop): the webview may only ask
/// for files it was given this way, never for an arbitrary path or URL.
pub fn allow_input(path: &Path) {
    registry().inputs.insert(path.to_path_buf());
}

pub fn is_allowed_input(path: &Path) -> bool {
    registry().inputs.contains(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn odd_path() -> PathBuf {
        use std::os::unix::ffi::OsStrExt;
        PathBuf::from(std::ffi::OsStr::from_bytes(b"/tmp/caf\xe9 test.mp4"))
    }

    #[test]
    fn a_plain_path_round_trips_unchanged() {
        let path = Path::new("/home/me/clip.mp4");

        assert_eq!(from_webview(&to_webview(path)), path);
    }

    #[cfg(unix)]
    #[test]
    fn a_non_utf8_path_comes_back_exactly() {
        let path = odd_path();

        let shown = to_webview(&path);

        assert!(shown.contains('\u{fffd}'));
        assert_eq!(from_webview(&shown), path);
    }

    #[test]
    fn an_unknown_string_is_taken_literally() {
        assert_eq!(
            from_webview("/somewhere/else.srt"),
            PathBuf::from("/somewhere/else.srt")
        );
    }

    #[test]
    fn only_chosen_files_are_allowed_as_inputs() {
        let chosen = Path::new("/tmp/paths-test-chosen.mp4");
        allow_input(chosen);

        assert!(is_allowed_input(chosen));
        assert!(!is_allowed_input(Path::new("/etc/passwd")));
        assert!(!is_allowed_input(Path::new("http://example.com/x.mp4")));
    }

    #[test]
    fn only_marked_files_count_as_outputs() {
        let ours = Path::new("/tmp/paths-test-ours.srt");
        mark_output(ours);

        assert!(is_output(ours));
        assert!(!is_output(Path::new("/etc/passwd")));
    }
}
