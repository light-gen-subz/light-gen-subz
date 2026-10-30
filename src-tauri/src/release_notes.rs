//! The changelog of a release, read at its tag on GitHub, so that the update banner can say
//! what a new version brings before it is installed.

use std::time::Duration;

use crate::http;

const REPO: &str = "light-gen-subz/light-gen-subz";
/// A changelog is a few tens of KB; anything much larger is not one.
const MAX_BYTES: usize = 512 * 1024;

/// `https://raw.githubusercontent.com/<repo>/v<version>/CHANGELOG[.fr].md`. The version is
/// checked first: it ends up in a URL.
pub fn notes_url(lang: &str, version: &str) -> Result<String, String> {
    let version = version.trim().trim_start_matches('v');
    let numeric = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit());
    let core = version.split(['-', '+']).next().unwrap_or("");
    let parts: Vec<&str> = core.split('.').collect();
    let suffix_ok = version[core.len()..]
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'));
    if parts.len() != 3 || !parts.iter().all(|p| numeric(p)) || !suffix_ok {
        return Err("Invalid version".to_string());
    }
    let file = if lang == "fr" {
        "CHANGELOG.fr.md"
    } else {
        "CHANGELOG.md"
    };
    Ok(format!(
        "https://raw.githubusercontent.com/{REPO}/v{version}/{file}"
    ))
}

#[tauri::command(async)]
pub fn release_notes(lang: String, version: String) -> Result<String, String> {
    let url = notes_url(&lang, &version)?;
    let res = http::client()
        .get(url)
        .timeout(Duration::from_secs(10))
        .send()
        .map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        return Err(format!("GitHub responded {}", res.status()));
    }
    let bytes = res.bytes().map_err(|e| e.to_string())?;
    if bytes.len() > MAX_BYTES {
        return Err("Changelog too large".to_string());
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_url_points_at_the_tag_in_the_right_language() {
        assert_eq!(
            notes_url("en", "1.5.0").unwrap(),
            "https://raw.githubusercontent.com/light-gen-subz/light-gen-subz/v1.5.0/CHANGELOG.md"
        );
        assert!(notes_url("fr", "v1.5.0")
            .unwrap()
            .ends_with("/v1.5.0/CHANGELOG.fr.md"));
        assert!(notes_url("de", "1.5.0").unwrap().ends_with("/CHANGELOG.md"));
        assert!(notes_url("en", "2.0.0-rc.1").is_ok());
    }

    #[test]
    fn anything_that_is_not_a_version_is_refused() {
        for bad in [
            "",
            "1.5",
            "main",
            "../../x",
            "1.5.0/../x",
            "1.5.0?x=1",
            "1.5.0 x",
            "a.b.c",
        ] {
            assert!(notes_url("en", bad).is_err(), "{bad}");
        }
    }
}
